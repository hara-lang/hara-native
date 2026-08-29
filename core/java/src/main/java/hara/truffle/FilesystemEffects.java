package hara.truffle;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Provider-neutral implementations of public filesystem operations derived from the primitive
 * {@link IFilesystem} surface.
 *
 * <p>These helpers preserve deterministic ordering, opaque pagination, no-follow traversal, and
 * exclusive temporary-entry creation without giving providers or guest code additional authority.
 */
final class FilesystemEffects {
  static final int DEFAULT_PAGE_LIMIT = 256;
  static final int MAX_PAGE_LIMIT = 1024;
  private static final int MAX_PAGES = 4096;
  private static final int MAX_TEMP_ATTEMPTS = 1024;
  private static final AtomicLong TEMP_SEQUENCE = new AtomicLong(1);

  private FilesystemEffects() {}

  static CompletionStage<Boolean> exists(
      IFilesystem filesystem, IFilesystem.CallContext context, String path) {
    Objects.requireNonNull(filesystem, "filesystem");
    Objects.requireNonNull(context, "filesystem call context");
    String logical = HaraLogicalPath.normalise(path);
    try {
      context.check(kind(filesystem), "stat", logical, null);
      return filesystem
          .stat(context, logical)
          .handle(
              (entry, error) -> {
                if (error == null) return true;
                Throwable cause = unwrap(error);
                if (cause instanceof FilesystemException failure
                    && "not-found".equals(failure.code())) {
                  return false;
                }
                throw completion(cause);
              });
    } catch (Throwable error) {
      return failed(error);
    }
  }

  static CompletionStage<List<IFilesystem.Entry>> entries(
      IFilesystem filesystem, IFilesystem.CallContext context, String path) {
    return entries(filesystem, context, path, DEFAULT_PAGE_LIMIT);
  }

  static CompletionStage<List<IFilesystem.Entry>> entries(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String path,
      int pageLimit) {
    Objects.requireNonNull(filesystem, "filesystem");
    Objects.requireNonNull(context, "filesystem call context");
    String logical = HaraLogicalPath.normalise(path);
    if (pageLimit <= 0 || pageLimit > MAX_PAGE_LIMIT) {
      throw new IllegalArgumentException(
          "filesystem page limit must be between 1 and " + MAX_PAGE_LIMIT);
    }
    ArrayList<IFilesystem.Entry> output = new ArrayList<>();
    HashSet<String> tokens = new HashSet<>();
    return collectPages(
            filesystem, context, logical, pageLimit, null, 0, tokens, output)
        .thenApply(
            ignored -> {
              output.sort(java.util.Comparator.comparing(IFilesystem.Entry::path));
              for (int index = 1; index < output.size(); index++) {
                if (output.get(index - 1).path().equals(output.get(index).path())) {
                  throw failure(
                      "io",
                      "filesystem entries contain a duplicate path",
                      kind(filesystem),
                      "entries",
                      logical,
                      null,
                      "duplicate-entry-path",
                      false,
                      null);
                }
              }
              return List.copyOf(output);
            });
  }

  static CompletionStage<List<String>> list(
      IFilesystem filesystem, IFilesystem.CallContext context, String path) {
    return entries(filesystem, context, path)
        .thenApply(values -> values.stream().map(IFilesystem.Entry::path).toList());
  }

  static CompletionStage<List<String>> walk(
      IFilesystem filesystem, IFilesystem.CallContext context, String path) {
    Objects.requireNonNull(filesystem, "filesystem");
    Objects.requireNonNull(context, "filesystem call context");
    String logical = HaraLogicalPath.normalise(path);
    ArrayList<String> output = new ArrayList<>();
    return filesystem
        .stat(context, logical)
        .thenCompose(
            entry -> walkEntry(filesystem, context, entry, Set.of(), output))
        .thenApply(
            ignored -> {
              output.sort(String::compareTo);
              return List.copyOf(output);
            });
  }

  static CompletionStage<String> tempFile(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String parent,
      String prefix,
      String suffix) {
    return temporary(filesystem, context, parent, prefix, suffix, false);
  }

  static CompletionStage<String> tempDirectory(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String parent,
      String prefix) {
    return temporary(filesystem, context, parent, prefix, "", true);
  }

  private static CompletionStage<Void> collectPages(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String path,
      int pageLimit,
      String token,
      int pageCount,
      Set<String> seenTokens,
      List<IFilesystem.Entry> output) {
    if (pageCount >= MAX_PAGES) {
      return failed(
          failure(
              "io",
              "filesystem entries exceed the page limit",
              kind(filesystem),
              "entries",
              path,
              null,
              "page-count-limit",
              false,
              null));
    }
    try {
      context.check(kind(filesystem), "entries", path, null);
      CompletionStage<IFilesystem.EntryPage> stage =
          Objects.requireNonNull(
              filesystem.entriesPage(
                  context, path, new IFilesystem.PageRequest(token, pageLimit)),
              "filesystem entries stage");
      return stage.thenCompose(
          page -> {
            if (page == null) {
              return failed(
                  failure(
                      "io",
                      "filesystem returned no entries page",
                      kind(filesystem),
                      "entries",
                      path,
                      null,
                      "missing-page",
                      false,
                      null));
            }
            output.addAll(page.entries());
            String next = page.nextToken();
            if (next == null) return CompletableFuture.completedFuture(null);
            if (!seenTokens.add(next)) {
              return failed(
                  failure(
                      "io",
                      "filesystem repeated an entries page token",
                      kind(filesystem),
                      "entries",
                      path,
                      null,
                      "page-token-cycle",
                      false,
                      null));
            }
            return collectPages(
                filesystem,
                context,
                path,
                pageLimit,
                next,
                pageCount + 1,
                seenTokens,
                output);
          });
    } catch (Throwable error) {
      return failed(error);
    }
  }

  private static CompletionStage<Void> walkEntry(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      IFilesystem.Entry entry,
      Set<String> ancestors,
      List<String> output) {
    if (entry.type() != IFilesystem.EntryType.DIRECTORY) {
      output.add(entry.path());
      return CompletableFuture.completedFuture(null);
    }
    String identity = entry.id() == null ? entry.path() : entry.id();
    if (ancestors.contains(identity)) {
      return failed(
          failure(
              "io",
              "filesystem traversal cycle detected",
              kind(filesystem),
              "walk",
              entry.path(),
              null,
              "cycle-detected",
              false,
              null));
    }
    HashSet<String> descendants = new HashSet<>(ancestors);
    descendants.add(identity);
    return entries(filesystem, context, entry.path())
        .thenCompose(
            children -> {
              CompletionStage<Void> chain = CompletableFuture.completedFuture(null);
              for (IFilesystem.Entry child : children) {
                chain =
                    chain.thenCompose(
                        ignored ->
                            walkEntry(
                                filesystem,
                                context,
                                child,
                                descendants,
                                output));
              }
              return chain;
            });
  }

  private static CompletionStage<String> temporary(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String parent,
      String prefix,
      String suffix,
      boolean directory) {
    Objects.requireNonNull(filesystem, "filesystem");
    Objects.requireNonNull(context, "filesystem call context");
    HaraLogicalPath.validateFragment(prefix, "temporary entry prefix");
    HaraLogicalPath.validateFragment(suffix, "temporary entry suffix");
    String logicalParent = HaraLogicalPath.normalise(parent);
    return filesystem
        .stat(context, logicalParent)
        .thenCompose(
            entry -> {
              if (entry.type() != IFilesystem.EntryType.DIRECTORY) {
                return failed(
                    failure(
                        "not-directory",
                        "temporary parent is not a directory",
                        kind(filesystem),
                        directory ? "temp-directory" : "temp-file",
                        logicalParent,
                        null,
                        "parent-not-directory",
                        false,
                        null));
              }
              return attemptTemporary(
                  filesystem,
                  context,
                  logicalParent,
                  prefix,
                  suffix,
                  directory,
                  0);
            });
  }

  private static CompletionStage<String> attemptTemporary(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String parent,
      String prefix,
      String suffix,
      boolean directory,
      int attempt) {
    if (attempt >= MAX_TEMP_ATTEMPTS) {
      return failed(
          failure(
              "io",
              "unable to allocate a unique temporary entry",
              kind(filesystem),
              directory ? "temp-directory" : "temp-file",
              parent,
              null,
              "temporary-name-exhausted",
              false,
              null));
    }
    String name =
        prefix + "-" + String.format("%016x", TEMP_SEQUENCE.getAndIncrement()) + suffix;
    String path = HaraLogicalPath.join(parent, name);
    CompletionStage<IFilesystem.Mutation> creation;
    try {
      context.check(
          kind(filesystem), directory ? "temp-directory" : "temp-file", path, null);
      creation =
          directory
              ? filesystem.mkdir(
                  context,
                  path,
                  new IFilesystem.MkdirOptions(false, false),
                  IFilesystem.MutationContext.none())
              : filesystem.write(
                  context,
                  path,
                  new byte[0],
                  new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                  IFilesystem.MutationContext.none());
    } catch (Throwable error) {
      return retryTemporary(
          filesystem, context, parent, prefix, suffix, directory, attempt, error);
    }
    return creation
        .handle(
            (mutation, error) -> {
              if (error != null) throw completion(unwrap(error));
              return mutation == null ? path : mutation.path();
            })
        .handle(
            (value, error) -> {
              if (error == null) {
                return CompletableFuture.completedFuture(value);
              }
              return retryTemporary(
                  filesystem,
                  context,
                  parent,
                  prefix,
                  suffix,
                  directory,
                  attempt,
                  unwrap(error));
            })
        .thenCompose(stage -> stage);
  }

  private static CompletionStage<String> retryTemporary(
      IFilesystem filesystem,
      IFilesystem.CallContext context,
      String parent,
      String prefix,
      String suffix,
      boolean directory,
      int attempt,
      Throwable error) {
    Throwable cause = unwrap(error);
    if (cause instanceof FilesystemException failure
        && "already-exists".equals(failure.code())) {
      return attemptTemporary(
          filesystem, context, parent, prefix, suffix, directory, attempt + 1);
    }
    return failed(cause);
  }

  private static String kind(IFilesystem filesystem) {
    return filesystem.descriptor().kind();
  }

  private static FilesystemException failure(
      String code,
      String message,
      String provider,
      String operation,
      String path,
      String target,
      String providerCode,
      boolean retryable,
      Throwable cause) {
    return new FilesystemException(
        code,
        message,
        provider,
        operation,
        path,
        target,
        providerCode,
        retryable,
        cause);
  }

  private static CompletionException completion(Throwable error) {
    return error instanceof CompletionException completion
        ? completion
        : new CompletionException(error);
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

  private static <T> CompletionStage<T> failed(Throwable error) {
    return CompletableFuture.failedFuture(unwrap(error));
  }
}
