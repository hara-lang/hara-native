package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import java.util.Map;
import java.util.concurrent.CompletionException;
import org.junit.Test;

public class SftpFilesystemSessionKernelTest {
  @Test
  public void verifiedSftpMountFlowsThroughPublicFileDispatch() throws Exception {
    try (SftpFilesystemFixture fixture = new SftpFilesystemFixture();
        SessionKernel kernel = kernel(fixture)) {
      kernel.registerFilesystemProvider(new SftpFilesystem.Factory());
      SessionModel.SessionMountId mount =
          join(
              kernel.createFilesystem(
                  "sftp",
                  Map.of(
                      "credential-ref", "sftp:test",
                      "root", "/srv/application",
                      "display", "SFTP fixture",
                      "max-transfer-bytes", 1024 * 1024)));
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("SFTP-READ"));

      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());
      assertTrue(binding.filesystem() instanceof SftpFilesystem);
      assertSame(binding.filesystem(), kernel.filesystemRuntime(session.id()).filesystem());
      assertEquals(
          "hello",
          session
              .eval(
                  "(std.foundation.string/decode-utf8"
                      + " (deref (File/read \"/README.md\")))")
              .asString());
      assertEquals(
          "file",
          session
              .eval("(name (:type (deref (File/stat \"/README.md\"))))")
              .asString());

      SessionKernel.FilesystemInfo info = kernel.filesystemInfo(mount);
      assertEquals("sftp", info.kind());
      assertEquals("SFTP fixture", info.display());
      assertFalse(info.readOnly());
      assertFalse(info.sourceLoadable());
      assertEquals(1, info.attachments());
      assertTrue(info.capabilities().contains(IFilesystem.Capability.READ));
      assertTrue(info.extensions().containsKey("provider/host-key-verified?"));
      assertFalse(info.toString().contains("sftp:test"));
      assertFalse(info.toString().contains("/srv/application"));

      kernel.detachFilesystem(session.id());
      assertTrue(binding.closed());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
    }
  }

  private static SessionKernel kernel(SftpFilesystemFixture fixture) {
    return new SessionKernel(
        true,
        false,
        false,
        null,
        reference -> {
          assertEquals("sftp:test", reference);
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
