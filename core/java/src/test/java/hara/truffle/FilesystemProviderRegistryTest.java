package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import org.junit.Test;

public class FilesystemProviderRegistryTest {
  @Test
  public void registersFactoriesDeterministicallyAndRejectsIncompatibleDuplicates() {
    FilesystemProviderRegistry registry = new FilesystemProviderRegistry();
    StubFactory first = new StubFactory("memory");
    registry.register(first).register(first);
    assertTrue(registry.contains("memory"));
    assertEquals(Set.of("memory"), registry.kinds());

    IllegalArgumentException duplicate =
        assertThrows(
            IllegalArgumentException.class,
            () -> registry.register(new StubFactory("memory")));
    assertTrue(duplicate.getMessage().contains("FILESYSTEM_PROVIDER_EXISTS memory"));
    assertFalse(registry.contains("native"));
  }

  @Test
  public void validatesBeforeOpeningAndReportsUnknownFactoriesAsFailedStages() {
    ExecutorService io = Executors.newSingleThreadExecutor();
    ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    try {
      IFilesystemFactory.OpenContext context =
          new IFilesystemFactory.OpenContext(io, scheduler, reference -> null);
      FilesystemProviderRegistry registry =
          new FilesystemProviderRegistry().register(new StubFactory("memory"));
      IFilesystem opened =
          registry.open("memory", context, Map.of("name", "fixture"))
              .toCompletableFuture()
              .join();
      assertSame(StubFilesystem.INSTANCE, opened);

      CompletionException missing =
          assertThrows(
              CompletionException.class,
              () -> registry.open("missing", context, Map.of()).toCompletableFuture().join());
      assertTrue(missing.getCause().getMessage().contains("FILESYSTEM_PROVIDER_NOT_FOUND missing"));
    } finally {
      io.shutdownNow();
      scheduler.shutdownNow();
    }
  }

  private static final class StubFactory implements IFilesystemFactory {
    private final String kind;

    StubFactory(String kind) {
      this.kind = kind;
    }

    @Override
    public String kind() {
      return kind;
    }

    @Override
    public void validate(Map<String, ?> configuration) {
      if (!"fixture".equals(configuration.get("name"))) {
        throw new IllegalArgumentException("fixture name is required");
      }
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      return CompletableFuture.completedFuture(StubFilesystem.INSTANCE);
    }
  }

  private enum StubFilesystem implements IFilesystem {
    INSTANCE;

    @Override
    public Descriptor descriptor() {
      return new Descriptor(
          "memory", "memory fixture", false, Capabilities.of(Capability.READ), null, Map.of());
    }

    @Override
    public CompletionStage<Entry> stat(CallContext context, String path) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<byte[]> read(CallContext context, String path) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<Mutation> write(
        CallContext context,
        String path,
        byte[] bytes,
        WriteOptions options,
        MutationContext mutation) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<EntryPage> entriesPage(
        CallContext context, String path, PageRequest request) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<Mutation> mkdir(
        CallContext context, String path, MkdirOptions options, MutationContext mutation) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<Mutation> delete(
        CallContext context, String path, DeleteOptions options, MutationContext mutation) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<Mutation> copy(
        CallContext context,
        String source,
        String target,
        CopyOptions options,
        MutationContext mutation) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<Mutation> move(
        CallContext context,
        String source,
        String target,
        MoveOptions options,
        MutationContext mutation) {
      return CompletableFuture.failedFuture(new UnsupportedOperationException());
    }

    @Override
    public CompletionStage<Void> close(CallContext context) {
      return CompletableFuture.completedFuture(null);
    }
  }
}
