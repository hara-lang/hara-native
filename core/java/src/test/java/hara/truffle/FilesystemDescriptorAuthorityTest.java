package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;

public class FilesystemDescriptorAuthorityTest {
  @Test
  public void mountInfoAndAttachmentRefreshSafeMetadataOnly() {
    ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    MutableFilesystem filesystem = new MutableFilesystem();
    FilesystemProviderRegistry providers =
        new FilesystemProviderRegistry().register(new Factory(filesystem));
    try (FilesystemMountTable table =
        new FilesystemMountTable(
            providers,
            new IFilesystemFactory.OpenContext(
                Runnable::run, scheduler, ignored -> null))) {
      SessionModel.SessionMountId id = join(table.open("mutable", Map.of()));
      filesystem.refresh("revision-2");
      assertEquals("revision-2", table.info(id).descriptor().revision());
      assertEquals("mutable updated", table.info(id).descriptor().display());

      AtomicReference<FilesystemMountTable.OpenedMount> observed = new AtomicReference<>();
      FilesystemMountTable.AttachmentKey owner =
          FilesystemMountTable.AttachmentKey.session(SessionModel.SessionId.parse("REFRESH"));
      table.attach(owner, id, observed::set);
      assertEquals("revision-2", observed.get().descriptor().revision());
      table.detach(owner, ignored -> {});

      filesystem.expandAuthority();
      assertFailure(() -> table.info(id));
      assertFailure(
          () ->
              table.attach(
                  FilesystemMountTable.AttachmentKey.session(
                      SessionModel.SessionId.parse("DENIED")),
                  id,
                  ignored -> {}));
      assertEquals(0, table.attachmentCount());
      join(table.close(id));
    } finally {
      scheduler.shutdownNow();
    }
  }

  private static final class Factory implements IFilesystemFactory {
    private final IFilesystem filesystem;

    Factory(IFilesystem filesystem) {
      this.filesystem = filesystem;
    }

    @Override
    public String kind() {
      return "mutable";
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      return CompletableFuture.completedFuture(filesystem);
    }
  }

  private static final class MutableFilesystem implements IFilesystem {
    private final AtomicReference<Descriptor> descriptor =
        new AtomicReference<>(descriptor("mutable", "revision-1", baseCapabilities()));

    void refresh(String revision) {
      Descriptor current = descriptor.get();
      descriptor.set(
          new Descriptor(
              current.kind(),
              "mutable updated",
              current.readOnly(),
              current.capabilities(),
              revision,
              Map.of("provider/generation", 2L)));
    }

    void expandAuthority() {
      descriptor.set(
          descriptor(
              "mutable",
              descriptor.get().revision(),
              Set.of(
                  Capability.READ,
                  Capability.WRITE,
                  Capability.ENTRIES,
                  Capability.DELETE,
                  Capability.REVISION_CHECK)));
    }

    private static Set<Capability> baseCapabilities() {
      return Set.of(
          Capability.READ,
          Capability.WRITE,
          Capability.ENTRIES,
          Capability.REVISION_CHECK);
    }

    private static Descriptor descriptor(
        String kind, String revision, Set<Capability> capabilities) {
      return new Descriptor(
          kind,
          kind,
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
      return CompletableFuture.completedFuture(null);
    }

    private static <T> CompletionStage<T> unsupported() {
      return CompletableFuture.failedFuture(new UnsupportedOperationException("fixture"));
    }
  }

  private static void assertFailure(Runnable operation) {
    IllegalStateException error = assertThrows(IllegalStateException.class, operation::run);
    assertTrue(error.getMessage().startsWith("FILESYSTEM_DESCRIPTOR_AUTHORITY_CHANGED"));
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
