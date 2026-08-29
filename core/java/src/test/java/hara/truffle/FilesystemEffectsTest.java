package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;

public class FilesystemEffectsTest {
  @Test
  public void entriesConsumeEveryOpaquePageAndSortByCanonicalPath() {
    Fixture filesystem = new Fixture();
    filesystem.pages.put(
        null,
        new IFilesystem.EntryPage(
            List.of(filesystem.file("/z")), "opaque-next"));
    filesystem.pages.put(
        "opaque-next",
        new IFilesystem.EntryPage(
            List.of(filesystem.file("/a")), null));

    List<IFilesystem.Entry> entries =
        join(
            FilesystemEffects.entries(
                filesystem, IFilesystem.CallContext.create(), "/", 1));
    assertEquals(List.of("/a", "/z"), entries.stream().map(IFilesystem.Entry::path).toList());
    assertEquals(java.util.Arrays.asList(null, "opaque-next"), filesystem.pageRequests);
  }

  @Test
  public void repeatedPageTokensFailClosedInsteadOfLooping() {
    Fixture filesystem = new Fixture();
    filesystem.pages.put(
        null,
        new IFilesystem.EntryPage(List.of(), "again"));
    filesystem.pages.put(
        "again",
        new IFilesystem.EntryPage(List.of(), "again"));

    FilesystemException error =
        assertThrows(
            FilesystemException.class,
            () ->
                join(
                    FilesystemEffects.entries(
                        filesystem, IFilesystem.CallContext.create(), "/", 1)));
    assertEquals("io", error.code());
    assertEquals("page-token-cycle", error.providerCode());
  }

  @Test
  public void walkRecursesOnlyIntoDirectoriesAndTreatsLinksAsLeaves() {
    Fixture filesystem = new Fixture();
    filesystem.nodes.put("/", filesystem.directory("/", "root"));
    filesystem.nodes.put("/dir", filesystem.directory("/dir", "dir"));
    filesystem.nodes.put("/dir/value", filesystem.file("/dir/value"));
    filesystem.nodes.put("/file", filesystem.file("/file"));
    filesystem.nodes.put("/link", filesystem.link("/link"));
    filesystem.directoryPages.put(
        "/",
        new IFilesystem.EntryPage(
            List.of(
                filesystem.nodes.get("/link"),
                filesystem.nodes.get("/dir"),
                filesystem.nodes.get("/file")),
            null));
    filesystem.directoryPages.put(
        "/dir",
        new IFilesystem.EntryPage(
            List.of(filesystem.nodes.get("/dir/value")), null));

    assertEquals(
        List.of("/dir/value", "/file", "/link"),
        join(
            FilesystemEffects.walk(
                filesystem, IFilesystem.CallContext.create(), "/")));
    assertEquals(List.of("/", "/dir"), filesystem.entryDirectories);
    assertFalse(filesystem.entryDirectories.contains("/link"));
  }

  @Test
  public void existsOnlyConvertsNotFoundIntoFalse() {
    Fixture filesystem = new Fixture();
    filesystem.nodes.put("/present", filesystem.file("/present"));
    assertTrue(
        join(
            FilesystemEffects.exists(
                filesystem, IFilesystem.CallContext.create(), "/present")));
    assertFalse(
        join(
            FilesystemEffects.exists(
                filesystem, IFilesystem.CallContext.create(), "/missing")));

    filesystem.statFailure =
        new FilesystemException(
            "permission-denied",
            "denied",
            "fixture",
            "stat",
            "/present",
            null,
            "denied",
            false,
            null);
    FilesystemException error =
        assertThrows(
            FilesystemException.class,
            () ->
                join(
                    FilesystemEffects.exists(
                        filesystem, IFilesystem.CallContext.create(), "/present")));
    assertEquals("permission-denied", error.code());
  }

  @Test
  public void temporaryEntriesUseExclusiveCreationAndRetryCollisions() {
    Fixture filesystem = new Fixture();
    filesystem.nodes.put("/tmp", filesystem.directory("/tmp", "tmp"));
    filesystem.collisions.set(2);

    String file =
        join(
            FilesystemEffects.tempFile(
                filesystem,
                IFilesystem.CallContext.create(),
                "/tmp",
                "scratch",
                ".bin"));
    assertTrue(file.startsWith("/tmp/scratch-"));
    assertTrue(file.endsWith(".bin"));
    assertEquals(3, filesystem.createCalls.get());
    assertEquals(IFilesystem.WriteMode.CREATE, filesystem.lastWriteOptions.mode());
    assertFalse(filesystem.lastWriteOptions.parents());

    filesystem.collisions.set(1);
    filesystem.createCalls.set(0);
    String directory =
        join(
            FilesystemEffects.tempDirectory(
                filesystem,
                IFilesystem.CallContext.create(),
                "/tmp",
                "work"));
    assertTrue(directory.startsWith("/tmp/work-"));
    assertEquals(2, filesystem.createCalls.get());
    assertFalse(filesystem.lastMkdirOptions.parents());
    assertFalse(filesystem.lastMkdirOptions.existsOk());
  }

  @Test
  public void walkRejectsProviderIdentityCycles() {
    Fixture filesystem = new Fixture();
    filesystem.nodes.put("/", filesystem.directory("/", "same"));
    filesystem.nodes.put("/loop", filesystem.directory("/loop", "same"));
    filesystem.directoryPages.put(
        "/",
        new IFilesystem.EntryPage(List.of(filesystem.nodes.get("/loop")), null));

    FilesystemException error =
        assertThrows(
            FilesystemException.class,
            () ->
                join(
                    FilesystemEffects.walk(
                        filesystem, IFilesystem.CallContext.create(), "/")));
    assertEquals("io", error.code());
    assertEquals("cycle-detected", error.providerCode());
  }

  private static final class Fixture implements IFilesystem {
    final Map<String, Entry> nodes = new HashMap<>();
    final Map<String, EntryPage> pages = new HashMap<>();
    final Map<String, EntryPage> directoryPages = new HashMap<>();
    final List<String> pageRequests = new ArrayList<>();
    final List<String> entryDirectories = new ArrayList<>();
    final AtomicInteger collisions = new AtomicInteger();
    final AtomicInteger createCalls = new AtomicInteger();
    WriteOptions lastWriteOptions;
    MkdirOptions lastMkdirOptions;
    RuntimeException statFailure;

    @Override
    public Descriptor descriptor() {
      return new Descriptor(
          "fixture",
          "fixture",
          false,
          Capabilities.of(
              Capability.READ,
              Capability.WRITE,
              Capability.ENTRIES,
              Capability.MKDIR),
          "1",
          Map.of());
    }

    @Override
    public CompletionStage<Entry> stat(CallContext context, String path) {
      if (statFailure != null) return CompletableFuture.failedFuture(statFailure);
      Entry entry = nodes.get(path);
      return entry == null
          ? CompletableFuture.failedFuture(notFound("stat", path))
          : CompletableFuture.completedFuture(entry);
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
      createCalls.incrementAndGet();
      lastWriteOptions = options;
      if (collisions.getAndUpdate(value -> Math.max(0, value - 1)) > 0) {
        return CompletableFuture.failedFuture(alreadyExists("write", path));
      }
      return CompletableFuture.completedFuture(Mutation.path(path));
    }

    @Override
    public CompletionStage<EntryPage> entriesPage(
        CallContext context, String path, PageRequest request) {
      if (directoryPages.containsKey(path)) {
        entryDirectories.add(path);
        return CompletableFuture.completedFuture(directoryPages.get(path));
      }
      pageRequests.add(request.token());
      EntryPage page = pages.get(request.token());
      return page == null
          ? CompletableFuture.failedFuture(notFound("entries", path))
          : CompletableFuture.completedFuture(page);
    }

    @Override
    public CompletionStage<Mutation> mkdir(
        CallContext context,
        String path,
        MkdirOptions options,
        MutationContext mutation) {
      createCalls.incrementAndGet();
      lastMkdirOptions = options;
      if (collisions.getAndUpdate(value -> Math.max(0, value - 1)) > 0) {
        return CompletableFuture.failedFuture(alreadyExists("mkdir", path));
      }
      return CompletableFuture.completedFuture(Mutation.path(path));
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

    Entry file(String path) {
      return entry(path, EntryType.FILE, path);
    }

    Entry link(String path) {
      return entry(path, EntryType.SYMLINK, path);
    }

    Entry directory(String path, String id) {
      return entry(path, EntryType.DIRECTORY, id);
    }

    private Entry entry(String path, EntryType type, String id) {
      return new Entry(
          path,
          HaraLogicalPath.fileName(path),
          type,
          type == EntryType.FILE ? 0L : null,
          null,
          id,
          "1",
          null,
          Map.of());
    }

    private static FilesystemException notFound(String operation, String path) {
      return new FilesystemException(
          "not-found",
          "missing",
          "fixture",
          operation,
          path,
          null,
          "missing",
          false,
          null);
    }

    private static FilesystemException alreadyExists(String operation, String path) {
      return new FilesystemException(
          "already-exists",
          "collision",
          "fixture",
          operation,
          path,
          null,
          "collision",
          false,
          null);
    }

    private static <T> CompletionStage<T> unsupported() {
      return CompletableFuture.failedFuture(new UnsupportedOperationException("fixture"));
    }
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
