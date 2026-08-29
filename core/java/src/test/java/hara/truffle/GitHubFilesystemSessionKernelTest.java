package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import java.nio.charset.StandardCharsets;
import java.util.Map;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import org.junit.Test;

public class GitHubFilesystemSessionKernelTest {
  @Test
  public void immutableCommitMountFlowsThroughTrustedKernelOwnership() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture();
        SessionKernel kernel = kernel(fixture)) {
      kernel.registerFilesystemProvider(new GitHubFilesystem.Factory());
      SessionModel.SessionMountId mount =
          join(
              kernel.createFilesystem(
                  "github",
                  Map.of(
                      "credential-ref", "github:test",
                      "repository", "hara-lang/hara",
                      "ref", fixture.client.initialCommit(),
                      "root", "/",
                      "mode", "read-only",
                      "display", "hara-lang/hara@fixture")));
      SessionKernel.Session session =
          kernel.create(SessionModel.SessionId.parse("GITHUB-READ"));

      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());
      assertTrue(binding.filesystem() instanceof GitHubFilesystem);
      assertSame(binding.filesystem(), kernel.filesystemRuntime(session.id()).filesystem());
      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8),
          join(binding.read("/README.md").future()));
      assertEquals(
          "hello",
          session
              .eval(
                  "(std.foundation.string/decode-utf8"
                      + " (deref (File/read \"/README.md\")))")
              .asString());
      String readmeRevision =
          join(binding.stat("/README.md").future()).revision();
      assertEquals(
          readmeRevision,
          session
              .eval(
                  "(get (:extensions (deref (File/stat \"/README.md\")))"
                      + " :file/revision)")
              .asString());

      SessionKernel.FilesystemInfo info = kernel.filesystemInfo(mount);
      assertEquals("github", info.kind());
      assertEquals("hara-lang/hara@fixture", info.display());
      assertTrue(info.readOnly());
      assertFalse(info.sourceLoadable());
      assertEquals(1, info.attachments());
      assertEquals(fixture.client.initialCommit(), info.revision());
      assertEquals("hara-lang/hara", info.extensions().get("provider/repository"));
      assertFalse(info.toString().contains("github:test"));

      kernel.detachFilesystem(session.id());
      assertTrue(binding.closed());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
    }
  }

  @Test
  public void writableBranchMutationRefreshesTheKernelVisibleRevision() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture();
        SessionKernel kernel = kernel(fixture)) {
      kernel.registerFilesystemProvider(new GitHubFilesystem.Factory());
      SessionModel.SessionMountId mount =
          join(
              kernel.createFilesystem(
                  "github",
                  Map.of(
                      "credential-ref", "github:test",
                      "repository", "hara-lang/hara",
                      "ref", "heads/main",
                      "root", "/",
                      "mode", "commit",
                      "display", "hara-lang/hara@main")));
      SessionKernel.Session session =
          kernel.create(SessionModel.SessionId.parse("GITHUB-WRITE"));
      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());
      String initial = kernel.filesystemInfo(mount).revision();

      IFilesystem.Mutation mutation =
          join(
              binding
                  .write(
                      "/kernel.bin",
                      new byte[] {0, 1, 0, (byte) 255},
                      new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                      IFilesystem.MutationContext.none())
                  .future());

      assertNotEquals(initial, mutation.mountRevision());
      assertEquals(fixture.client.head(), mutation.mountRevision());
      assertEquals(mutation.mountRevision(), kernel.filesystemInfo(mount).revision());
      assertArrayEquals(
          new byte[] {0, 1, 0, (byte) 255},
          join(binding.read("/kernel.bin").future()));
      assertEquals(1, fixture.client.commitMessages().size());
      assertTrue(fixture.client.commitMessages().get(0).contains("write /kernel.bin"));

      kernel.detachFilesystem(session.id());
      kernel.closeFilesystem(mount);
    }
  }

  private static SessionKernel kernel(GitHubFilesystemFixture fixture) {
    return new SessionKernel(
        true,
        false,
        false,
        null,
        reference -> {
          assertEquals("github:test", reference);
          return fixture.client;
        });
  }

  private static <T> T join(CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      Throwable cause = error.getCause();
      if (cause instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }
}
