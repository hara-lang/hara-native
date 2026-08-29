package hara.truffle;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.PublicKey;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Stream;
import org.apache.sshd.common.file.virtualfs.VirtualFileSystemFactory;
import org.apache.sshd.common.keyprovider.KeyPairProvider;
import org.apache.sshd.server.SshServer;
import org.apache.sshd.sftp.server.SftpSubsystemFactory;

/**
 * Isolated in-process SFTP server fixture for {@link MinaSftpClientTest}: a freshly generated,
 * never-persisted host key pair and a dedicated temporary directory root, so tests never depend on
 * (or pollute) any real host, port, or filesystem state.
 */
final class MinaSftpServerFixture implements AutoCloseable {
  static final String USERNAME = "hara-test";
  static final String PASSWORD = "hara-test-secret";

  private final SshServer server;
  private final KeyPair hostKeyPair;
  private final Path root;

  MinaSftpServerFixture() throws Exception {
    this.root = Files.createTempDirectory("hara-sftp-fixture");
    KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA");
    generator.initialize(2048);
    this.hostKeyPair = generator.generateKeyPair();

    SshServer server = SshServer.setUpDefaultServer();
    server.setPort(0);
    server.setKeyPairProvider(KeyPairProvider.wrap(hostKeyPair));
    server.setPasswordAuthenticator(
        (username, password, session) -> USERNAME.equals(username) && PASSWORD.equals(password));
    server.setSubsystemFactories(List.of(new SftpSubsystemFactory()));
    server.setFileSystemFactory(new VirtualFileSystemFactory(root));
    server.start();
    this.server = server;
  }

  String host() {
    return "127.0.0.1";
  }

  int port() {
    return server.getPort();
  }

  PublicKey hostPublicKey() {
    return hostKeyPair.getPublic();
  }

  /** Resolves a path against the server's real filesystem root, for out-of-band setup/assertions. */
  Path resolve(String relative) {
    return root.resolve(relative);
  }

  @Override
  public void close() throws Exception {
    try {
      server.stop();
    } finally {
      deleteRecursive(root);
    }
  }

  private static void deleteRecursive(Path path) throws IOException {
    if (!Files.exists(path)) return;
    try (Stream<Path> stream = Files.walk(path)) {
      List<Path> ordered = stream.sorted(Comparator.reverseOrder()).toList();
      for (Path entry : ordered) {
        Files.delete(entry);
      }
    }
  }
}
