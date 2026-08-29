package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import java.util.Map;
import java.util.concurrent.CompletionException;
import org.junit.Test;

public class GoogleDriveFilesystemSessionKernelTest {
  @Test
  public void driveMountFlowsThroughPublicFileDispatch() throws Exception {
    try (GoogleDriveFilesystemFixture fixture = new GoogleDriveFilesystemFixture();
        SessionKernel kernel = kernel(fixture)) {
      kernel.registerFilesystemProvider(new GoogleDriveFilesystem.Factory());
      SessionModel.SessionMountId mount =
          join(
              kernel.createFilesystem(
                  "google-drive",
                  Map.of(
                      "credential-ref", "drive:test",
                      "root-id", fixture.rootId,
                      "display", "Drive fixture",
                      "workspace-documents", "unsupported")));
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("DRIVE-READ"));

      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());
      assertTrue(binding.filesystem() instanceof GoogleDriveFilesystem);
      assertSame(binding.filesystem(), kernel.filesystemRuntime(session.id()).filesystem());
      assertEquals(
          "hello",
          session
              .eval(
                  "(std.foundation.string/decode-utf8"
                      + " (deref (File/read \"/README.md\")))")
              .asString());
      String itemId =
          session
              .eval("(get (:extensions (deref (File/stat \"/README.md\"))) :file/id)")
              .asString();
      assertTrue(itemId.startsWith("file-"));

      SessionKernel.FilesystemInfo info = kernel.filesystemInfo(mount);
      assertEquals("google-drive", info.kind());
      assertEquals("Drive fixture", info.display());
      assertFalse(info.readOnly());
      assertFalse(info.sourceLoadable());
      assertEquals(1, info.attachments());
      assertTrue(info.capabilities().contains(IFilesystem.Capability.READ));
      assertTrue(info.extensions().containsKey("provider/root-scoped?"));
      assertFalse(info.toString().contains("drive:test"));
      assertFalse(info.toString().contains(fixture.rootId));

      kernel.detachFilesystem(session.id());
      assertTrue(binding.closed());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
    }
  }

  private static SessionKernel kernel(GoogleDriveFilesystemFixture fixture) {
    return new SessionKernel(
        true,
        false,
        false,
        null,
        reference -> {
          assertEquals("drive:test", reference);
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
