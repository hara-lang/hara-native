package hara.truffle;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.time.Instant;
import java.time.ZonedDateTime;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.Locale;
import java.util.Objects;
import java.util.Set;
import javax.xml.XMLConstants;
import javax.xml.parsers.DocumentBuilderFactory;
import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;

/** Production JDK HTTP transport for the trusted WebDAV filesystem client boundary. */
final class WebdavHttpClient implements WebdavFilesystem.Client {
  private static final int MAX_XML_BYTES = 4 * 1024 * 1024;
  private static final int MAX_REDIRECTS = 3;
  private static final String PROPFIND_BODY =
      "<?xml version=\"1.0\" encoding=\"utf-8\"?>"
          + "<d:propfind xmlns:d=\"DAV:\"><d:prop>"
          + "<d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getetag/>"
          + "</d:prop></d:propfind>";

  private final HttpClient http;
  private final URI mountedRoot;
  private final String authorization;
  private final Duration requestTimeout;
  private final Set<IFilesystem.Capability> capabilities;
  private volatile boolean closed;

  static WebdavHttpClient basic(
      HttpClient http,
      URI mountedRoot,
      String username,
      char[] password,
      Duration requestTimeout) {
    Objects.requireNonNull(username, "WebDAV username");
    Objects.requireNonNull(password, "WebDAV password");
    byte[] credential =
        (username + ":" + new String(password)).getBytes(StandardCharsets.UTF_8);
    try {
      return new WebdavHttpClient(
          http,
          mountedRoot,
          "Basic " + Base64.getEncoder().encodeToString(credential),
          requestTimeout);
    } finally {
      java.util.Arrays.fill(credential, (byte) 0);
      java.util.Arrays.fill(password, '\0');
    }
  }

  WebdavHttpClient(
      HttpClient http, URI mountedRoot, String authorization, Duration requestTimeout) {
    this.http = Objects.requireNonNull(http, "WebDAV HTTP client");
    this.mountedRoot = canonicalRoot(Objects.requireNonNull(mountedRoot, "WebDAV root"));
    if (!"https".equalsIgnoreCase(this.mountedRoot.getScheme())
        && !isLoopbackHttp(this.mountedRoot)) {
      throw new IllegalArgumentException("WebDAV production transport requires HTTPS");
    }
    if (http.followRedirects() != HttpClient.Redirect.NEVER) {
      throw new IllegalArgumentException(
          "WebDAV HTTP client must disable automatic redirects so credentials stay authority-scoped");
    }
    this.authorization = requireAuthorization(authorization);
    this.requestTimeout =
        requestTimeout == null || requestTimeout.isNegative() || requestTimeout.isZero()
            ? Duration.ofSeconds(30)
            : requestTimeout;
    this.capabilities =
        Set.of(
            IFilesystem.Capability.READ,
            IFilesystem.Capability.WRITE,
            IFilesystem.Capability.ENTRIES,
            IFilesystem.Capability.MKDIR,
            IFilesystem.Capability.DELETE,
            IFilesystem.Capability.MOVE,
            IFilesystem.Capability.REVISION_CHECK);
  }

  @Override
  public boolean authenticated() {
    return !closed;
  }

  @Override
  public boolean transportVerified() {
    return !closed;
  }

  @Override
  public Set<IFilesystem.Capability> capabilities() {
    return capabilities;
  }

  @Override
  public WebdavFilesystem.RemoteEntry lstat(String path) throws Exception {
    List<WebdavFilesystem.RemoteEntry> values = propfind(uri(path), "0");
    if (values.isEmpty()) throw failure("not-found", "404", false);
    return values.get(0);
  }

  @Override
  public byte[] read(String path, long maxBytes) throws Exception {
    HttpResponse<byte[]> response = request("GET", uri(path), null, List.of());
    expect(response, Set.of(200, 206));
    byte[] body = response.body();
    if ((long) body.length > maxBytes) {
      throw failure("quota-exceeded", "response-too-large", false);
    }
    return body;
  }

  @Override
  public void write(
      String path,
      byte[] bytes,
      IFilesystem.WriteMode mode,
      IFilesystem.MutationContext mutation)
      throws Exception {
    ArrayList<Header> headers = new ArrayList<>();
    headers.add(new Header("Content-Type", "application/octet-stream"));
    if (mode == IFilesystem.WriteMode.CREATE) headers.add(new Header("If-None-Match", "*"));
    if (mutation.expectedRevision() != null) {
      headers.add(new Header("If-Match", mutation.expectedRevision()));
    }
    HttpResponse<byte[]> response = request("PUT", uri(path), bytes, headers);
    expect(response, Set.of(200, 201, 204));
  }

  @Override
  public List<WebdavFilesystem.RemoteEntry> entries(String path) throws Exception {
    return propfind(uri(path), "1");
  }

  @Override
  public void mkdir(String path, IFilesystem.MutationContext mutation) throws Exception {
    ArrayList<Header> headers = new ArrayList<>();
    if (mutation.expectedTargetRevision() != null) {
      headers.add(new Header("If-Match", mutation.expectedTargetRevision()));
    }
    HttpResponse<byte[]> response = request("MKCOL", collectionUri(uri(path)), new byte[0], headers);
    expect(response, Set.of(201, 204));
  }

  @Override
  public void delete(
      String path, boolean directory, IFilesystem.MutationContext mutation) throws Exception {
    ArrayList<Header> headers = new ArrayList<>();
    if (mutation.expectedRevision() != null) {
      headers.add(new Header("If-Match", mutation.expectedRevision()));
    }
    HttpResponse<byte[]> response = request("DELETE", uri(path), null, headers);
    expect(response, Set.of(200, 202, 204));
  }

  @Override
  public void move(
      String source,
      String target,
      boolean replace,
      boolean atomic,
      IFilesystem.MutationContext mutation)
      throws Exception {
    if (atomic) throw failure("unsupported", "atomic-move", false);
    ArrayList<Header> headers = new ArrayList<>();
    headers.add(new Header("Destination", uri(target).toASCIIString()));
    headers.add(new Header("Overwrite", replace ? "T" : "F"));
    if (mutation.expectedRevision() != null) {
      headers.add(new Header("If-Match", mutation.expectedRevision()));
    }
    HttpResponse<byte[]> response = request("MOVE", uri(source), null, headers);
    expect(response, Set.of(201, 204));
  }

  @Override
  public void close() {
    closed = true;
  }

  private List<WebdavFilesystem.RemoteEntry> propfind(URI target, String depth) throws Exception {
    HttpResponse<byte[]> response =
        request(
            "PROPFIND",
            target,
            PROPFIND_BODY.getBytes(StandardCharsets.UTF_8),
            List.of(
                new Header("Depth", depth),
                new Header("Content-Type", "application/xml; charset=utf-8")));
    expect(response, Set.of(207));
    if (response.body().length > MAX_XML_BYTES) {
      throw failure("quota-exceeded", "multistatus-too-large", false);
    }
    return parseMultistatus(target, response.body(), "1".equals(depth));
  }

  private HttpResponse<byte[]> request(
      String method, URI initial, byte[] body, List<Header> headers) throws Exception {
    ensureOpen();
    URI target = checkedUri(initial);
    for (int redirect = 0; redirect <= MAX_REDIRECTS; redirect++) {
      HttpRequest.Builder builder =
          HttpRequest.newBuilder(target)
              .timeout(requestTimeout)
              .header("Authorization", authorization)
              .header("Accept", "*/*");
      for (Header header : headers) builder.header(header.name(), header.value());
      HttpRequest.BodyPublisher publisher =
          body == null
              ? HttpRequest.BodyPublishers.noBody()
              : HttpRequest.BodyPublishers.ofByteArray(body);
      HttpResponse<byte[]> response;
      try {
        response =
            http.send(
                builder.method(method, publisher).build(), HttpResponse.BodyHandlers.ofByteArray());
      } catch (java.net.http.HttpTimeoutException timeout) {
        throw failure("timeout", "http-timeout", true);
      } catch (InterruptedException interrupted) {
        Thread.currentThread().interrupt();
        throw failure("cancelled", "interrupted", true);
      } catch (IOException offline) {
        throw failure("offline", "transport-io", true);
      }
      int status = response.statusCode();
      if (!isRedirect(status)) return response;
      String location = response.headers().firstValue("location").orElse(null);
      if (location == null) throw failure("io", "redirect-without-location", false);
      URI next = checkedUri(target.resolve(location));
      if (!sameAuthority(target, next)) {
        throw failure("permission-denied", "cross-origin-redirect", false);
      }
      target = next;
    }
    throw failure("io", "too-many-redirects", false);
  }

  private List<WebdavFilesystem.RemoteEntry> parseMultistatus(
      URI requested, byte[] bytes, boolean depthOne) throws Exception {
    DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
    factory.setNamespaceAware(true);
    factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
    factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
    factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
    factory.setFeature("http://apache.org/xml/features/nonvalidating/load-external-dtd", false);
    factory.setXIncludeAware(false);
    factory.setExpandEntityReferences(false);
    factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_DTD, "");
    factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_SCHEMA, "");

    NodeList responses =
        factory
            .newDocumentBuilder()
            .parse(new ByteArrayInputStream(bytes))
            .getElementsByTagNameNS("DAV:", "response");
    ArrayList<WebdavFilesystem.RemoteEntry> result = new ArrayList<>();
    for (int i = 0; i < responses.getLength(); i++) {
      Element response = (Element) responses.item(i);
      String href = text(response, "href");
      if (href == null) continue;
      URI resource = checkedUri(requested.resolve(href));
      boolean self = equivalentResource(requested, resource);
      if (depthOne && self) continue;
      if (!depthOne && !self) continue;
      String name = resourceName(resource);
      result.add(entry(resource, response, name));
    }
    return List.copyOf(result);
  }

  private WebdavFilesystem.RemoteEntry entry(URI resource, Element response, String name) {
    boolean collection =
        response.getElementsByTagNameNS("DAV:", "collection").getLength() > 0;
    Long size = collection ? null : parseLong(text(response, "getcontentlength"));
    Long modified = parseModified(text(response, "getlastmodified"));
    String etag = text(response, "getetag");
    String id = resource.toASCIIString();
    return new WebdavFilesystem.RemoteEntry(
        name,
        collection ? IFilesystem.EntryType.DIRECTORY : IFilesystem.EntryType.FILE,
        size,
        modified,
        id,
        etag,
        new IFilesystem.Capabilities(capabilities),
        java.util.Map.of());
  }

  private void expect(HttpResponse<byte[]> response, Set<Integer> accepted) throws Exception {
    if (accepted.contains(response.statusCode())) return;
    int status = response.statusCode();
    throw switch (status) {
      case 401 -> failure("authentication-failed", "401", false);
      case 403 -> failure("permission-denied", "403", false);
      case 404 -> failure("not-found", "404", false);
      case 409 -> failure("conflict", "409", false);
      case 412 -> failure("conflict", "412", false);
      case 423 -> failure("conflict", "423-locked", true);
      case 429 -> failure("rate-limited", "429", true);
      case 507 -> failure("quota-exceeded", "507", false);
      default ->
          status >= 500
              ? failure("offline", Integer.toString(status), true)
              : failure("io", Integer.toString(status), false);
    };
  }

  private URI uri(String value) throws WebdavFilesystem.ClientFailure {
    return checkedUri(URI.create(value));
  }

  private URI checkedUri(URI candidate) throws WebdavFilesystem.ClientFailure {
    URI absolute =
        candidate.isAbsolute() ? candidate.normalize() : mountedRoot.resolve(candidate).normalize();
    if (!sameAuthority(mountedRoot, absolute) || !underRoot(mountedRoot, absolute)) {
      throw failure("outside-root", "authority-or-root-escape", false);
    }
    String rawPath = absolute.getRawPath();
    String lower = rawPath == null ? "" : rawPath.toLowerCase(Locale.ROOT);
    if (lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c")) {
      throw failure("outside-root", "encoded-path-escape", false);
    }
    return absolute;
  }

  private static URI canonicalRoot(URI root) {
    URI normalized = root.normalize();
    if (normalized.getScheme() == null || normalized.getHost() == null) {
      throw new IllegalArgumentException("WebDAV root must be absolute");
    }
    String path = normalized.getPath() == null ? "/" : normalized.getPath();
    if (!path.endsWith("/")) path += "/";
    try {
      return new URI(
          normalized.getScheme().toLowerCase(Locale.ROOT),
          normalized.getUserInfo(),
          normalized.getHost().toLowerCase(Locale.ROOT),
          normalized.getPort(),
          path,
          null,
          null);
    } catch (java.net.URISyntaxException impossible) {
      throw new IllegalArgumentException("invalid WebDAV root", impossible);
    }
  }

  private static boolean sameAuthority(URI left, URI right) {
    return left.getScheme().equalsIgnoreCase(right.getScheme())
        && Objects.equals(left.getHost(), right.getHost())
        && effectivePort(left) == effectivePort(right);
  }

  private static boolean underRoot(URI root, URI candidate) {
    String rootPath = root.normalize().getPath();
    String path = candidate.normalize().getPath();
    return path != null && rootPath != null && path.startsWith(rootPath);
  }

  private static int effectivePort(URI uri) {
    if (uri.getPort() >= 0) return uri.getPort();
    return "https".equalsIgnoreCase(uri.getScheme()) ? 443 : 80;
  }

  private static boolean isLoopbackHttp(URI uri) {
    String host = uri.getHost();
    return "http".equalsIgnoreCase(uri.getScheme())
        && ("127.0.0.1".equals(host) || "localhost".equalsIgnoreCase(host) || "::1".equals(host));
  }

  private static boolean isRedirect(int status) {
    return status == 301 || status == 302 || status == 307 || status == 308;
  }

  private static boolean equivalentResource(URI left, URI right) {
    String a = left.normalize().getPath();
    String b = right.normalize().getPath();
    if (a == null || b == null) return false;
    return stripTrailingSlash(a).equals(stripTrailingSlash(b));
  }

  private static String stripTrailingSlash(String value) {
    return value.length() > 1 && value.endsWith("/")
        ? value.substring(0, value.length() - 1)
        : value;
  }

  private static URI collectionUri(URI uri) {
    if (uri.getPath().endsWith("/")) return uri;
    return URI.create(uri.toASCIIString() + "/");
  }

  private static String resourceName(URI uri) {
    String path = stripTrailingSlash(uri.getPath());
    int slash = path.lastIndexOf('/');
    return java.net.URLDecoder.decode(path.substring(slash + 1), StandardCharsets.UTF_8);
  }

  private static String text(Element parent, String localName) {
    NodeList nodes = parent.getElementsByTagNameNS("DAV:", localName);
    if (nodes.getLength() == 0) return null;
    Node node = nodes.item(0);
    String value = node.getTextContent();
    return value == null ? null : value.trim();
  }

  private static Long parseLong(String value) {
    if (value == null || value.isBlank()) return null;
    try {
      return Long.parseLong(value);
    } catch (NumberFormatException ignored) {
      return null;
    }
  }

  private static Long parseModified(String value) {
    if (value == null || value.isBlank()) return null;
    try {
      return ZonedDateTime.parse(value, DateTimeFormatter.RFC_1123_DATE_TIME)
          .toInstant()
          .toEpochMilli();
    } catch (RuntimeException ignored) {
      try {
        return Instant.parse(value).toEpochMilli();
      } catch (RuntimeException ignoredAgain) {
        return null;
      }
    }
  }

  private static String requireAuthorization(String value) {
    if (value == null || value.isBlank() || value.indexOf('\r') >= 0 || value.indexOf('\n') >= 0) {
      throw new IllegalArgumentException("WebDAV authorization is required");
    }
    return value;
  }

  private static WebdavFilesystem.ClientFailure failure(
      String code, String providerCode, boolean retryable) {
    return new WebdavFilesystem.ClientFailure(code, providerCode, retryable);
  }

  private void ensureOpen() throws WebdavFilesystem.ClientFailure {
    if (closed) throw failure("provider-closed", "client-closed", false);
  }

  private record Header(String name, String value) {}
}
