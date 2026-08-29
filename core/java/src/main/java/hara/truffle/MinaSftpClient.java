package hara.truffle;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.SocketAddress;
import java.nio.file.Path;
import java.security.GeneralSecurityException;
import java.security.KeyPair;
import java.security.PublicKey;
import java.time.Duration;
import java.util.ArrayList;
import java.util.EnumSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.atomic.AtomicBoolean;

import org.apache.sshd.client.ClientBuilder;
import org.apache.sshd.client.SshClient;
import org.apache.sshd.client.auth.password.PasswordIdentityProvider;
import org.apache.sshd.client.config.hosts.HostConfigEntryResolver;
import org.apache.sshd.client.config.hosts.KnownHostEntry;
import org.apache.sshd.client.future.ConnectFuture;
import org.apache.sshd.client.keyverifier.ServerKeyVerifier;
import org.apache.sshd.client.session.ClientSession;
import org.apache.sshd.common.config.keys.KeyUtils;
import org.apache.sshd.common.config.keys.PublicKeyEntryResolver;
import org.apache.sshd.common.keyprovider.KeyIdentityProvider;
import org.apache.sshd.core.CoreModuleProperties;
import org.apache.sshd.sftp.client.SftpClient;
import org.apache.sshd.sftp.client.SftpClientFactory;
import org.apache.sshd.sftp.client.extensions.openssh.OpenSSHPosixRenameExtension;
import org.apache.sshd.sftp.common.SftpConstants;
import org.apache.sshd.sftp.common.SftpException;

/**
 * Production JVM transport adapter binding {@link SftpFilesystem.Client} to Apache MINA SSHD
 * {@code sshd-sftp}.
 *
 * <p>This class owns the trusted connection boundary: constructing/starting the underlying {@link
 * SshClient}, installing an explicit host-key verification policy (pinned keys or a trusted
 * known-hosts file - never MINA's permissive default {@code AcceptAllServerKeyVerifier} and never
 * trust-on-first-use), authenticating with an explicitly supplied credential (never an ambient
 * identity such as {@code ~/.ssh/id_rsa}, an SSH agent, or {@code ~/.ssh/config}), and mapping
 * SFTP protocol status/transport/auth failures to typed {@link SftpFilesystem.ClientFailure}
 * values without depending on server message text.
 */
final class MinaSftpClient implements SftpFilesystem.Client {
  /** Host-owned credential explicitly selected for one connection; never read from disk here. */
  sealed interface Credential permits PasswordCredential, PrivateKeyCredential {
    String username();
  }

  record PasswordCredential(String username, String password) implements Credential {
    PasswordCredential {
      username = requireText(username, "SFTP username");
      if (password == null || password.isEmpty()) {
        throw new IllegalArgumentException("SFTP password must not be empty");
      }
    }
  }

  record PrivateKeyCredential(String username, KeyPair keyPair) implements Credential {
    PrivateKeyCredential {
      username = requireText(username, "SFTP username");
      Objects.requireNonNull(keyPair, "SFTP key pair");
      Objects.requireNonNull(keyPair.getPrivate(), "SFTP private key");
      Objects.requireNonNull(keyPair.getPublic(), "SFTP public key");
    }
  }

  /**
   * Explicit, fail-closed host-key trust policy. There is no accept-all or trust-on-first-use
   * variant: unknown or changed keys are always rejected before any filesystem access.
   */
  sealed interface HostKeyPolicy permits PinnedHostKeys, TrustedKnownHostsFile {}

  record PinnedHostKeys(Set<PublicKey> keys) implements HostKeyPolicy {
    PinnedHostKeys {
      if (keys == null || keys.isEmpty()) {
        throw new IllegalArgumentException("pinned SFTP host keys must be non-empty");
      }
      keys = Set.copyOf(keys);
    }
  }

  record TrustedKnownHostsFile(Path file) implements HostKeyPolicy {
    TrustedKnownHostsFile {
      Objects.requireNonNull(file, "trusted known-hosts file");
    }
  }

  record ConnectOptions(
      String host,
      int port,
      Credential credential,
      HostKeyPolicy hostKeyPolicy,
      Duration connectTimeout,
      Duration authTimeout) {
    ConnectOptions {
      host = requireText(host, "SFTP host");
      if (port < 1 || port > 65535) {
        throw new IllegalArgumentException("SFTP port must be between 1 and 65535");
      }
      credential = Objects.requireNonNull(credential, "SFTP credential");
      hostKeyPolicy = Objects.requireNonNull(hostKeyPolicy, "SFTP host key policy");
      connectTimeout = requirePositive(connectTimeout, "SFTP connect timeout");
      authTimeout = requirePositive(authTimeout, "SFTP auth timeout");
    }
  }

  /** Records why a {@link ServerKeyVerifier} rejected a key, without ever parsing SSH messages. */
  private static final class Rejection {
    private volatile String code;

    void set(String value) {
      code = value;
    }

    String get() {
      return code;
    }
  }

  private final SshClient sshClient;
  private final ClientSession session;
  private final SftpClient sftp;
  private final Set<IFilesystem.Capability> capabilities;
  private final AtomicBoolean closed = new AtomicBoolean();

  private MinaSftpClient(
      SshClient sshClient, ClientSession session, SftpClient sftp, Set<IFilesystem.Capability> capabilities) {
    this.sshClient = sshClient;
    this.session = session;
    this.sftp = sftp;
    this.capabilities = capabilities;
  }

  /**
   * Establishes one trusted SFTP transport: starts a dedicated {@link SshClient} with no ambient
   * identities and an explicit host-key verifier, connects and authenticates within the supplied
   * bounded deadlines, and negotiates the {@link SftpClient} plus its proven capabilities.
   */
  static MinaSftpClient connect(ConnectOptions options) throws SftpFilesystem.ClientFailure {
    Objects.requireNonNull(options, "SFTP connect options");
    Rejection rejection = new Rejection();
    SshClient client = ClientBuilder.builder().build();
    client.setServerKeyVerifier(
        verifierFor(options.hostKeyPolicy(), options.host(), options.port(), rejection));
    // Never resolve ~/.ssh/config aliases and never fall back to ambient identities/agents.
    client.setHostConfigEntryResolver(HostConfigEntryResolver.EMPTY);
    client.setKeyIdentityProvider(KeyIdentityProvider.EMPTY_KEYS_PROVIDER);
    client.setPasswordIdentityProvider(PasswordIdentityProvider.EMPTY_PASSWORDS_PROVIDER);
    CoreModuleProperties.IO_CONNECT_TIMEOUT.set(client, options.connectTimeout());
    CoreModuleProperties.AUTH_TIMEOUT.set(client, options.authTimeout());
    client.start();

    ClientSession session = null;
    try {
      long connectStart = System.nanoTime();
      ConnectFuture connectFuture;
      try {
        connectFuture =
            client.connect(options.credential().username(), options.host(), options.port(), null, null);
        connectFuture.verify(options.connectTimeout());
      } catch (IOException error) {
        throw classify(
            error, rejection, connectStart, options.connectTimeout(), "connect-timeout", "connect-failed", true);
      }
      session = connectFuture.getSession();
      CoreModuleProperties.CHANNEL_OPEN_TIMEOUT.set(session, options.authTimeout());
      installIdentity(session, options.credential());

      long authStart = System.nanoTime();
      try {
        session.auth().verify(options.authTimeout());
      } catch (IOException error) {
        throw classify(
            error,
            rejection,
            authStart,
            options.authTimeout(),
            "auth-timeout",
            "authentication-failed",
            false);
      }

      SftpClient sftp = SftpClientFactory.instance().createSftpClient(session);
      return new MinaSftpClient(client, session, sftp, negotiateCapabilities(sftp));
    } catch (SftpFilesystem.ClientFailure failure) {
      closeQuietly(session, client);
      throw failure;
    } catch (IOException | RuntimeException error) {
      closeQuietly(session, client);
      throw new SftpFilesystem.ClientFailure("io", error.getClass().getSimpleName(), true);
    }
  }

  @Override
  public boolean authenticated() {
    // A MinaSftpClient instance only ever exists after connect() authenticated successfully;
    // construction fails closed otherwise, so this is always true for a live instance.
    return true;
  }

  @Override
  public boolean hostKeyVerified() {
    // As above: construction fails closed unless the explicit host-key policy accepted the key.
    return true;
  }

  @Override
  public Set<IFilesystem.Capability> capabilities() {
    return capabilities;
  }

  @Override
  public SftpFilesystem.RemoteEntry lstat(String path) throws Exception {
    requireOpen();
    try {
      return remoteEntry(entryName(path), sftp.lstat(path));
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public byte[] read(String path, long maxBytes) throws Exception {
    requireOpen();
    try (InputStream input = sftp.read(path)) {
      ByteArrayOutputStream buffer = new ByteArrayOutputStream();
      byte[] chunk = new byte[8192];
      long total = 0L;
      int got;
      while ((got = input.read(chunk)) != -1) {
        total += got;
        if (total > maxBytes) {
          throw new SftpFilesystem.ClientFailure("quota-exceeded", "sftp-transfer-limit", false);
        }
        buffer.write(chunk, 0, got);
      }
      return buffer.toByteArray();
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public void write(String path, byte[] bytes, IFilesystem.WriteMode mode, IFilesystem.MutationContext mutation)
      throws Exception {
    requireOpen();
    try (OutputStream output = sftp.write(path, openModes(mode))) {
      output.write(bytes);
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public List<SftpFilesystem.RemoteEntry> entries(String path) throws Exception {
    requireOpen();
    try {
      ArrayList<SftpFilesystem.RemoteEntry> values = new ArrayList<>();
      for (SftpClient.DirEntry entry : sftp.readDir(path)) {
        String name = entry.getFilename();
        if (".".equals(name) || "..".equals(name)) continue;
        values.add(remoteEntry(name, entry.getAttributes()));
      }
      return values;
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public void mkdir(String path, IFilesystem.MutationContext mutation) throws Exception {
    requireOpen();
    try {
      sftp.mkdir(path);
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public void delete(String path, boolean directory, IFilesystem.MutationContext mutation) throws Exception {
    requireOpen();
    try {
      if (directory) {
        sftp.rmdir(path);
      } else {
        sftp.remove(path);
      }
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public void move(String source, String target, boolean replace, boolean atomic, IFilesystem.MutationContext mutation)
      throws Exception {
    requireOpen();
    try {
      if (atomic || replace) {
        // Standard SFTP v3 rename cannot overwrite an existing target; only the
        // posix-rename@openssh.com extension provides a proven atomic, overwriting rename.
        // Its presence is exactly what gates IFilesystem.Capability.ATOMIC_MOVE, so replace also relies on
        // it rather than a non-atomic delete-then-rename fallback.
        OpenSSHPosixRenameExtension rename = sftp.getExtension(OpenSSHPosixRenameExtension.class);
        if (rename == null || !rename.isSupported()) {
          throw new SftpFilesystem.ClientFailure("unsupported", "posix-rename-unavailable", false);
        }
        rename.posixRename(source, target);
      } else {
        sftp.rename(source, target);
      }
    } catch (IOException error) {
      throw mapIoException(error);
    }
  }

  @Override
  public void close() throws Exception {
    if (!closed.compareAndSet(false, true)) return;
    Exception failure = null;
    try {
      sftp.close();
    } catch (Exception error) {
      failure = error;
    }
    try {
      session.close();
    } catch (Exception error) {
      if (failure == null) failure = error;
    }
    try {
      sshClient.stop();
    } catch (Exception error) {
      if (failure == null) failure = error;
    }
    if (failure != null) throw failure;
  }

  private void requireOpen() throws SftpFilesystem.ClientFailure {
    if (closed.get()) {
      throw new SftpFilesystem.ClientFailure("provider-closed", "client-closed", false);
    }
  }

  private SftpFilesystem.RemoteEntry remoteEntry(String name, SftpClient.Attributes attrs) {
    return new SftpFilesystem.RemoteEntry(
        name,
        entryType(attrs),
        size(attrs),
        modifiedAt(attrs),
        null,
        revision(attrs),
        new IFilesystem.Capabilities(capabilities),
        Map.of("provider/protocol", "sftp"));
  }

  private static Set<SftpClient.OpenMode> openModes(IFilesystem.WriteMode mode) {
    return switch (mode) {
      case CREATE -> EnumSet.of(SftpClient.OpenMode.Write, SftpClient.OpenMode.Create, SftpClient.OpenMode.Exclusive);
      case REPLACE -> EnumSet.of(SftpClient.OpenMode.Write, SftpClient.OpenMode.Create, SftpClient.OpenMode.Truncate);
      case APPEND -> EnumSet.of(SftpClient.OpenMode.Write, SftpClient.OpenMode.Create, SftpClient.OpenMode.Append);
    };
  }

  private static IFilesystem.EntryType entryType(SftpClient.Attributes attrs) {
    if (attrs.isDirectory()) return IFilesystem.EntryType.DIRECTORY;
    if (attrs.isSymbolicLink()) return IFilesystem.EntryType.SYMLINK;
    if (attrs.isRegularFile()) return IFilesystem.EntryType.FILE;
    return IFilesystem.EntryType.OTHER;
  }

  private static Long size(SftpClient.Attributes attrs) {
    return attrs.getFlags().contains(SftpClient.Attribute.Size) ? attrs.getSize() : null;
  }

  private static Long modifiedAt(SftpClient.Attributes attrs) {
    if (!attrs.getFlags().contains(SftpClient.Attribute.ModifyTime)) return null;
    var time = attrs.getModifyTime();
    return time == null ? null : time.toMillis();
  }

  private static String revision(SftpClient.Attributes attrs) {
    // A weak, non-authoritative observability token; SFTP has no native compare-and-swap
    // revision, so REVISION_CHECK is never advertised and this value is never enforced.
    Long fileSize = size(attrs);
    Long modified = modifiedAt(attrs);
    return "sftp:" + (fileSize == null ? "-" : fileSize) + ":" + (modified == null ? "-" : modified);
  }

  private static String entryName(String path) {
    String normalized = path.length() > 1 && path.endsWith("/") ? path.substring(0, path.length() - 1) : path;
    int separator = normalized.lastIndexOf('/');
    String name = separator < 0 ? normalized : normalized.substring(separator + 1);
    return name.isEmpty() ? "root" : name;
  }

  private static void installIdentity(ClientSession session, Credential credential) {
    if (credential instanceof PasswordCredential password) {
      session.addPasswordIdentity(password.password());
    } else if (credential instanceof PrivateKeyCredential key) {
      session.addPublicKeyIdentity(key.keyPair());
    } else {
      throw new IllegalArgumentException("unsupported SFTP credential type");
    }
  }

  private static Set<IFilesystem.Capability> negotiateCapabilities(SftpClient sftp) {
    // READ/WRITE/ENTRIES/MKDIR/DELETE/APPEND/MOVE (non-replacing rename) are core SFTP v3
    // operations proven available on every negotiated session. ATOMIC_MOVE - and therefore a
    // replacing move() - additionally requires the posix-rename@openssh.com extension, since
    // standard SFTP v3 rename can never overwrite an existing target.
    EnumSet<IFilesystem.Capability> values =
        EnumSet.of(
            IFilesystem.Capability.READ,
            IFilesystem.Capability.WRITE,
            IFilesystem.Capability.ENTRIES,
            IFilesystem.Capability.MKDIR,
            IFilesystem.Capability.DELETE,
            IFilesystem.Capability.APPEND,
            IFilesystem.Capability.MOVE);
    OpenSSHPosixRenameExtension rename = sftp.getExtension(OpenSSHPosixRenameExtension.class);
    if (rename != null && rename.isSupported()) {
      values.add(IFilesystem.Capability.ATOMIC_MOVE);
    }
    return Set.copyOf(values);
  }

  private static ServerKeyVerifier verifierFor(
      HostKeyPolicy policy, String host, int port, Rejection rejection) {
    if (policy instanceof PinnedHostKeys pinned) {
      return (session, address, serverKey) -> {
        for (PublicKey trusted : pinned.keys()) {
          if (KeyUtils.compareKeys(trusted, serverKey)) return true;
        }
        rejection.set("host-key-rejected");
        return false;
      };
    }
    if (policy instanceof TrustedKnownHostsFile knownHosts) {
      return (session, address, serverKey) ->
          verifyKnownHosts(knownHosts.file(), host, port, session, serverKey, rejection);
    }
    throw new IllegalArgumentException("unsupported SFTP host key policy");
  }

  private static boolean verifyKnownHosts(
      Path file,
      String host,
      int port,
      ClientSession session,
      PublicKey serverKey,
      Rejection rejection) {
    List<KnownHostEntry> entries;
    try {
      entries = KnownHostEntry.readKnownHostEntries(file);
    } catch (IOException error) {
      rejection.set("host-key-store-unavailable");
      return false;
    }
    boolean hostKnown = false;
    for (KnownHostEntry entry : entries) {
      if (!entry.isHostMatch(host, port)) continue;
      String marker = entry.getMarker();
      if (marker != null && !marker.isBlank()) {
        // Certificate-authority and other marked entries are not trusted by this bounded
        // verifier; a "revoked" marker for a matching key fails closed immediately.
        if ("revoked".equalsIgnoreCase(marker) && matchesEntryKey(entry, session, serverKey)) {
          rejection.set("host-key-revoked");
          return false;
        }
        continue;
      }
      hostKnown = true;
      if (matchesEntryKey(entry, session, serverKey)) return true;
    }
    rejection.set(hostKnown ? "host-key-changed" : "host-key-unknown");
    return false;
  }

  private static boolean matchesEntryKey(KnownHostEntry entry, ClientSession session, PublicKey serverKey) {
    try {
      PublicKey known = entry.getKeyEntry().resolvePublicKey(session, Map.of(), PublicKeyEntryResolver.FAILING);
      return known != null && KeyUtils.compareKeys(known, serverKey);
    } catch (IOException | GeneralSecurityException error) {
      return false;
    }
  }

  private static SftpFilesystem.ClientFailure classify(
      IOException error,
      Rejection rejection,
      long startNanos,
      Duration timeout,
      String timeoutCode,
      String genericCode,
      boolean genericRetryable) {
    // Host-key rejection is checked first regardless of which future (connect or auth) surfaces
    // the resulting exception: MINA's key exchange runs asynchronously, so the verifier may run
    // - and reject - after connectFuture.verify() already returned, only surfacing as an
    // exception from the later auth handshake.
    String rejected = rejection.get();
    if (rejected != null) {
      return new SftpFilesystem.ClientFailure(rejected, error.getClass().getSimpleName(), false);
    }
    if (System.nanoTime() - startNanos >= timeout.toNanos()) {
      return new SftpFilesystem.ClientFailure(timeoutCode, error.getClass().getSimpleName(), true);
    }
    return new SftpFilesystem.ClientFailure(genericCode, error.getClass().getSimpleName(), genericRetryable);
  }

  private static SftpFilesystem.ClientFailure mapIoException(IOException error) {
    if (error instanceof SftpException sftpError) {
      return switch (sftpError.getStatus()) {
        case SftpConstants.SSH_FX_NO_SUCH_FILE, SftpConstants.SSH_FX_NO_SUCH_PATH ->
            new SftpFilesystem.ClientFailure("not-found", "SSH_FX_NO_SUCH_FILE", false);
        case SftpConstants.SSH_FX_PERMISSION_DENIED, SftpConstants.SSH_FX_WRITE_PROTECT ->
            new SftpFilesystem.ClientFailure("permission-denied", "SSH_FX_PERMISSION_DENIED", false);
        case SftpConstants.SSH_FX_FILE_ALREADY_EXISTS ->
            new SftpFilesystem.ClientFailure("already-exists", "SSH_FX_FILE_ALREADY_EXISTS", false);
        case SftpConstants.SSH_FX_FILE_IS_A_DIRECTORY ->
            new SftpFilesystem.ClientFailure("is-directory", "SSH_FX_FILE_IS_A_DIRECTORY", false);
        case SftpConstants.SSH_FX_NOT_A_DIRECTORY ->
            new SftpFilesystem.ClientFailure("not-directory", "SSH_FX_NOT_A_DIRECTORY", false);
        case SftpConstants.SSH_FX_DIR_NOT_EMPTY ->
            new SftpFilesystem.ClientFailure("directory-not-empty", "SSH_FX_DIR_NOT_EMPTY", false);
        case SftpConstants.SSH_FX_QUOTA_EXCEEDED, SftpConstants.SSH_FX_NO_SPACE_ON_FILESYSTEM ->
            new SftpFilesystem.ClientFailure("quota-exceeded", "SSH_FX_QUOTA_EXCEEDED", false);
        case SftpConstants.SSH_FX_OP_UNSUPPORTED ->
            new SftpFilesystem.ClientFailure("unsupported", "SSH_FX_OP_UNSUPPORTED", false);
        case SftpConstants.SSH_FX_LOCK_CONFLICT, SftpConstants.SSH_FX_BYTE_RANGE_LOCK_CONFLICT ->
            new SftpFilesystem.ClientFailure("conflict", "SSH_FX_LOCK_CONFLICT", true);
        case SftpConstants.SSH_FX_INVALID_FILENAME,
                SftpConstants.SSH_FX_INVALID_PARAMETER,
                SftpConstants.SSH_FX_INVALID_HANDLE ->
            new SftpFilesystem.ClientFailure("invalid-path", "SSH_FX_INVALID_PARAMETER", false);
        case SftpConstants.SSH_FX_CONNECTION_LOST, SftpConstants.SSH_FX_NO_CONNECTION ->
            new SftpFilesystem.ClientFailure("io", "SSH_FX_CONNECTION_LOST", true);
        default ->
            new SftpFilesystem.ClientFailure("io", "SSH_FX_STATUS_" + sftpError.getStatus(), true);
      };
    }
    return new SftpFilesystem.ClientFailure("io", error.getClass().getSimpleName(), true);
  }

  private static void closeQuietly(ClientSession session, SshClient client) {
    try {
      if (session != null) session.close();
    } catch (IOException ignored) {
      // Best-effort cleanup after a failed connection attempt.
    } finally {
      try {
        client.stop();
      } catch (RuntimeException ignored) {
        // Best-effort cleanup after a failed connection attempt.
      }
    }
  }

  private static String requireText(String value, String label) {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException(label + " is required");
    }
    return value;
  }

  private static Duration requirePositive(Duration value, String label) {
    Objects.requireNonNull(value, label);
    if (value.isZero() || value.isNegative()) {
      throw new IllegalArgumentException(label + " must be positive");
    }
    return value;
  }
}
