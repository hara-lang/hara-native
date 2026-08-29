package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import java.util.Map;
import java.util.concurrent.CompletionException;
import org.junit.Test;

public class S3FilesystemSessionKernelTest {
  @Test
  public void objectMountFlowsThroughPublicFileDispatch() throws Exception {
    try (S3FilesystemFixture fixture = new S3FilesystemFixture();
        SessionKernel kernel = kernel(fixture)) {
      kernel.registerFilesystemProvider(new S3Filesystem.Factory());
      SessionModel.SessionMountId mount =
          join(
              kernel.createFilesystem(
                  "s3",
                  Map.of(
                      "credential-ref", "s3:test",
                      "bucket", S3FilesystemFixture.BUCKET,
                      "prefix", S3FilesystemFixture.PREFIX,
                      "display", "Object fixture")));
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("S3-READ"));

      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());
      assertTrue(binding.filesystem() instanceof S3Filesystem);
      assertSame(binding.filesystem(), kernel.filesystemRuntime(session.id()).filesystem());
      assertEquals(
          "hello",
          session
              .eval(
                  "(std.foundation.string/decode-utf8"
                      + " (deref (File/read \"/README.md\")))")
              .asString());
      assertEquals(
          "directory",
          session.eval("(name (:type (deref (File/stat \"/data\"))))").asString());

      SessionKernel.FilesystemInfo info = kernel.filesystemInfo(mount);
      assertEquals("s3", info.kind());
      assertEquals("Object fixture", info.display());
      assertFalse(info.readOnly());
      assertFalse(info.sourceLoadable());
      assertEquals(1, info.attachments());
      assertTrue(info.capabilities().contains(IFilesystem.Capability.READ));
      assertTrue(info.extensions().containsKey("provider/virtual-directories?"));
      assertFalse(info.toString().contains("s3:test"));
      assertFalse(info.toString().contains(S3FilesystemFixture.BUCKET));
      assertFalse(info.toString().contains(S3FilesystemFixture.PREFIX));

      kernel.detachFilesystem(session.id());
      assertTrue(binding.closed());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
    }
  }

  private static SessionKernel kernel(S3FilesystemFixture fixture) {
    return new SessionKernel(
        true,
        false,
        false,
        null,
        reference -> {
          assertEquals("s3:test", reference);
          return fixture.client;
        });
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
}
