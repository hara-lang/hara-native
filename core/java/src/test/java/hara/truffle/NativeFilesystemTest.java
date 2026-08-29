package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;

public class NativeFilesystemTest {
  @Test
  public void exposesNativeOperationsThroughTheProviderNeutralAsyncContract() throws Exception {
    Path root = Files.createTempDirectory("hara-native-filesystem-");
    ExecutorService io = Executors.newSingleThreadExecutor();
    ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    try {
      FilesystemProviderRegistry registry =
          new FilesystemProviderRegistry().register(new NativeFilesystem.Factory());
      IFilesystem filesystem =
          registry
              .open(
                  "native",
                  new IFilesystemFactory.OpenContext(io, scheduler, reference -> null),
                  Map.of("root", root.toString(), "display", "test mount"))
              .toCompletableFuture()
              .join();

      assertEquals("native", filesystem.descriptor().kind());
      assertEquals("test mount", filesystem.descriptor().display());
      assertFalse(filesystem.descriptor().readOnly());
      assertFalse(filesystem.descriptor().extensions().containsKey("root"));
      assertTrue(filesystem.capabilities().contains(IFilesystem.Capability.APPEND));
      assertFalse(filesystem.capabilities().contains(IFilesystem.Capability.ATOMIC_MOVE));

      IFilesystem.CallContext call = IFilesystem.CallContext.create().withTraceId("case-1");
      assertEquals(
          "/work",
          filesystem
              .mkdir(
                  call,
                  "/work",
                  new IFilesystem.MkdirOptions(true, true),
                  IFilesystem.MutationContext.none())
              .toCompletableFuture()
              .join()
              .path());
      filesystem
          .write(
              call,
              "/work/b.bin",
              new byte[] {2},
              new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
              IFilesystem.MutationContext.none())
          .toCompletableFuture()
          .join();
      filesystem
          .write(
              call,
              "/work/a.bin",
              new byte[] {1},
              new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
              IFilesystem.MutationContext.none())
          .toCompletableFuture()
          .join();
      assertArrayEquals(
          new byte[] {1}, filesystem.read(call, "/work/a.bin").toCompletableFuture().join());

      IFilesystem.EntryPage first =
          filesystem
              .entriesPage(call, "/work", new IFilesystem.PageRequest(null, 1))
              .toCompletableFuture()
              .join();
      assertEquals(
          List.of("/work/a.bin"),
          first.entries().stream().map(IFilesystem.Entry::path).toList());
      assertEquals("1", first.nextToken());
      IFilesystem.EntryPage second =
          filesystem
              .entriesPage(call, "/work", new IFilesystem.PageRequest(first.nextToken(), 1))
              .toCompletableFuture()
              .join();
      assertEquals(
          List.of("/work/b.bin"),
          second.entries().stream().map(IFilesystem.Entry::path).toList());
      assertNull(second.nextToken());
      assertTrue(second.entries().get(0).modifiedAt() > 0);

      CompletionException revision =
          assertThrows(
              CompletionException.class,
              () ->
                  filesystem
                      .write(
                          call,
                          "/work/a.bin",
                          new byte[] {9},
                          new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
                          new IFilesystem.MutationContext("stale", null))
                      .toCompletableFuture()
                      .join());
      assertEquals("unsupported", ((FilesystemException) revision.getCause()).code());

      IFilesystem.CallContext expired = IFilesystem.CallContext.within(Duration.ZERO);
      CompletionException timeout =
          assertThrows(
              CompletionException.class,
              () -> filesystem.read(expired, "/work/a.bin").toCompletableFuture().join());
      assertEquals("timeout", ((FilesystemException) timeout.getCause()).code());

      IFilesystem.CallContext cancelled = IFilesystem.CallContext.create();
      assertTrue(cancelled.cancel());
      CompletionException cancellation =
          assertThrows(
              CompletionException.class,
              () -> filesystem.read(cancelled, "/work/a.bin").toCompletableFuture().join());
      assertEquals("cancelled", ((FilesystemException) cancellation.getCause()).code());

      filesystem.close(IFilesystem.CallContext.create()).toCompletableFuture().join();
      CompletionException closed =
          assertThrows(
              CompletionException.class,
              () -> filesystem.stat(call, "/work/a.bin").toCompletableFuture().join());
      assertEquals("provider-closed", ((FilesystemException) closed.getCause()).code());
    } finally {
      io.shutdownNow();
      scheduler.shutdownNow();
      deleteTree(root);
    }
  }

  @Test
  public void closeSettlesPendingOperationsOnceAndIgnoresLateWorkers() throws Exception {
    Path root = Files.createTempDirectory("hara-native-filesystem-close-");
    ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    ArrayList<Runnable> queued = new ArrayList<>();
    try {
      NativeFilesystem filesystem =
          new NativeFilesystem(new HaraFileProvider(root), "queued", queued::add, scheduler);
      var pending =
          filesystem.read(IFilesystem.CallContext.create(), "/not-run").toCompletableFuture();
      AtomicInteger settlements = new AtomicInteger();
      pending.whenComplete((value, error) -> settlements.incrementAndGet());
      filesystem.close(IFilesystem.CallContext.create()).toCompletableFuture().join();
      CompletionException closed = assertThrows(CompletionException.class, pending::join);
      FilesystemException failure = (FilesystemException) closed.getCause();
      assertEquals("provider-closed", failure.code());
      assertEquals("read", failure.operation());
      assertEquals("/not-run", failure.path());
      for (Runnable operation : queued) operation.run();
      assertEquals(1, settlements.get());
    } finally {
      scheduler.shutdownNow();
      deleteTree(root);
    }
  }

  private static void deleteTree(Path root) throws IOException {
    if (root == null || !Files.exists(root, LinkOption.NOFOLLOW_LINKS)) return;
    try (var paths = Files.walk(root)) {
      for (Path path : paths.sorted(java.util.Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }
}
