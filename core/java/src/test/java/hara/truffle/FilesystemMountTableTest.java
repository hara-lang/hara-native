package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;

public class FilesystemMountTableTest {
  @Test
  public void providerMountsRetainExactCapabilitiesAndAttachmentCounts() {
    try (Fixture fixture = new Fixture()) {
      TestFactory factory = new TestFactory("remote", "remote");
      fixture.table.register(factory);
      SessionModel.SessionMountId id =
          join(fixture.table.open("remote", Map.of("safe-option", "value")));
      TestFilesystem filesystem = factory.last();

      assertSame(filesystem, fixture.table.filesystem(id));
      assertNull(fixture.table.graalFilesystem(id));
      FilesystemMountTable.Info initial = fixture.table.info(id);
      assertEquals(id, initial.id());
      assertEquals("remote", initial.descriptor().kind());
      assertEquals("safe remote", initial.descriptor().display());
      assertFalse(initial.sourceLoadable());
      assertEquals(0, initial.attachments());
      assertFalse(initial.toString().contains("secret-token"));

      fixture.table.retain(id);
      fixture.table.retain(id);
      assertEquals(2, fixture.table.info(id).attachments());
      assertFailure("FILESYSTEM_ATTACHED", () -> join(fixture.table.close(id)));
      assertEquals(0, filesystem.closeCalls.get());

      fixture.table.release(id);
      fixture.table.release(id);
      join(fixture.table.close(id));
      assertEquals(1, filesystem.closeCalls.get());
      assertEquals(0, fixture.table.size());
      assertFailure("NO_FILESYSTEM", () -> fixture.table.filesystem(id));
    }
  }

  @Test
  public void ownerAttachmentReplacementIsAtomicAndPassesTheExactCapability() {
    try (Fixture fixture = new Fixture()) {
      TestFactory factory = new TestFactory("remote", "remote");
      fixture.table.register(factory);
      SessionModel.SessionMountId first = join(fixture.table.open("remote", Map.of()));
      TestFilesystem firstFilesystem = factory.last();
      SessionModel.SessionMountId second = join(fixture.table.open("remote", Map.of()));
      TestFilesystem secondFilesystem = factory.last();
      FilesystemMountTable.AttachmentKey owner =
          FilesystemMountTable.AttachmentKey.session(SessionModel.SessionId.parse("UI"));
      AtomicInteger installations = new AtomicInteger();
      AtomicReference<FilesystemMountTable.OpenedMount> observed = new AtomicReference<>();

      assertNull(
          fixture.table.attach(
              owner,
              first,
              mount -> {
                installations.incrementAndGet();
                observed.set(mount);
              }));
      assertSame(firstFilesystem, observed.get().filesystem());
      assertNull(observed.get().graalFilesystem());
      assertEquals(first, fixture.table.attachment(owner));
      assertEquals(1, fixture.table.info(first).attachments());
      assertEquals(1, fixture.table.attachmentCount());

      assertEquals(
          first,
          fixture.table.attach(
              owner,
              first,
              ignored -> {
                throw new AssertionError("same-mount attachment was reinstalled");
              }));
      assertEquals(1, installations.get());
      assertEquals(1, fixture.table.info(first).attachments());

      assertFailure(
          "installation rejected",
          () ->
              fixture.table.attach(
                  owner,
                  second,
                  ignored -> {
                    throw new IllegalStateException("installation rejected");
                  }));
      assertEquals(first, fixture.table.attachment(owner));
      assertEquals(1, fixture.table.info(first).attachments());
      assertEquals(0, fixture.table.info(second).attachments());

      assertEquals(
          first,
          fixture.table.attach(
              owner,
              second,
              mount -> {
                installations.incrementAndGet();
                observed.set(mount);
              }));
      assertSame(secondFilesystem, observed.get().filesystem());
      assertEquals(second, fixture.table.attachment(owner));
      assertEquals(0, fixture.table.info(first).attachments());
      assertEquals(1, fixture.table.info(second).attachments());

      assertFailure(
          "removal rejected",
          () ->
              fixture.table.detach(
                  owner,
                  ignored -> {
                    throw new IllegalStateException("removal rejected");
                  }));
      assertEquals(second, fixture.table.attachment(owner));
      assertEquals(1, fixture.table.info(second).attachments());

      assertEquals(
          second,
          fixture.table.detach(owner, mountId -> assertEquals(second, mountId)));
      assertNull(fixture.table.attachment(owner));
      assertEquals(0, fixture.table.info(second).attachments());
      assertEquals(0, fixture.table.attachmentCount());
      join(fixture.table.close(first));
      join(fixture.table.close(second));
    }
  }

  @Test
  public void sessionAndSandboxAttachmentKeysCannotCollide() {
    try (Fixture fixture = new Fixture()) {
      TestFactory factory = new TestFactory("remote", "remote");
      fixture.table.register(factory);
      SessionModel.SessionMountId id = join(fixture.table.open("remote", Map.of()));
      FilesystemMountTable.AttachmentKey session =
          FilesystemMountTable.AttachmentKey.session(SessionModel.SessionId.parse("1"));
      FilesystemMountTable.AttachmentKey sandbox =
          FilesystemMountTable.AttachmentKey.sandbox(new SandboxModel.SandboxId(1));

      fixture.table.attach(session, id, ignored -> {});
      fixture.table.attach(sandbox, id, ignored -> {});
      assertEquals(2, fixture.table.info(id).attachments());
      assertEquals(2, fixture.table.attachmentCount());
      assertEquals(id, fixture.table.releaseAttachment(session));
      assertEquals(1, fixture.table.info(id).attachments());
      assertEquals(id, fixture.table.releaseAttachment(sandbox));
      assertEquals(0, fixture.table.info(id).attachments());
      assertNull(fixture.table.releaseAttachment(session));
      join(fixture.table.close(id));
    }
  }

  @Test
  public void nativeMountAloneReceivesTheLocalGraalSourceAdapter() throws Exception {
    Path root = Files.createTempDirectory("hara-ifilesystem-mount");
    try (Fixture fixture = new Fixture()) {
      SessionModel.SessionMountId id = join(fixture.table.openNative(root));
      assertTrue(fixture.table.filesystem(id) instanceof NativeFilesystem);
      HaraMountedFileSystem graalFilesystem = fixture.table.graalFilesystem(id);
      assertNotNull(graalFilesystem);
      assertEquals(root.toAbsolutePath().normalize(), graalFilesystem.root());

      FilesystemMountTable.Info info = fixture.table.info(id);
      assertEquals("native", info.descriptor().kind());
      assertTrue(info.sourceLoadable());
      assertFalse(info.toString().contains(root.toString()));
      assertEquals(Map.of(), info.descriptor().extensions());
      join(fixture.table.close(id));
    } finally {
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void failedPublicationClosesTheProviderAndPublishesNoMount() {
    try (Fixture fixture = new Fixture()) {
      TestFactory factory = new TestFactory("declared", "different");
      fixture.table.register(factory);
      assertFailure(
          "FILESYSTEM_PROVIDER_KIND_MISMATCH",
          () -> join(fixture.table.open("declared", Map.of())));
      assertEquals(1, factory.last().closeCalls.get());
      assertEquals(0, fixture.table.size());
    }
  }

  @Test
  public void closeAllIsIdempotentAndPreventsLaterAuthorityExpansion() {
    Fixture fixture = new Fixture();
    TestFactory factory = new TestFactory("remote", "remote");
    fixture.table.register(factory);
    SessionModel.SessionMountId id = join(fixture.table.open("remote", Map.of()));
    fixture.table.attach(
        FilesystemMountTable.AttachmentKey.session(SessionModel.SessionId.parse("UI")),
        id,
        ignored -> {});
    join(fixture.table.closeAll());
    join(fixture.table.closeAll());
    assertEquals(1, factory.last().closeCalls.get());
    assertEquals(0, fixture.table.size());
    assertEquals(0, fixture.table.attachmentCount());
    assertFailure(
        "FILESYSTEM_TABLE_CLOSED",
        () -> join(fixture.table.open("remote", Map.of())));
    fixture.close();
  }

  @Test
  public void attachmentUnderflowAndUnknownMountsFailClosed() {
    try (Fixture fixture = new Fixture()) {
      TestFactory factory = new TestFactory("remote", "remote");
      fixture.table.register(factory);
      SessionModel.SessionMountId id = join(fixture.table.open("remote", Map.of()));
      assertFailure("FILESYSTEM_ATTACHMENT_UNDERFLOW", () -> fixture.table.release(id));
      assertFailure(
          "NO_FILESYSTEM",
          () -> fixture.table.info(SessionModel.SessionMountId.of(id.value() + 1)));
      join(fixture.table.close(id));
    }
  }

  private static final class Fixture implements AutoCloseable {
    final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    final FilesystemMountTable table =
        new FilesystemMountTable(
            new IFilesystemFactory.OpenContext(
                Runnable::run,
                scheduler,
                ignored -> null));

    @Override
    public void close() {
      try {
        table.close();
      } finally {
        scheduler.shutdownNow();
      }
    }
  }

  private static final class TestFactory implements IFilesystemFactory {
    private final String kind;
    private final String descriptorKind;
    private final List<TestFilesystem> opened = new ArrayList<>();

    TestFactory(String kind, String descriptorKind) {
      this.kind = kind;
      this.descriptorKind = descriptorKind;
    }

    @Override
    public String kind() {
      return kind;
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      TestFilesystem filesystem = new TestFilesystem(descriptorKind);
      opened.add(filesystem);
      return CompletableFuture.completedFuture(filesystem);
    }

    TestFilesystem last() {
      return opened.get(opened.size() - 1);
    }
  }

  private static final class TestFilesystem implements IFilesystem {
    private final Descriptor descriptor;
    final AtomicInteger closeCalls = new AtomicInteger();

    TestFilesystem(String kind) {
      descriptor =
          new Descriptor(
              kind,
              "safe " + kind,
              false,
              Capabilities.of(
                  Capability.READ,
                  Capability.WRITE,
                  Capability.ENTRIES,
                  Capability.REVISION_CHECK),
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

  private static void assertFailure(String prefix, Runnable operation) {
    RuntimeException error = assertThrows(RuntimeException.class, operation::run);
    assertTrue(
        "expected error prefix " + prefix + " but got " + error,
        error.getMessage() != null && error.getMessage().startsWith(prefix));
  }

  private static <T> T join(CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      Throwable cause = unwrap(error);
      if (cause instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
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
}
