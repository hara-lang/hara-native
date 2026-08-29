package hara.truffle;

import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.net.http.HttpTimeoutException;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;

/** GitHub REST implementation of the authenticated Git object boundary. */
final class GitHubRestObjectClient implements GitHubObjectClient {
  @FunctionalInterface
  interface TokenProvider {
    String accessToken();
  }

  private static final URI DEFAULT_ENDPOINT = URI.create("https://api.github.com/");
  private static final String API_VERSION = "2026-03-10";
  private static final int MAX_TREE_DEPTH = 256;
  private static final int MAX_TREE_ENTRIES = 100_000;
  private static final int MAX_BLOB_BYTES = 100 * 1024 * 1024;
  private static final Set<Integer> OK = Set.of(200);
  private static final Set<Integer> CREATED = Set.of(201);

  private final HttpClient http;
  private final URI endpoint;
  private final TokenProvider tokens;
  private final Duration requestTimeout;

  GitHubRestObjectClient(TokenProvider tokens) {
    this(HttpClient.newHttpClient(), DEFAULT_ENDPOINT, tokens, Duration.ofSeconds(30));
  }

  GitHubRestObjectClient(
      HttpClient http,
      URI endpoint,
      TokenProvider tokens,
      Duration requestTimeout) {
    this.http = Objects.requireNonNull(http, "GitHub HTTP client");
    this.endpoint = normalizeEndpoint(endpoint);
    this.tokens = Objects.requireNonNull(tokens, "GitHub token provider");
    this.requestTimeout = Objects.requireNonNull(requestTimeout, "GitHub request timeout");
    if (requestTimeout.isZero() || requestTimeout.isNegative()) {
      throw new IllegalArgumentException("GitHub request timeout must be positive");
    }
  }

  @Override
  public CompletionStage<Revision> resolveRevision(String repository, String reference) {
    String validatedRepository = repository(repository);
    String validatedReference = requireText(reference, "GitHub reference");
    CompletionStage<String> commit;
    if (validatedReference.matches("[0-9a-fA-F]{7,64}")) {
      commit = CompletableFuture.completedFuture(validatedReference.toLowerCase(Locale.ROOT));
    } else {
      commit = referenceSha(validatedRepository, validatedReference);
    }
    return commit.thenCompose(
        sha ->
            json(
                    "GET",
                    repositoryEndpoint(validatedRepository)
                        + "/git/commits/"
                        + encodeSegment(sha),
                    null,
                    OK,
                    false)
                .thenApply(
                    response ->
                        new Revision(
                            GitHubRestJson.string(response, "sha"),
                            GitHubRestJson.string(
                                GitHubRestJson.object(response, "tree"), "sha"))));
  }

  @Override
  public CompletionStage<TreeSnapshot> readTree(String repository, String treeSha) {
    String validatedRepository = repository(repository);
    String validatedSha = sha(treeSha, "GitHub tree SHA");
    return readTreeNode(validatedRepository, validatedSha, "", 0)
        .thenApply(
            entries -> {
              if (entries.size() > MAX_TREE_ENTRIES) {
                throw failure(
                    FailureKind.UNSUPPORTED,
                    "GitHub tree exceeds the provider entry limit",
                    "tree-entry-limit",
                    false,
                    null);
              }
              entries.sort(java.util.Comparator.comparing(TreeEntry::path));
              return new TreeSnapshot(validatedSha, entries, false);
            });
  }

  @Override
  public CompletionStage<byte[]> readBlob(String repository, String blobSha) {
    String validatedRepository = repository(repository);
    String validatedSha = sha(blobSha, "GitHub blob SHA");
    return request(
            "GET",
            repositoryEndpoint(validatedRepository)
                + "/git/blobs/"
                + encodeSegment(validatedSha),
            "application/vnd.github.raw+json",
            null)
        .thenApply(response -> expect(response, OK, false).body().clone());
  }

  @Override
  public CompletionStage<String> createBlob(String repository, byte[] bytes) {
    String validatedRepository = repository(repository);
    byte[] frozen = Objects.requireNonNull(bytes, "GitHub blob bytes").clone();
    if (frozen.length > MAX_BLOB_BYTES) {
      return failed(
          failure(
              FailureKind.UNSUPPORTED,
              "GitHub blobs cannot exceed 100 MiB",
              "blob-size-limit",
              false,
              null));
    }
    byte[] body =
        GitHubRestJson.encode(
            GitHubRestJson.objectMap(
                "content", Base64.getEncoder().encodeToString(frozen),
                "encoding", "base64"));
    return json(
            "POST",
            repositoryEndpoint(validatedRepository) + "/git/blobs",
            body,
            CREATED,
            false)
        .thenApply(response -> GitHubRestJson.string(response, "sha"));
  }

  @Override
  public CompletionStage<String> createTree(
      String repository, String baseTreeSha, List<TreeChange> changes) {
    String validatedRepository = repository(repository);
    String validatedBase = sha(baseTreeSha, "GitHub base tree SHA");
    ArrayList<Object> encodedChanges = new ArrayList<>();
    for (TreeChange change : List.copyOf(changes)) {
      LinkedHashMap<String, Object> encoded = new LinkedHashMap<>();
      encoded.put("path", change.path());
      if (change.sha() == null) {
        encoded.put("sha", null);
      } else {
        encoded.put("mode", change.mode());
        encoded.put("type", change.type());
        encoded.put("sha", change.sha());
      }
      encodedChanges.add(encoded);
    }
    byte[] body =
        GitHubRestJson.encode(
            GitHubRestJson.objectMap(
                "base_tree", validatedBase,
                "tree", encodedChanges));
    return json(
            "POST",
            repositoryEndpoint(validatedRepository) + "/git/trees",
            body,
            CREATED,
            false)
        .thenApply(response -> GitHubRestJson.string(response, "sha"));
  }

  @Override
  public CompletionStage<String> createCommit(
      String repository, String message, String treeSha, String parentCommitSha) {
    String validatedRepository = repository(repository);
    String validatedMessage = requireText(message, "GitHub commit message");
    String validatedTree = sha(treeSha, "GitHub commit tree SHA");
    String validatedParent = sha(parentCommitSha, "GitHub parent commit SHA");
    byte[] body =
        GitHubRestJson.encode(
            GitHubRestJson.objectMap(
                "message", validatedMessage,
                "tree", validatedTree,
                "parents", List.of(validatedParent)));
    return json(
            "POST",
            repositoryEndpoint(validatedRepository) + "/git/commits",
            body,
            CREATED,
            false)
        .thenApply(response -> GitHubRestJson.string(response, "sha"));
  }

  @Override
  public CompletionStage<Void> updateReference(
      String repository,
      String reference,
      String expectedCommitSha,
      String newCommitSha) {
    String validatedRepository = repository(repository);
    String validatedReference = requireRef(reference);
    String expected = sha(expectedCommitSha, "expected GitHub commit SHA");
    String next = sha(newCommitSha, "new GitHub commit SHA");
    return referenceSha(validatedRepository, validatedReference)
        .thenCompose(
            current -> {
              if (!expected.equals(current)) {
                return failed(
                    failure(
                        FailureKind.CONFLICT,
                        "GitHub reference moved before update",
                        "reference-moved",
                        true,
                        null));
              }
              byte[] body =
                  GitHubRestJson.encode(
                      GitHubRestJson.objectMap("sha", next, "force", false));
              return request(
                      "PATCH",
                      repositoryEndpoint(validatedRepository)
                          + "/git/refs/"
                          + encodePath(validatedReference),
                      "application/vnd.github+json",
                      body)
                  .thenApply(
                      response -> {
                        expect(response, OK, true);
                        return null;
                      });
            });
  }

  private CompletionStage<List<TreeEntry>> readTreeNode(
      String repository, String treeSha, String prefix, int depth) {
    if (depth > MAX_TREE_DEPTH) {
      return failed(
          failure(
              FailureKind.UNSUPPORTED,
              "GitHub tree nesting exceeds the provider limit",
              "tree-depth-limit",
              false,
              null));
    }
    return json(
            "GET",
            repositoryEndpoint(repository) + "/git/trees/" + encodeSegment(treeSha),
            null,
            OK,
            false)
        .thenCompose(
            response -> {
              if (GitHubRestJson.bool(response, "truncated")) {
                return failed(
                    failure(
                        FailureKind.UNSUPPORTED,
                        "GitHub returned a truncated non-recursive tree",
                        "tree-truncated",
                        true,
                        null));
              }
              ArrayList<TreeEntry> direct = new ArrayList<>();
              ArrayList<CompletionStage<List<TreeEntry>>> nested = new ArrayList<>();
              for (JsonValue value : GitHubRestJson.array(response, "tree")) {
                JsonValue.Object entry = GitHubRestJson.asObject(value, "GitHub tree entry");
                String relative = GitHubRestJson.string(entry, "path");
                String path = prefix.isEmpty() ? relative : prefix + "/" + relative;
                String type = GitHubRestJson.string(entry, "type");
                String sha = GitHubRestJson.string(entry, "sha");
                direct.add(
                    new TreeEntry(
                        path,
                        GitHubRestJson.string(entry, "mode"),
                        type,
                        sha,
                        GitHubRestJson.optionalLong(entry, "size")));
                if ("tree".equals(type)) {
                  nested.add(readTreeNode(repository, sha, path, depth + 1));
                }
              }
              if (nested.isEmpty()) return CompletableFuture.completedFuture(direct);
              CompletableFuture<?>[] futures =
                  nested.stream()
                      .map(CompletionStage::toCompletableFuture)
                      .toArray(CompletableFuture[]::new);
              return CompletableFuture.allOf(futures)
                  .thenApply(
                      ignored -> {
                        ArrayList<TreeEntry> output = new ArrayList<>(direct);
                        for (CompletionStage<List<TreeEntry>> subtree : nested) {
                          output.addAll(subtree.toCompletableFuture().join());
                        }
                        if (output.size() > MAX_TREE_ENTRIES) {
                          throw failure(
                              FailureKind.UNSUPPORTED,
                              "GitHub tree exceeds the provider entry limit",
                              "tree-entry-limit",
                              false,
                              null);
                        }
                        return output;
                      });
            });
  }

  private CompletionStage<String> referenceSha(String repository, String reference) {
    String validatedReference = requireRef(reference);
    return json(
            "GET",
            repositoryEndpoint(repository)
                + "/git/ref/"
                + encodePath(validatedReference),
            null,
            OK,
            false)
        .thenApply(
            response ->
                sha(
                    GitHubRestJson.string(
                        GitHubRestJson.object(response, "object"), "sha"),
                    "GitHub reference SHA"));
  }

  private CompletionStage<JsonValue.Object> json(
      String method,
      String path,
      byte[] body,
      Set<Integer> success,
      boolean validationIsConflict) {
    return request(method, path, "application/vnd.github+json", body)
        .thenApply(
            response -> {
              expect(response, success, validationIsConflict);
              try {
                return GitHubRestJson.object(response.body());
              } catch (RuntimeException error) {
                throw failure(
                    FailureKind.IO,
                    "GitHub returned malformed JSON",
                    "invalid-json",
                    false,
                    error);
              }
            });
  }

  private CompletionStage<HttpResponse<byte[]>> request(
      String method, String path, String accept, byte[] body) {
    final String token;
    try {
      token = tokens.accessToken();
    } catch (Throwable error) {
      return failed(
          failure(
              FailureKind.AUTHENTICATION,
              "GitHub credential resolution failed",
              "credential-resolution",
              false,
              error));
    }
    if (token == null || token.isBlank()) {
      return failed(
          failure(
              FailureKind.AUTHENTICATION,
              "GitHub credentials are unavailable",
              "credential-unavailable",
              false,
              null));
    }

    HttpRequest.Builder request =
        HttpRequest.newBuilder(resolve(path))
            .timeout(requestTimeout)
            .header("Accept", accept)
            .header("Authorization", "Bearer " + token)
            .header("User-Agent", "hara-filesystem")
            .header("X-GitHub-Api-Version", API_VERSION);
    if (body == null) {
      request.method(method, HttpRequest.BodyPublishers.noBody());
    } else {
      request
          .header("Content-Type", "application/json; charset=utf-8")
          .method(method, HttpRequest.BodyPublishers.ofByteArray(body));
    }
    return http
        .sendAsync(request.build(), HttpResponse.BodyHandlers.ofByteArray())
        .handle(
            (response, error) -> {
              if (error == null) return response;
              throw new CompletionException(mapTransport(error));
            });
  }

  private HttpResponse<byte[]> expect(
      HttpResponse<byte[]> response,
      Set<Integer> success,
      boolean validationIsConflict) {
    if (success.contains(response.statusCode())) return response;
    int status = response.statusCode();
    String providerCode =
        response
            .headers()
            .firstValue("x-github-request-id")
            .map(value -> "http-" + status + ":" + value)
            .orElse("http-" + status);
    FailureKind kind;
    boolean retryable = false;
    if (status == 401) {
      kind = FailureKind.AUTHENTICATION;
    } else if (status == 403 && rateLimited(response)) {
      kind = FailureKind.RATE_LIMITED;
      retryable = true;
    } else if (status == 403) {
      kind = FailureKind.PERMISSION;
    } else if (status == 404) {
      kind = FailureKind.NOT_FOUND;
    } else if (status == 409 || (status == 422 && validationIsConflict)) {
      kind = FailureKind.CONFLICT;
      retryable = true;
    } else if (status == 422) {
      kind = FailureKind.UNSUPPORTED;
    } else if (status == 429) {
      kind = FailureKind.RATE_LIMITED;
      retryable = true;
    } else if (status >= 500) {
      kind = FailureKind.OFFLINE;
      retryable = true;
    } else {
      kind = FailureKind.IO;
    }
    throw failure(
        kind,
        "GitHub REST request failed with HTTP " + status,
        providerCode,
        retryable,
        null);
  }

  private static boolean rateLimited(HttpResponse<?> response) {
    return response
            .headers()
            .firstValue("x-ratelimit-remaining")
            .map("0"::equals)
            .orElse(false)
        || response.headers().firstValue("retry-after").isPresent();
  }

  private static Failure mapTransport(Throwable error) {
    Throwable cause = unwrap(error);
    if (cause instanceof Failure failure) return failure;
    if (cause instanceof HttpTimeoutException) {
      return failure(
          FailureKind.OFFLINE,
          "GitHub REST request timed out",
          "http-timeout",
          true,
          cause);
    }
    if (cause instanceof IOException) {
      return failure(
          FailureKind.OFFLINE,
          "GitHub REST endpoint is unavailable",
          "http-unavailable",
          true,
          cause);
    }
    return failure(
        FailureKind.IO,
        "GitHub REST request failed",
        cause.getClass().getSimpleName(),
        false,
        cause);
  }

  private URI resolve(String path) {
    if (path == null || !path.startsWith("/") || path.startsWith("//")) {
      throw new IllegalArgumentException("GitHub API path must be absolute");
    }
    return endpoint.resolve(path.substring(1));
  }

  private static URI normalizeEndpoint(URI value) {
    URI endpoint = Objects.requireNonNull(value, "GitHub API endpoint");
    if (!endpoint.isAbsolute() || endpoint.getHost() == null
        || !("https".equals(endpoint.getScheme()) || "http".equals(endpoint.getScheme()))) {
      throw new IllegalArgumentException("GitHub API endpoint must be an absolute HTTP URI");
    }
    String text = endpoint.toString();
    return URI.create(text.endsWith("/") ? text : text + "/");
  }

  private static String repositoryEndpoint(String repository) {
    String[] parts = repository.split("/", -1);
    return "/repos/" + encodeSegment(parts[0]) + "/" + encodeSegment(parts[1]);
  }

  private static String repository(String value) {
    String repository = requireText(value, "GitHub repository");
    if (!repository.matches("[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")) {
      throw new IllegalArgumentException("GitHub repository must be owner/name");
    }
    return repository;
  }

  private static String requireRef(String value) {
    String reference = requireText(value, "GitHub reference");
    if (!(reference.startsWith("heads/") || reference.startsWith("tags/"))) {
      throw new IllegalArgumentException("GitHub reference must be heads/* or tags/*");
    }
    for (String segment : reference.split("/", -1)) {
      if (segment.isEmpty() || ".".equals(segment) || "..".equals(segment)) {
        throw new IllegalArgumentException("GitHub reference is malformed");
      }
    }
    return reference;
  }

  private static String sha(String value, String label) {
    String sha = requireText(value, label).toLowerCase(Locale.ROOT);
    if (!sha.matches("[0-9a-f]{7,64}")) throw new IllegalArgumentException(label + " is malformed");
    return sha;
  }

  private static String requireText(String value, String label) {
    if (value == null || value.isBlank()) throw new IllegalArgumentException(label + " is required");
    return value;
  }

  private static String encodePath(String value) {
    String[] segments = value.split("/", -1);
    StringBuilder output = new StringBuilder();
    for (int index = 0; index < segments.length; index++) {
      if (index > 0) output.append('/');
      output.append(encodeSegment(segments[index]));
    }
    return output.toString();
  }

  private static String encodeSegment(String value) {
    StringBuilder output = new StringBuilder();
    for (byte current : value.getBytes(StandardCharsets.UTF_8)) {
      int unsigned = current & 0xff;
      if ((unsigned >= 'a' && unsigned <= 'z')
          || (unsigned >= 'A' && unsigned <= 'Z')
          || (unsigned >= '0' && unsigned <= '9')
          || unsigned == '-'
          || unsigned == '_'
          || unsigned == '.'
          || unsigned == '~') {
        output.append((char) unsigned);
      } else {
        output.append('%');
        output.append(Character.toUpperCase(Character.forDigit((unsigned >>> 4) & 0xf, 16)));
        output.append(Character.toUpperCase(Character.forDigit(unsigned & 0xf, 16)));
      }
    }
    return output.toString();
  }

  private static Throwable unwrap(Throwable error) {
    Throwable current = error;
    while ((current instanceof CompletionException
            || current instanceof java.util.concurrent.ExecutionException)
        && current.getCause() != null) {
      current = current.getCause();
    }
    return current;
  }

  private static Failure failure(
      FailureKind kind,
      String message,
      String providerCode,
      boolean retryable,
      Throwable cause) {
    return new Failure(kind, message, providerCode, retryable, cause);
  }

  private static <T> CompletionStage<T> failed(Throwable error) {
    return CompletableFuture.failedFuture(error);
  }
}
