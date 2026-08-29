package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.List;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;

public class WebdavHttpClientTest {
  @Test
  public void readsListsAndCarriesRevisionPreconditionsOverDav() throws Exception {
    AtomicReference<String> putIfNoneMatch = new AtomicReference<>();
    AtomicReference<String> putIfMatch = new AtomicReference<>();
    try (Fixture fixture = new Fixture()) {
      fixture.server.createContext(
          "/dav/README.md",
          exchange -> {
            switch (exchange.getRequestMethod()) {
              case "GET" -> respond(exchange, 200, "hello".getBytes(StandardCharsets.UTF_8));
              case "PUT" -> {
                putIfNoneMatch.set(exchange.getRequestHeaders().getFirst("If-None-Match"));
                putIfMatch.set(exchange.getRequestHeaders().getFirst("If-Match"));
                exchange.getRequestBody().readAllBytes();
                respond(exchange, 204, new byte[0]);
              }
              default -> respond(exchange, 405, new byte[0]);
            }
          });
      fixture.server.createContext(
          "/dav/",
          exchange -> {
            if (!"PROPFIND".equals(exchange.getRequestMethod())) {
              respond(exchange, 405, new byte[0]);
              return;
            }
            assertEquals("1", exchange.getRequestHeaders().getFirst("Depth"));
            String body =
                "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">"
                    + response("/dav/", true, null, "\"root\"")
                    + response("/dav/README.md", false, "5", "\"r1\"")
                    + "</d:multistatus>";
            respond(exchange, 207, body.getBytes(StandardCharsets.UTF_8));
          });
      fixture.start();

      WebdavHttpClient client = fixture.client();
      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8), client.read(fixture.url("README.md"), 1024));
      List<WebdavFilesystem.RemoteEntry> entries = client.entries(fixture.url(""));
      assertEquals(1, entries.size());
      assertEquals("README.md", entries.get(0).name());
      assertEquals("\"r1\"", entries.get(0).revision());

      client.write(
          fixture.url("README.md"),
          "new".getBytes(StandardCharsets.UTF_8),
          IFilesystem.WriteMode.CREATE,
          IFilesystem.MutationContext.none());
      assertEquals("*", putIfNoneMatch.get());

      client.write(
          fixture.url("README.md"),
          "next".getBytes(StandardCharsets.UTF_8),
          IFilesystem.WriteMode.REPLACE,
          new IFilesystem.MutationContext("\"r1\"", null));
      assertEquals("\"r1\"", putIfMatch.get());
      client.close();
      assertFalse(client.authenticated());
    }
  }

  @Test
  public void rejectsReturnedHrefOutsideMountedAuthorityAndRoot() throws Exception {
    try (Fixture fixture = new Fixture()) {
      fixture.server.createContext(
          "/dav/",
          exchange -> {
            String body =
                "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">"
                    + response("/dav/", true, null, "\"root\"")
                    + response("https://example.invalid/stolen", false, "4", "\"x\"")
                    + "</d:multistatus>";
            respond(exchange, 207, body.getBytes(StandardCharsets.UTF_8));
          });
      fixture.start();
      WebdavFilesystem.ClientFailure failure =
          assertThrows(
              WebdavFilesystem.ClientFailure.class,
              () -> fixture.client().entries(fixture.url("")));
      assertEquals("outside-root", failure.code());
    }
  }

  @Test
  public void requiresRedirectNeverAndMapsDavStatusWithoutLeakingAuthorization() throws Exception {
    try (Fixture fixture = new Fixture()) {
      fixture.server.createContext("/dav/locked", exchange -> respond(exchange, 423, new byte[0]));
      fixture.start();
      WebdavHttpClient client = fixture.client();
      WebdavFilesystem.ClientFailure failure =
          assertThrows(
              WebdavFilesystem.ClientFailure.class,
              () -> client.read(fixture.url("locked"), 1024));
      assertEquals("conflict", failure.code());
      assertTrue(failure.retryable());

      HttpClient following =
          HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NORMAL).build();
      assertThrows(
          IllegalArgumentException.class,
          () ->
              new WebdavHttpClient(
                  following,
                  URI.create(fixture.url("")),
                  "Bearer must-not-leak",
                  Duration.ofSeconds(2)));
    }
  }

  private static String response(String href, boolean directory, String size, String etag) {
    return "<d:response><d:href>"
        + href
        + "</d:href><d:propstat><d:prop><d:resourcetype>"
        + (directory ? "<d:collection/>" : "")
        + "</d:resourcetype>"
        + (size == null ? "" : "<d:getcontentlength>" + size + "</d:getcontentlength>")
        + "<d:getetag>"
        + etag
        + "</d:getetag></d:prop></d:propstat></d:response>";
  }

  private static void respond(HttpExchange exchange, int status, byte[] body) throws IOException {
    exchange.sendResponseHeaders(status, body.length);
    if (body.length != 0) exchange.getResponseBody().write(body);
    exchange.close();
  }

  private static final class Fixture implements AutoCloseable {
    final HttpServer server;
    private boolean started;

    Fixture() throws IOException {
      server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
    }

    void start() {
      server.start();
      started = true;
    }

    String url(String path) {
      return "http://127.0.0.1:" + server.getAddress().getPort() + "/dav/" + path;
    }

    WebdavHttpClient client() {
      return new WebdavHttpClient(
          HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NEVER).build(),
          URI.create(url("")),
          "Bearer fixture-token",
          Duration.ofSeconds(2));
    }

    @Override
    public void close() {
      if (started) server.stop(0);
    }
  }
}
