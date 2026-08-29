package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;

public class FilesystemRuntimeBindingTest {
  @Test
  public void dispatchUsesTheExactProviderAndProducesTraceableCalls() {
    ControlledFilesystem filesystem = ControlledFilesystem.readWrite();
    FilesystemRuntimeBinding binding = new FilesystemRuntimeBinding(filesystem);

    FilesystemRuntimeBinding.Pending<byte[]> pending = binding.read("/value.bin");
    assertEquals(1, filesystem.readCalls.get());
    assertEquals("/value.bin", filesystem.lastPath);
    assertTrue(filesystem.lastContext.traceId().startsWith("filesystem/fixture/read/"));
    assertEquals(1, binding.pendingCount());

    filesystem.readStage.complete(new byte[] {0, 1, 0, (byte) 255});
    assertArrayEquals(new byte[] {0, 1, 0, (byte) 255}, join(pending.future()));
    assertEquals(0, binding.pendingCount());
    assertSame(filesystem, binding.filesystem());
    assertEquals("fixture", binding.descriptor().kind());
  }

  @Test
  public void descriptorRefreshesSafeMetadataButRejectsAuthorityChanges() {
    ControlledFilesystem filesystem = ControlledFilesystem.readWrite();
    FilesystemRuntimeBinding binding = new FilesystemRuntimeBinding(filesystem);
    filesystem.updateSafeDescriptor("2");
    assertEquals("2", binding.descriptor().revision());
    assertEquals("fixture updated", binding.descriptor().display());

    filesystem.expandAuthority();
    FilesystemException changed =
        assertThrows(FilesystemException.class, binding::descriptor);
    assertEquals("io", changed.code());
    assertEquals("descriptor-authority-changed", changed.providerCode());
    assertEquals(0, filesystem.readCalls.get());

    FilesystemException rejected =
        assertThrows(
            FilesystemException.class,
            () -> join(binding.read("/denied").future()));
    assertEquals("descriptor-authority-changed", rejected.providerCode());
    assertEquals(0, filesystem.readCalls.get());
  }

  @Test
  public void cancellationSettlesOnceAndIgnoresLateProviderSuccess() {
    ControlledFilesystem filesystem = ControlledFilesystem.readWrite();
    FilesystemRuntimeBinding binding = new FilesystemRuntimeBinding(filesystem);
    FilesystemRuntimeBinding.Pending<byte[]> pending = binding.read("/slow.bin");
    IFilesystem.CallContext call = filesystem.lastContext;

    assertTrue(pending.cancel());
    FilesystemException cancelled =
        assertThrows(FilesystemException.class, () -> join(pending.future()));
    assertEquals("cancelled", cancelled.code());
    assertTrue(call.cancelled());
    assertFalse(pending.cancel());

    filesystem.readStage.complete(new byte[] {7});
    FilesystemException stillCancelled =
        assertThrows(FilesystemException.class, () -> join(pending.future()));
    assertEquals("cancelled", stillCancelled.code());
    assertEquals(0, binding.pendingCount());
  }

  @Test
  public void closeLinearizesAfterStartedInvocationAndBeforeItsSettlement() throws Exception {
    BlockingFilesystem filesystem = new BlockingFilesystem();
    FilesystemRuntimeBinding binding = new FilesystemRuntimeBinding(filesystem);
    ExecutorService executor = Executors.newFixedThreadPool(2);
    try {
      Future<FilesystemRuntimeBinding.Pending<byte[]>> started =
          executor.submit(() -> binding.read("/race.bin"));
      assertTrue(filesystem.entered.await(5, TimeUnit.SECONDS));
      Future<?> closing = executor.submit(binding::close);
      assertFalse(closing.isDone());

      filesystem.release.countDown();
      FilesystemRuntimeBinding.Pending<byte[]> pending = started.get(5, TimeUnit.SECONDS);
      closing.get(5, TimeUnit.SECONDS);
      assertEquals(1, filesystem.readCalls.get());
      FilesystemException closed =
          assertThrows(FilesystemException.class, () -> join(pending.future()));
      assertEquals("provider-closed", closed.code());

      filesystem.readStage.complete(new byte[] {1});
      FilesystemException late =
          assertThrows(FilesystemException.class, () -> join(pending.future()));
      assertEquals("provider-closed", late.code());
    } finally {
      filesystem.release.countDown();
      executor.shutdownNow();
    }
  }

  @Test
  public void detachClosesOnlyTheBindingAndRejectsLateSettlementsAndReuse() {
    ControlledFilesystem filesystem = ControlledFilesystem.readWrite();
    FilesystemRuntimeBinding binding = new FilesystemRuntimeBinding(filesystem);
    FilesystemRuntimeBinding.Pending<byte[]> pending = binding.read("/pending.bin");

    binding.close();
    binding.close();
    assertTrue(binding.closed());
    assertEquals(0, binding.pendingCount());
    FilesystemException closed =
        assertThrows(FilesystemException.class, () -> join(pending.future()));
    assertEquals("provider-closed", closed.code());
    assertTrue(filesystem.lastContext.cancelled());

    filesystem.readStage.complete(new byte[] {1});
    FilesystemException late =
        assertThrows(FilesystemException.class, () -> join(pending.future()));
    assertEquals("provider-closed", late.code());

    FilesystemException reused =
        assertThrows(
            FilesystemException.class,
            () -> join(binding.read("/after-close").future()));
    assertEquals("provider-closed", reused.code());
    assertEquals(1, filesystem.readCalls.get());
    assertEquals(0, filesystem.closeCalls.get());
  }

  @Test
  public void compoundOperationsRequireTheirCompleteCapabilityClosure() {
    ControlledFilesystem readOnly =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.READ));
    assertUnsupported(
        new FilesystemRuntimeBinding(readOnly)
            .write(
                "/new",
                new byte[] {1},
                new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                IFilesystem.MutationContext.none())
            .future(),
        "capability-unavailable:write");
    assertEquals(0, readOnly.writeCalls.get());

    ControlledFilesystem appendWithoutWrite =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.APPEND));
    assertUnsupported(
        new FilesystemRuntimeBinding(appendWithoutWrite)
            .write(
                "/append",
                new byte[] {1},
                new IFilesystem.WriteOptions(IFilesystem.WriteMode.APPEND, false),
                IFilesystem.MutationContext.none())
            .future(),
        "capability-unavailable:write");
    assertEquals(0, appendWithoutWrite.writeCalls.get());

    ControlledFilesystem entriesWithoutRead =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.ENTRIES));
    assertUnsupported(
        new FilesystemRuntimeBinding(entriesWithoutRead).walk("/").future(),
        "capability-unavailable:read");
    assertEquals(0, entriesWithoutRead.statCalls.get());
    assertEquals(0, entriesWithoutRead.entriesCalls.get());

    ControlledFilesystem writeWithoutRead =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.WRITE));
    assertUnsupported(
        new FilesystemRuntimeBinding(writeWithoutRead)
            .tempFile("/tmp", "tmp", "")
            .future(),
        "capability-unavailable:read");
    assertEquals(0, writeWithoutRead.statCalls.get());
    assertEquals(0, writeWithoutRead.writeCalls.get());

    ControlledFilesystem mkdirWithoutRead =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.MKDIR));
    assertUnsupported(
        new FilesystemRuntimeBinding(mkdirWithoutRead)
            .tempDirectory("/tmp", "tmp")
            .future(),
        "capability-unavailable:read");
    assertEquals(0, mkdirWithoutRead.statCalls.get());
    assertEquals(0, mkdirWithoutRead.mkdirCalls.get());

    ControlledFilesystem noRevision =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.WRITE));
    assertUnsupported(
        new FilesystemRuntimeBinding(noRevision)
            .write(
                "/checked",
                new byte[] {1},
                new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
                new IFilesystem.MutationContext("revision-1", null))
            .future(),
        "capability-unavailable:revision-check");
    assertEquals(0, noRevision.writeCalls.get());

    ControlledFilesystem copyOnly =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.COPY));
    assertUnsupported(
        new FilesystemRuntimeBinding(copyOnly)
            .copy(
                "/source",
                "/target",
                new IFilesystem.CopyOptions(false, false, true),
                IFilesystem.MutationContext.none())
            .future(),
        "capability-unavailable:preserve-modified");
    assertEquals(0, copyOnly.copyCalls.get());

    ControlledFilesystem moveOnly =
        new ControlledFilesystem(Set.of(IFilesystem.Capability.MOVE));
    assertUnsupported(
        new FilesystemRuntimeBinding(moveOnly)
            .move(
                "/source",
                "/target",
                new IFilesystem.MoveOptions(false, false, true),
                IFilesystem.MutationContext.none())
            .future(),
        "capability-unavailable:atomic-move");
    assertEquals(0, moveOnly.moveCalls.get());
  }

  @Test
  public void unknownProviderFailuresAreNormalizedWithoutMessageLeakage() {
    ControlledFilesystem filesystem = ControlledFilesystem.readWrite();
    FilesystemRuntimeBinding binding = new FilesystemRuntimeBinding(filesystem);
    FilesystemRuntimeBinding.Pending<byte[]> pending = binding.read("/secret");
    filesystem.readStage.completeExceptionally(
        new IllegalStateException("credential=secret-token"));

    FilesystemException failure =
        assertThrows(FilesystemException.class, () -> join(pending.future()));
    assertEquals("io", failure.code());
    assertEquals("filesystem operation failed", failure.getMessage());
    assertEquals("IllegalStateException", failure.providerCode());
    assertFalse(failure.data().toString().contains("secret-token"));
  }

  private static class ControlledFilesystem implements IFilesystem {
    private final AtomicReference<Descriptor> descriptor;
    final CompletableFuture<byte[]> readStage = new CompletableFuture<>();
    final AtomicInteger statCalls = new AtomicInteger();
    final AtomicInteger readCalls = new AtomicInteger();
    final AtomicInteger writeCalls = new AtomicInteger();
    final AtomicInteger entriesCalls = new AtomicInteger();
    final AtomicInteger mkdirCalls = new AtomicInteger();
    final AtomicInteger copyCalls = new AtomicInteger();
    final AtomicInteger moveCalls = new AtomicInteger();
    final AtomicInteger closeCalls = new AtomicInteger();
    volatile CallContext lastContext;
    volatile String lastPath;

    ControlledFilesystem(Set<Capability> capabilities) {
      descriptor = new AtomicReference<>(descriptor("fixture", "fixture", "1", capabilities));
    }

    static ControlledFilesystem readWrite() {
      return new ControlledFilesystem(
          Set.of(
              Capability.READ,
              Capability.WRITE,
              Capability.APPEND,
              Capability.ENTRIES,
              Capability.MKDIR,
              Capability.DELETE,
              Capability.COPY,
              Capability.MOVE,
              Capability.REVISION_CHECK));
    }

    void updateSafeDescriptor(String revision) {
      Descriptor current = descriptor.get();
      descriptor.set(
          new Descriptor(
              current.kind(),
              "fixture updated",
              current.readOnly(),
              current.capabilities(),
              revision,
              Map.of("provider/generation", 2L)));
    }

    void expandAuthority() {
      Descriptor current = descriptor.get();
      java.util.EnumSet<Capability> capabilities =
          java.util.EnumSet.copyOf(current.capabilities().values());
      capabilities.add(Capability.WATCH);
      descriptor.set(
          descriptor(
              current.kind(),
              current.display(),
              current.revision(),
              capabilities));
    }

    private static Descriptor descriptor(
        String kind, String display, String revision, Set<Capability> capabilities) {
      return new Descriptor(
          kind,
          display,
          false,
          new Capabilities(capabilities),
          revision,
          Map.of());
    }

    @Override
    public Descriptor descriptor() {
      return descriptor.get();
    }

    @Override
    public CompletionStage<Entry> stat(CallContext context, String path) {
      statCalls.incrementAndGet();
      lastContext = context;
      lastPath = path;
      return CompletableFuture.completedFuture(
          new Entry(
              path,
              HaraLogicalPath.fileName(path),
              EntryType.FILE,
              0L,
              null,
              path,
              "1",
              null,
              Map.of()));
    }

    @Override
    public CompletionStage<byte[]> read(CallContext context, String path) {
      readCalls.incrementAndGet();
      lastContext = context;
      lastPath = path;
      return readStage;
    }

    @Override
    public CompletionStage<Mutation> write(
        CallContext context,
        String path,
        byte[] bytes,
        WriteOptions options,
        MutationContext mutation) {
      writeCalls.incrementAndGet();
      return CompletableFuture.completedFuture(Mutation.path(path));
    }

    @Override
    public CompletionStage<EntryPage> entriesPage(
        CallContext context, String path, PageRequest request) {
      entriesCalls.incrementAndGet();
      return CompletableFuture.completedFuture(new EntryPage(List.of(), null));
    }

    @Override
    public CompletionStage<Mutation> mkdir(
        CallContext context,
        String path,
        MkdirOptions options,
        MutationContext mutation) {
      mkdirCalls.incrementAndGet();
      return CompletableFuture.completedFuture(Mutation.path(path));
    }

    @Override
    public CompletionStage<Mutation> delete(
        CallContext context,
        String path,
        DeleteOptions options,
        MutationContext mutation) {
      return CompletableFuture.completedFuture(Mutation.path(path));
    }

    @Override
    public CompletionStage<Mutation> copy(
        CallContext context,
        String source,
        String target,
        CopyOptions options,
        MutationContext mutation) {
      copyCalls.incrementAndGet();
      return CompletableFuture.completedFuture(Mutation.path(target));
    }

    @Override
    public CompletionStage<Mutation> move(
        CallContext context,
        String source,
        String target,
        MoveOptions options,
        MutationContext mutation) {
      moveCalls.incrementAndGet();
      return CompletableFuture.completedFuture(Mutation.path(target));
    }

    @Override
    public CompletionStage<Void> close(CallContext context) {
      closeCalls.incrementAndGet();
      return CompletableFuture.completedFuture(null);
    }
  }

  private static final class BlockingFilesystem extends ControlledFilesystem {
    final CountDownLatch entered = new CountDownLatch(1);
    final CountDownLatch release = new CountDownLatch(1);

    BlockingFilesystem() {
      super(Set.of(IFilesystem.Capability.READ));
    }

    @Override
    public CompletionStage<byte[]> read(CallContext context, String path) {
      readCalls.incrementAndGet();
      lastContext = context;
      lastPath = path;
      entered.countDown();
      try {
        if (!release.await(5, TimeUnit.SECONDS)) {
          return CompletableFuture.failedFuture(
              new IllegalStateException("fixture read was not released"));
        }
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        return CompletableFuture.failedFuture(error);
      }
      return readStage;
    }
  }

  private static void assertUnsupported(
      CompletionStage<?> stage, String providerCode) {
    FilesystemException failure =
        assertThrows(FilesystemException.class, () -> join(stage));
    assertEquals("unsupported", failure.code());
    assertEquals(providerCode, failure.providerCode());
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
