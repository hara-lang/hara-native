package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyPairGenerator;
import java.security.PublicKey;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletionException;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import org.apache.sshd.client.config.hosts.KnownHostHashValue;
import org.apache.sshd.common.config.keys.AuthorizedKeyEntry;
import org.junit.Test;

/**
 * Exercises {@link MinaSftpClient} against a real, isolated in-process SFTP server: pinned and
 * known-hosts host-key verification (positive and negative), confinement via genuine no-follow
 * {@code lstat}, mutation through the real protocol, and deterministic close.
 */
public class MinaSftpClientTest {
  @Test
  public void connectsOverRealProtocolAndPerformsLstatNoFollowOperations() throws Exception {
    try (MinaSftpServerFixture serverFixture = new MinaSftpServerFixture()) {
      MinaSftpClient client = connect(serverFixture);
      try {
        assertTrue(client.authenticated());
        assertTrue(client.hostKeyVerified());
        Set<IFilesystem.Capability> capabilities = client.capabilities();
        assertTrue(capabilities.contains(IFilesystem.Capability.READ));
        assertTrue(capabilities.contains(IFilesystem.Capability.WRITE));
        assertTrue(capabilities.contains(IFilesystem.Capability.ENTRIES));
        assertTrue(capabilities.contains(IFilesystem.Capability.MKDIR));
        assertTrue(capabilities.contains(IFilesystem.Capability.DELETE));
        assertTrue(capabilities.contains(IFilesystem.Capability.APPEND));
        assertTrue(capabilities.contains(IFilesystem.Capability.MOVE));
        assertTrue(capabilities.contains(IFilesystem.Capability.ATOMIC_MOVE));

        client.mkdir("/dir", IFilesystem.MutationContext.none());
        byte[] payload = "hello real sftp".getBytes(StandardCharsets.UTF_8);
        client.write(
            "/dir/file.txt", payload, IFilesystem.WriteMode.CREATE, IFilesystem.MutationContext.none());

        SftpFilesystem.RemoteEntry stat = client.lstat("/dir/file.txt");
        assertEquals(IFilesystem.EntryType.FILE, stat.type());
        assertEquals(Long.valueOf(payload.length), stat.size());
        assertArrayEquals(payload, client.read("/dir/file.txt", 1024));

        List<SftpFilesystem.RemoteEntry> entries = client.entries("/dir");
        assertEquals(1, entries.size());
        assertEquals("file.txt", entries.get(0).name());

        // A real symlink on the server-side filesystem, pointing outside the mounted directory.
        // A genuine no-follow lstat must report SYMLINK (never the target's type), which is what
        // lets SftpFilesystem's root-confinement guard reject it before any data is read.
        Path outside = serverFixture.resolve("outside.txt");
        Files.writeString(outside, "outside content");
        Files.createSymbolicLink(serverFixture.resolve("dir/escape"), outside);
        SftpFilesystem.RemoteEntry symlinkEntry = client.lstat("/dir/escape");
        assertEquals(IFilesystem.EntryType.SYMLINK, symlinkEntry.type());
        client.delete("/dir/escape", false, IFilesystem.MutationContext.none());

        client.move("/dir/file.txt", "/dir/renamed.txt", true, true, IFilesystem.MutationContext.none());
        assertEquals(IFilesystem.EntryType.FILE, client.lstat("/dir/renamed.txt").type());

        client.delete("/dir/renamed.txt", false, IFilesystem.MutationContext.none());
        client.delete("/dir", true, IFilesystem.MutationContext.none());

        try {
          client.lstat("/dir");
          fail("expected not-found after deleting the directory");
        } catch (SftpFilesystem.ClientFailure failure) {
          assertEquals("not-found", failure.code());
        }
      } finally {
        client.close();
      }

      try {
        client.lstat("/dir");
        fail("expected provider-closed after close()");
      } catch (SftpFilesystem.ClientFailure failure) {
        assertEquals("provider-closed", failure.code());
      }
    }
  }

  @Test
  public void mountsThroughSftpFilesystemFactoryOverRealProtocolSession() throws Exception {
    try (MinaSftpServerFixture serverFixture = new MinaSftpServerFixture();
        FixtureExecutors executors = new FixtureExecutors()) {
      Files.createDirectories(serverFixture.resolve("app"));
      MinaSftpClient client = connect(serverFixture);

      IFilesystemFactory.OpenContext context =
          executors.context(
              reference -> {
                assertEquals("sftp:real", reference);
                return client;
              });
      IFilesystem filesystem =
          join(
              new SftpFilesystem.Factory()
                  .open(
                      context,
                      Map.of(
                          "credential-ref", "sftp:real",
                          "root", "/app",
                          "display", "Real SFTP mount",
                          "max-transfer-bytes", 1024 * 1024)));
      try {
        assertEquals("sftp", filesystem.descriptor().kind());
        assertTrue(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.READ));

        byte[] payload = "hello real sftp".getBytes(StandardCharsets.UTF_8);
        IFilesystem.Mutation write =
            join(
                filesystem.write(
                    IFilesystem.CallContext.create(),
                    "/greeting.txt",
                    payload,
                    new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                    IFilesystem.MutationContext.none()));
        assertEquals("/greeting.txt", write.path());

        assertArrayEquals(
            payload, join(filesystem.read(IFilesystem.CallContext.create(), "/greeting.txt")));

        IFilesystem.EntryPage page =
            join(
                filesystem.entriesPage(
                    IFilesystem.CallContext.create(), "/", IFilesystem.PageRequest.first()));
        assertEquals(1, page.entries().size());
        assertEquals("/greeting.txt", page.entries().get(0).path());

        join(
            filesystem.delete(
                IFilesystem.CallContext.create(),
                "/greeting.txt",
                new IFilesystem.DeleteOptions(false),
                IFilesystem.MutationContext.none()));
      } finally {
        join(filesystem.close(IFilesystem.CallContext.create()));
      }
    }
  }

  @Test
  public void rejectsMismatchedPinnedHostKeyBeforeAnyAuthentication() throws Exception {
    try (MinaSftpServerFixture serverFixture = new MinaSftpServerFixture()) {
      PublicKey wrongKey = generateKeyPairPublicKey();
      MinaSftpClient.ConnectOptions options =
          new MinaSftpClient.ConnectOptions(
              serverFixture.host(),
              serverFixture.port(),
              new MinaSftpClient.PasswordCredential(
                  MinaSftpServerFixture.USERNAME, MinaSftpServerFixture.PASSWORD),
              new MinaSftpClient.PinnedHostKeys(Set.of(wrongKey)),
              Duration.ofSeconds(5),
              Duration.ofSeconds(5));
      try {
        MinaSftpClient.connect(options);
        fail("expected host key rejection");
      } catch (SftpFilesystem.ClientFailure failure) {
        assertEquals("host-key-rejected", failure.code());
        assertFalse(failure.retryable());
      }
    }
  }

  @Test
  public void knownHostsFileTrustsMatchingEntryAndFailsClosedOnUnknownOrChangedKey() throws Exception {
    try (MinaSftpServerFixture serverFixture = new MinaSftpServerFixture()) {
      Path knownHosts = Files.createTempFile("hara-known-hosts", "");
      try {
        Files.writeString(
            knownHosts,
            knownHostsLine(serverFixture.host(), serverFixture.port(), serverFixture.hostPublicKey()));
        MinaSftpClient trusted =
            MinaSftpClient.connect(
                new MinaSftpClient.ConnectOptions(
                    serverFixture.host(),
                    serverFixture.port(),
                    new MinaSftpClient.PasswordCredential(
                        MinaSftpServerFixture.USERNAME, MinaSftpServerFixture.PASSWORD),
                    new MinaSftpClient.TrustedKnownHostsFile(knownHosts),
                    Duration.ofSeconds(5),
                    Duration.ofSeconds(5)));
        trusted.close();

        // Unknown host: the file has no entry at all for this address - never trust on first use.
        Files.writeString(
            knownHosts,
            knownHostsLine("unrelated-host", serverFixture.port(), serverFixture.hostPublicKey()));
        try {
          connectWithKnownHosts(serverFixture, knownHosts);
          fail("expected unknown-host rejection");
        } catch (SftpFilesystem.ClientFailure failure) {
          assertEquals("host-key-unknown", failure.code());
        }

        // Changed key: a matching host entry with a different key must also fail closed.
        Files.writeString(
            knownHosts,
            knownHostsLine(serverFixture.host(), serverFixture.port(), generateKeyPairPublicKey()));
        try {
          connectWithKnownHosts(serverFixture, knownHosts);
          fail("expected changed-key rejection");
        } catch (SftpFilesystem.ClientFailure failure) {
          assertEquals("host-key-changed", failure.code());
        }
      } finally {
        Files.deleteIfExists(knownHosts);
      }
    }
  }

  private static MinaSftpClient connectWithKnownHosts(MinaSftpServerFixture serverFixture, Path knownHosts)
      throws Exception {
    return MinaSftpClient.connect(
        new MinaSftpClient.ConnectOptions(
            serverFixture.host(),
            serverFixture.port(),
            new MinaSftpClient.PasswordCredential(
                MinaSftpServerFixture.USERNAME, MinaSftpServerFixture.PASSWORD),
            new MinaSftpClient.TrustedKnownHostsFile(knownHosts),
            Duration.ofSeconds(5),
            Duration.ofSeconds(5)));
  }

  private static String knownHostsLine(String host, int port, PublicKey key) throws Exception {
    StringBuilder builder = new StringBuilder();
    builder.append(KnownHostHashValue.createHostPattern(host, port)).append(' ');
    AuthorizedKeyEntry.appendPublicKeyEntry(builder, key);
    return builder.toString();
  }

  private static PublicKey generateKeyPairPublicKey() throws Exception {
    KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA");
    generator.initialize(2048);
    return generator.generateKeyPair().getPublic();
  }

  private static MinaSftpClient connect(MinaSftpServerFixture serverFixture) throws Exception {
    return MinaSftpClient.connect(
        new MinaSftpClient.ConnectOptions(
            serverFixture.host(),
            serverFixture.port(),
            new MinaSftpClient.PasswordCredential(
                MinaSftpServerFixture.USERNAME, MinaSftpServerFixture.PASSWORD),
            new MinaSftpClient.PinnedHostKeys(Set.of(serverFixture.hostPublicKey())),
            Duration.ofSeconds(5),
            Duration.ofSeconds(5)));
  }

  private static <T> T join(java.util.concurrent.CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      Throwable cause = error.getCause();
      if (cause instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }

  private static final class FixtureExecutors implements AutoCloseable {
    private final java.util.concurrent.ExecutorService io = Executors.newCachedThreadPool();
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();

    IFilesystemFactory.OpenContext context(IFilesystemFactory.CredentialResolver credentials) {
      return new IFilesystemFactory.OpenContext(io, scheduler, credentials);
    }

    @Override
    public void close() {
      io.shutdownNow();
      scheduler.shutdownNow();
    }
  }
}
