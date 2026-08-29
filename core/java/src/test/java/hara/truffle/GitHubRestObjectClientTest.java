package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.assertThrows;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;

public class GitHubRestObjectClientTest {
  private static final String COMMIT = "1111111111111111111111111111111111111111";
  private static final String ROOT_TREE = "2222222222222222222222222222222222222222";
  private static final String README_BLOB = "3333333333333333333333333333333333333333";
  private static final String SOURCE_TREE = "4444444444444444444444444444444444444444";
  private static final String SOURCE_BLOB = "5555555555555555555555555555555555555555";
  private static final String CREATED_BLOB = "6666666666666666666666666666666666666666";
  private static final String CREATED_TREE = "7777777777777777777777777777777777777777";
  private static final String CREATED_COMMIT = "8888888888888888888888888888888888888888";
  private static final String COMPETING_COMMIT = "9999999999999999999999999999999999999999";

  @Test
  public void restClientReadsAndWritesExactGitObjectsWithoutForcingRefs() throws Exception {
    try (Server server = new Server()) {
      GitHubRestObjectClient client = server.client();
      GitHubObjectClient.Revision revision =
          join(client.resolveRevision("hara-lang/hara", "heads/main"));
      assertEquals(COMMIT, revision.commitSha());
      assertEquals(ROOT_TREE, revision.treeSha());

      GitHubObjectClient.TreeSnapshot tree =
          join(client.readTree("hara-lang/hara", ROOT_TREE));
      assertFalse(tree.truncated());
      assertEquals(
          List.of("README.md", "src", "src/main.hal"),
          tree.entries().stream().map(GitHubObjectClient.TreeEntry::path).toList());
      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8),
          join(client.readBlob("hara-lang/hara", README_BLOB)));

      assertEquals(
          CREATED_BLOB,
          join(client.createBlob("hara-lang/hara", new byte[] {0, 1, 0, (byte) 255})));
      assertEquals(
          CREATED_TREE,
          join(
              client.createTree(
                  "hara-lang/hara",
                  ROOT_TREE,
                  List.of(
                      new GitHubObjectClient.TreeChange(
                          "data/new.bin", "100644", "blob", CREATED_BLOB),
                      GitHubObjectClient.TreeChange.delete("README.md")))));
      assertEquals(
          CREATED_COMMIT,
          join(
              client.createCommit(
                  "hara-lang/hara", "hara filesystem: write /data/new.bin", CREATED_TREE, COMMIT)));
      join(
          client.updateReference(
              "hara-lang/hara", "heads/main", COMMIT, CREATED_COMMIT));

      assertEquals(CREATED_COMMIT, server.reference.get());
      assertEquals(1, server.patchCount.get());
      assertTrue(server.authorizationObserved);
      assertTrue(server.apiVersionObserved);

      JsonValue.Object blob = GitHubRestJson.object(server.createdBlobBody);
      assertEquals("base64", GitHubRestJson.string(blob, "encoding"));
      assertArrayEquals(
          new byte[] {0, 1, 0, (byte) 255},
          java.util.Base64.getDecoder().decode(GitHubRestJson.string(blob, "content")));

      JsonValue.Object refUpdate = GitHubRestJson.object(server.referenceBody);
      assertEquals(CREATED_COMMIT, GitHubRestJson.string(refUpdate, "sha"));
      assertFalse(GitHubRestJson.bool(refUpdate, "force"));
    }
  }

  @Test
  public void movedReferenceIsRejectedBeforeThePatchRequest() throws Exception {
    try (Server server = new Server()) {
      server.reference.set(COMPETING_COMMIT);
      GitHubRestObjectClient client = server.client();
      GitHubObjectClient.Failure error =
          assertThrows(
              GitHubObjectClient.Failure.class,
              () ->
                  join(
                      client.updateReference(
                          "hara-lang/hara", "heads/main", COMMIT, CREATED_COMMIT)));
      assertEquals(GitHubObjectClient.FailureKind.CONFLICT, error.kind());
      assertEquals("reference-moved", error.providerCode());
      assertEquals(0, server.patchCount.get());
    }
  }

  @Test
  public void rateLimitsAndCredentialsMapWithoutLeakingTheBearerValue() throws Exception {
    try (Server server = new Server()) {
      server.rateLimited = true;
      GitHubRestObjectClient client = server.client();
      GitHubObjectClient.Failure error =
          assertThrows(
              GitHubObjectClient.Failure.class,
              () -> join(client.resolveRevision("hara-lang/hara", "heads/main")));
      assertEquals(GitHubObjectClient.FailureKind.RATE_LIMITED, error.kind());
      assertTrue(error.retryable());
      assertFalse(error.getMessage().contains(Server.TOKEN));
    }

    GitHubRestObjectClient missing =
        new GitHubRestObjectClient(
            HttpClient.newHttpClient(),
            URI.create("http://127.0.0.1:1/"),
            () -> null,
            Duration.ofSeconds(1));
    GitHubObjectClient.Failure unavailable =
        assertThrows(
            GitHubObjectClient.Failure.class,
            () -> join(missing.resolveRevision("hara-lang/hara", "heads/main")));
    assertEquals(GitHubObjectClient.FailureKind.AUTHENTICATION, unavailable.kind());
    assertEquals("credential-unavailable", unavailable.providerCode());
  }

  private static final class Server implements AutoCloseable {
    static final String TOKEN = "secret-test-token";

    private final HttpServer server;
    private final java.util.concurrent.ExecutorService executor =
        Executors.newCachedThreadPool();
    final AtomicReference<String> reference = new AtomicReference<>(COMMIT);
    final AtomicInteger patchCount = new AtomicInteger();
    final List<String> paths = new CopyOnWriteArrayList<>();
    volatile boolean authorizationObserved;
    volatile boolean apiVersionObserved;
    volatile boolean rateLimited;
    volatile byte[] createdBlobBody;
    volatile byte[] referenceBody;

    Server() throws IOException {
      server =
          HttpServer.create(
              new InetSocketAddress(InetAddress.getLoopbackAddress(), 0), 0);
      server.createContext("/", this::handle);
      server.setExecutor(executor);
      server.start();
    }

    GitHubRestObjectClient client() {
      return new GitHubRestObjectClient(
          HttpClient.newBuilder().executor(executor).build(),
          URI.create(
              "http://127.0.0.1:" + server.getAddress().getPort() + "/"),
          () -> TOKEN,
          Duration.ofSeconds(5));
    }

    private void handle(HttpExchange exchange) throws IOException {
      String path = exchange.getRequestURI().getRawPath();
      String method = exchange.getRequestMethod();
      paths.add(method + " " + path);
      authorizationObserved |=
          ("Bearer " + TOKEN).equals(exchange.getRequestHeaders().getFirst("Authorization"));
      apiVersionObserved |=
          "2026-03-10".equals(exchange.getRequestHeaders().getFirst("X-GitHub-Api-Version"));

      if (rateLimited) {
        exchange.getResponseHeaders().add("X-RateLimit-Remaining", "0");
        respond(exchange, 403, "{\"message\":\"rate limited\"}");
        return;
      }
      String prefix = "/repos/hara-lang/hara";
      if ((prefix + "/git/ref/heads/main").equals(path) && "GET".equals(method)) {
        respond(
            exchange,
            200,
            "{\"ref\":\"refs/heads/main\",\"object\":{" 
                + "\"type\":\"commit\",\"sha\":\""
                + reference.get()
                + "\"}}" );
        return;
      }
      if ((prefix + "/git/commits/" + COMMIT).equals(path) && "GET".equals(method)) {
        respond(
            exchange,
            200,
            "{\"sha\":\"" + COMMIT + "\",\"tree\":{\"sha\":\"" + ROOT_TREE + "\"}}" );
        return;
      }
      if ((prefix + "/git/trees/" + ROOT_TREE).equals(path) && "GET".equals(method)) {
        respond(
            exchange,
            200,
            "{\"sha\":\"" + ROOT_TREE + "\",\"truncated\":false,\"tree\":["
                + "{\"path\":\"README.md\",\"mode\":\"100644\",\"type\":\"blob\",\"sha\":\""
                + README_BLOB
                + "\",\"size\":5},"
                + "{\"path\":\"src\",\"mode\":\"040000\",\"type\":\"tree\",\"sha\":\""
                + SOURCE_TREE
                + "\",\"size\":null}]}" );
        return;
      }
      if ((prefix + "/git/trees/" + SOURCE_TREE).equals(path) && "GET".equals(method)) {
        respond(
            exchange,
            200,
            "{\"sha\":\"" + SOURCE_TREE + "\",\"truncated\":false,\"tree\":["
                + "{\"path\":\"main.hal\",\"mode\":\"100644\",\"type\":\"blob\",\"sha\":\""
                + SOURCE_BLOB
                + "\",\"size\":7}]}" );
        return;
      }
      if ((prefix + "/git/blobs/" + README_BLOB).equals(path) && "GET".equals(method)) {
        respond(exchange, 200, "hello".getBytes(StandardCharsets.UTF_8));
        return;
      }
      if ((prefix + "/git/blobs").equals(path) && "POST".equals(method)) {
        createdBlobBody = exchange.getRequestBody().readAllBytes();
        respond(exchange, 201, "{\"sha\":\"" + CREATED_BLOB + "\"}");
        return;
      }
      if ((prefix + "/git/trees").equals(path) && "POST".equals(method)) {
        exchange.getRequestBody().readAllBytes();
        respond(exchange, 201, "{\"sha\":\"" + CREATED_TREE + "\"}");
        return;
      }
      if ((prefix + "/git/commits").equals(path) && "POST".equals(method)) {
        exchange.getRequestBody().readAllBytes();
        respond(exchange, 201, "{\"sha\":\"" + CREATED_COMMIT + "\"}");
        return;
      }
      if ((prefix + "/git/refs/heads/main").equals(path) && "PATCH".equals(method)) {
        patchCount.incrementAndGet();
        referenceBody = exchange.getRequestBody().readAllBytes();
        JsonValue.Object update = GitHubRestJson.object(referenceBody);
        reference.set(GitHubRestJson.string(update, "sha"));
        respond(
            exchange,
            200,
            "{\"ref\":\"refs/heads/main\",\"object\":{\"type\":\"commit\",\"sha\":\""
                + reference.get()
                + "\"}}" );
        return;
      }
      respond(exchange, 404, "{\"message\":\"not found\"}");
    }

    private static void respond(HttpExchange exchange, int status, String body)
        throws IOException {
      respond(exchange, status, body.getBytes(StandardCharsets.UTF_8));
    }

    private static void respond(HttpExchange exchange, int status, byte[] body)
        throws IOException {
      exchange.getResponseHeaders().add("Content-Type", "application/json");
      exchange.sendResponseHeaders(status, body.length);
      exchange.getResponseBody().write(body);
      exchange.close();
    }

    @Override
    public void close() {
      server.stop(0);
      executor.shutdownNow();
    }
  }

  private static <T> T join(CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      if (error.getCause() instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }
}
