package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;

public class SessionKernelFilesystemMountTest {
  @Test
  public void remoteMountAttachesTheExactProviderWithoutInventingAGraalFilesystem() {
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      TestFactory factory = new TestFactory();
      kernel.registerFilesystemProvider(factory);
      SessionModel.SessionMountId mount =
          join(kernel.createFilesystem("remote", Map.of("display", "safe remote")));
      SessionKernel.Session session =
          kernel.create(SessionModel.SessionId.parse("REMOTE"));

      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());
      assertSame(factory.filesystem, binding.filesystem());
      assertNull(kernel.filesystemInfo(mount).root());
      assertEquals("remote", kernel.filesystemInfo(mount).kind());
      assertEquals("safe remote", kernel.filesystemInfo(mount).display());
      assertFalse(kernel.filesystemInfo(mount).sourceLoadable());
      assertEquals(1, kernel.filesystemInfo(mount).attachments());
      assertFalse(kernel.filesystemInfo(mount).toString().contains("credential"));

      kernel.detachFilesystem(session.id());
      assertTrue(binding.closed());
      assertNull(kernel.filesystemRuntime(session.id()));
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      assertEquals(0, factory.filesystem.closeCalls.get());

      kernel.closeFilesystem(mount);
      assertEquals(1, factory.filesystem.closeCalls.get());
    }
  }

  @Test
  public void sessionCloseReleasesItsBindingAndMountAttachmentExactlyOnce() {
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      TestFactory factory = new TestFactory();
      kernel.registerFilesystemProvider(factory);
      SessionModel.SessionMountId mount =
          join(kernel.createFilesystem("remote", Map.of()));
      SessionKernel.Session session =
          kernel.create(SessionModel.SessionId.parse("CLOSING"));
      kernel.attachFilesystem(session.id(), mount);
      FilesystemRuntimeBinding binding = kernel.filesystemRuntime(session.id());

      kernel.closeSession(session.id());
      assertTrue(binding.closed());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
      assertEquals(1, factory.filesystem.closeCalls.get());
    }
  }

  @Test
  public void nativeMountRetainsItsLocalOnlySourceLoadingAdapter() throws Exception {
    Path root = Files.createTempDirectory("hara-session-filesystem");
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      SessionModel.SessionMountId mount = kernel.createFilesystem(root);
      SessionKernel.Session session =
          kernel.create(SessionModel.SessionId.parse("NATIVE"));
      kernel.attachFilesystem(session.id(), mount);

      SessionKernel.FilesystemInfo info = kernel.filesystemInfo(mount);
      assertEquals("native", info.kind());
      assertEquals(root.toAbsolutePath().normalize(), info.root());
      assertTrue(info.sourceLoadable());
      assertTrue(kernel.filesystemRuntime(session.id()).filesystem() instanceof NativeFilesystem);
      assertEquals(3L, session.eval("(+ 1 2)").asLong());

      kernel.detachFilesystem(session.id());
      kernel.closeFilesystem(mount);
    } finally {
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void inProcessSandboxRejectsRemoteMountsAndRollsBackAttachmentAccounting() {
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      TestFactory factory = new TestFactory();
      kernel.registerFilesystemProvider(factory);
      SessionModel.SessionMountId mount =
          join(kernel.createFilesystem("remote", Map.of()));
      SandboxModel.SandboxSpec spec =
          new SandboxModel.SandboxSpec(
              SandboxModel.SPEC_PROTOCOL,
              "in-process",
              "hara.standard/0-alpha",
              "user",
              List.of(),
              mount,
              HaraPersistentValues.normalize(Map.of()),
              SandboxModel.SandboxLimits.defaults());

      SandboxModel.SandboxException failure =
          assertThrows(
              SandboxModel.SandboxException.class,
              () -> kernel.openSandbox(spec));
      assertEquals(SandboxModel.ErrorCode.UNSUPPORTED, failure.code());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      assertEquals(0, factory.filesystem.closeCalls.get());
      kernel.closeFilesystem(mount);
      assertEquals(1, factory.filesystem.closeCalls.get());
    }
  }

  @Test
  public void providerOpenUsesTheKernelCredentialResolverWithoutExposingItInInfo() {
    Object credential = new Object();
    try (SessionKernel kernel =
        new SessionKernel(
            true,
            false,
            false,
            null,
            reference -> {
              assertEquals("credential-ref", reference);
              return credential;
            })) {
      CredentialFactory factory = new CredentialFactory(credential);
      kernel.registerFilesystemProvider(factory);
      SessionModel.SessionMountId mount =
          join(kernel.createFilesystem("credentialled", Map.of("credential-ref", "credential-ref")));
      SessionKernel.FilesystemInfo info = kernel.filesystemInfo(mount);
      assertEquals("credentialled", info.kind());
      assertFalse(info.toString().contains("credential-ref"));
      assertFalse(info.toString().contains(credential.toString()));
      kernel.closeFilesystem(mount);
    }
  }

  private static final class TestFactory implements IFilesystemFactory {
    final TestFilesystem filesystem = new TestFilesystem("remote", "safe remote");

    @Override
    public String kind() {
      return "remote";
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      return CompletableFuture.completedFuture(filesystem);
    }
  }

  private static final class CredentialFactory implements IFilesystemFactory {
    private final Object expected;

    CredentialFactory(Object expected) {
      this.expected = expected;
    }

    @Override
    public String kind() {
      return "credentialled";
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      Object credential = context.credentials().resolve((String) configuration.get("credential-ref"));
      assertSame(expected, credential);
      return CompletableFuture.completedFuture(
          new TestFilesystem("credentialled", "safe credentialled mount"));
    }
  }

  private static final class TestFilesystem implements IFilesystem {
    private final Descriptor descriptor;
    final AtomicInteger closeCalls = new AtomicInteger();

    TestFilesystem(String kind, String display) {
      descriptor =
          new Descriptor(
              kind,
              display,
              false,
              new Capabilities(
                  Set.of(
                      Capability.READ,
                      Capability.WRITE,
                      Capability.ENTRIES,
                      Capability.REVISION_CHECK)),
              "revision-1",
              Map.of("provider/repository", "owner/repository"));
    }

    @Override
    public Descriptor descriptor() {
      return descriptor;
    }

    @Override
    public CompletionStage<Entry> stat(CallContext context, String path) {
      return unsupported();
    }

    @Override
    public CompletionStage<byte[]> read(CallContext context, String path) {
      return unsupported();
    }

    @Override
    public CompletionStage<Mutation> write(
        CallContext context,
        String path,
        byte[] bytes,
        WriteOptions options,
        MutationContext mutation) {
      return unsupported();
    }

    @Override
    public CompletionStage<EntryPage> entriesPage(
        CallContext context, String path, PageRequest request) {
      return unsupported();
    }

    @Override
    public CompletionStage<Mutation> mkdir(
        CallContext context,
        String path,
        MkdirOptions options,
        MutationContext mutation) {
      return unsupported();
    }

    @Override
    public CompletionStage<Mutation> delete(
        CallContext context,
        String path,
        DeleteOptions options,
        MutationContext mutation) {
      return unsupported();
    }

    @Override
    public CompletionStage<Mutation> copy(
        CallContext context,
        String source,
        String target,
        CopyOptions options,
        MutationContext mutation) {
      return unsupported();
    }

    @Override
    public CompletionStage<Mutation> move(
        CallContext context,
        String source,
        String target,
        MoveOptions options,
        MutationContext mutation) {
      return unsupported();
    }

    @Override
    public CompletionStage<Void> close(CallContext context) {
      closeCalls.incrementAndGet();
      return CompletableFuture.completedFuture(null);
    }

    private static <T> CompletionStage<T> unsupported() {
      return CompletableFuture.failedFuture(new UnsupportedOperationException("fixture"));
    }
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
