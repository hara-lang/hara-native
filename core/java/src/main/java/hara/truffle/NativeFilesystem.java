package hara.truffle;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executor;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/** Asynchronous provider-neutral adapter around the confined local filesystem implementation. */
final class NativeFilesystem implements IFilesystem {
  static final class Factory implements IFilesystemFactory {
    @Override
    public String kind() {
      return "native";
    }

    @Override
    public void validate(Map<String, ?> configuration) {
      IFilesystemFactory.super.validate(configuration);
      Object root = configuration.get("root");
      if (!(root instanceof String value) || value.isBlank()) {
        throw new IllegalArgumentException("native filesystem root is required");
      }
      Set<String> allowed = Set.of("root", "display");
      for (String key : configuration.keySet()) {
        if (!allowed.contains(key)) {
          throw new IllegalArgumentException("unknown native filesystem option " + key);
        }
      }
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      return CompletableFuture.supplyAsync(
          () -> {
            String root = (String) configuration.get("root");
            Path rootPath = Path.of(root).toAbsolutePath().normalize();
            if (!Files.isDirectory(rootPath)) {
              throw new IllegalArgumentException("FILESYSTEM_NOT_FOUND " + rootPath);
            }
            String display =
                configuration.get("display") instanceof String value && !value.isBlank()
                    ? value
                    : "native mount";
            return new NativeFilesystem(
                new HaraFileProvider(rootPath),
                display,
                context.ioExecutor(),
                context.scheduler());
          },
          context.ioExecutor());
    }
  }

  @FunctionalInterface
  private interface Operation<T> {
    T call() throws Exception;
  }

  private final HaraFileProvider provider;
  private final Descriptor descriptor;
  private final Executor ioExecutor;
  private final ScheduledExecutorService scheduler;
  private record PendingOperation(
      CompletableFuture<?> result, String operation, String path, String target) {}

  private final AtomicBoolean closed = new AtomicBoolean();
  private final Object lifecycle = new Object();
  private final Set<PendingOperation> pending = ConcurrentHashMap.newKeySet();

  NativeFilesystem(
      HaraFileProvider provider,
      String display,
      Executor ioExecutor,
      ScheduledExecutorService scheduler) {
    this.provider = Objects.requireNonNull(provider, "native file provider");
    this.ioExecutor = Objects.requireNonNull(ioExecutor, "filesystem I/O executor");
    this.scheduler = Objects.requireNonNull(scheduler, "filesystem scheduler");
    this.descriptor =
        new Descriptor(
            "native", display, false, Capabilities.nativeReadWrite(), null, Map.of());
  }

  @Override
  public Descriptor descriptor() {
    return descriptor;
  }

  @Override
  public CompletionStage<Entry> stat(CallContext context, String path) {
    Objects.requireNonNull(path, "filesystem path");
    return submit(
        context,
        "stat",
        path,
        null,
        () -> {
          String logical = normalise(path);
          return entry(provider.stat(logical));
        });
  }

  @Override
  public CompletionStage<byte[]> read(CallContext context, String path) {
    Objects.requireNonNull(path, "filesystem path");
    return submit(context, "read", path, null, () -> provider.read(normalise(path)));
  }

  @Override
  public CompletionStage<Mutation> write(
      CallContext context,
      String path,
      byte[] bytes,
      WriteOptions options,
      MutationContext mutation) {
    Objects.requireNonNull(path, "filesystem path");
    Objects.requireNonNull(bytes, "filesystem bytes");
    HaraFileProvider.WriteMode mode =
        switch (Objects.requireNonNull(options, "write options").mode()) {
          case CREATE -> HaraFileProvider.WriteMode.CREATE;
          case REPLACE -> HaraFileProvider.WriteMode.REPLACE;
          case APPEND -> HaraFileProvider.WriteMode.APPEND;
        };
    byte[] frozen = bytes.clone();
    return submit(
        context,
        "write",
        path,
        null,
        () -> {
          String logical = normalise(path);
          requireNoRevision(mutation, "write", logical, null);
          return Mutation.path(
              provider.write(
                  logical,
                  frozen,
                  new HaraFileProvider.WriteOptions(mode, options.parents())));
        });
  }

  @Override
  public CompletionStage<EntryPage> entriesPage(
      CallContext context, String path, PageRequest request) {
    Objects.requireNonNull(path, "filesystem path");
    Objects.requireNonNull(request, "filesystem page request");
    return submit(
        context,
        "entries",
        path,
        null,
        () -> {
          String logical = normalise(path);
          List<HaraFileProvider.Entry> values = provider.entries(logical);
          int start = pageOffset(request.token(), values.size());
          int end = Math.min(values.size(), Math.addExact(start, request.limit()));
          ArrayList<Entry> entries = new ArrayList<>(end - start);
          for (HaraFileProvider.Entry value : values.subList(start, end)) entries.add(entry(value));
          return new EntryPage(entries, end < values.size() ? Integer.toString(end) : null);
        });
  }

  @Override
  public CompletionStage<Mutation> mkdir(
      CallContext context, String path, MkdirOptions options, MutationContext mutation) {
    Objects.requireNonNull(path, "filesystem path");
    Objects.requireNonNull(options, "mkdir options");
    return submit(
        context,
        "mkdir",
        path,
        null,
        () -> {
          String logical = normalise(path);
          requireNoRevision(mutation, "mkdir", logical, null);
          return Mutation.path(
              provider.mkdir(
                  logical,
                  new HaraFileProvider.MkdirOptions(options.parents(), options.existsOk())));
        });
  }

  @Override
  public CompletionStage<Mutation> delete(
      CallContext context, String path, DeleteOptions options, MutationContext mutation) {
    Objects.requireNonNull(path, "filesystem path");
    Objects.requireNonNull(options, "delete options");
    return submit(
        context,
        "delete",
        path,
        null,
        () -> {
          String logical = normalise(path);
          requireNoRevision(mutation, "delete", logical, null);
          return Mutation.path(
              provider.delete(logical, new HaraFileProvider.DeleteOptions(options.missingOk())));
        });
  }

  @Override
  public CompletionStage<Mutation> copy(
      CallContext context,
      String source,
      String target,
      CopyOptions options,
      MutationContext mutation) {
    Objects.requireNonNull(source, "filesystem source");
    Objects.requireNonNull(target, "filesystem target");
    Objects.requireNonNull(options, "copy options");
    return submit(
        context,
        "copy",
        source,
        target,
        () -> {
          String logicalSource = normalise(source);
          String logicalTarget = normalise(target);
          requireNoRevision(mutation, "copy", logicalSource, logicalTarget);
          return Mutation.path(
              provider.copy(
                  logicalSource,
                  logicalTarget,
                  new HaraFileProvider.CopyOptions(
                      options.replace(), options.parents(), options.preserveModified())));
        });
  }

  @Override
  public CompletionStage<Mutation> move(
      CallContext context,
      String source,
      String target,
      MoveOptions options,
      MutationContext mutation) {
    Objects.requireNonNull(source, "filesystem source");
    Objects.requireNonNull(target, "filesystem target");
    Objects.requireNonNull(options, "move options");
    return submit(
        context,
        "move",
        source,
        target,
        () -> {
          String logicalSource = normalise(source);
          String logicalTarget = normalise(target);
          requireNoRevision(mutation, "move", logicalSource, logicalTarget);
          return Mutation.path(
              provider.move(
                  logicalSource,
                  logicalTarget,
                  new HaraFileProvider.MoveOptions(
                      options.replace(), options.parents(), options.atomic())));
        });
  }

  @Override
  public CompletionStage<Void> close(CallContext context) {
    Objects.requireNonNull(context, "filesystem call context");
    CompletableFuture<Void> result = new CompletableFuture<>();
    try {
      context.check("native", "close", null, null);
      List<PendingOperation> operations;
      synchronized (lifecycle) {
        if (!closed.compareAndSet(false, true)) {
          result.complete(null);
          return result;
        }
        operations = List.copyOf(pending);
        pending.clear();
      }
      for (PendingOperation pendingOperation : operations) {
        pendingOperation.result().completeExceptionally(
            FilesystemException.providerClosed(
                "native",
                pendingOperation.operation(),
                pendingOperation.path(),
                pendingOperation.target()));
      }
      result.complete(null);
    } catch (Throwable error) {
      result.completeExceptionally(error);
    }
    return result;
  }

  private <T> CompletionStage<T> submit(
      CallContext context,
      String operation,
      String path,
      String target,
      Operation<T> action) {
    Objects.requireNonNull(context, "filesystem call context");
    Objects.requireNonNull(action, "filesystem operation");
    CompletableFuture<T> result = new CompletableFuture<>();
    try {
      context.check("native", operation, path, target);
    } catch (FilesystemException error) {
      result.completeExceptionally(error);
      return result;
    }

    PendingOperation pendingOperation =
        new PendingOperation(result, operation, path, target);
    synchronized (lifecycle) {
      if (closed.get()) {
        result.completeExceptionally(
            FilesystemException.providerClosed("native", operation, path, target));
        return result;
      }
      pending.add(pendingOperation);
    }
    AutoCloseable cancellation =
        context.onCancel(
            () ->
                result.completeExceptionally(
                    FilesystemException.cancelled("native", operation, path, target)));
    ScheduledFuture<?> deadline =
        scheduleDeadline(context, result, operation, path, target);
    result.whenComplete(
        (value, error) -> {
          pending.remove(pendingOperation);
          if (deadline != null) deadline.cancel(false);
          try {
            cancellation.close();
          } catch (Exception ignored) {
            // The cancellation registration is an in-memory removal only.
          }
        });

    try {
      ioExecutor.execute(
          () -> {
            if (result.isDone()) return;
            try {
              context.check("native", operation, path, target);
              if (closed.get()) {
                throw FilesystemException.providerClosed("native", operation, path, target);
              }
              T value = action.call();
              context.check("native", operation, path, target);
              result.complete(value);
            } catch (Throwable error) {
              result.completeExceptionally(
                  error instanceof FilesystemException filesystem
                      ? filesystem
                      : FilesystemException.fromLegacy(
                          "native", operation, path, target, error));
            }
          });
    } catch (Throwable error) {
      result.completeExceptionally(
          FilesystemException.fromLegacy("native", operation, path, target, error));
    }
    return result;
  }

  private ScheduledFuture<?> scheduleDeadline(
      CallContext context,
      CompletableFuture<?> result,
      String operation,
      String path,
      String target) {
    if (!context.hasDeadline()) return null;
    long delay = context.remainingNanos();
    try {
      return scheduler.schedule(
          () ->
              result.completeExceptionally(
                  FilesystemException.timeout("native", operation, path, target)),
          delay,
          TimeUnit.NANOSECONDS);
    } catch (Throwable error) {
      result.completeExceptionally(
          FilesystemException.fromLegacy("native", operation, path, target, error));
      return null;
    }
  }

  private static void requireNoRevision(
      MutationContext mutation, String operation, String path, String target) {
    if (mutation != null && mutation.required()) {
      throw FilesystemException.unsupportedRevision("native", operation, path, target);
    }
  }

  private static int pageOffset(String token, int size) {
    if (token == null) return 0;
    try {
      int offset = Integer.parseInt(token);
      if (offset < 0 || offset > size) throw new NumberFormatException();
      return offset;
    } catch (NumberFormatException error) {
      throw new FilesystemException(
          "invalid-path",
          "invalid filesystem page token",
          "native",
          "entries",
          null,
          null,
          "invalid-page-token",
          false,
          error);
    }
  }

  private static Entry entry(HaraFileProvider.Entry value) {
    EntryType type =
        switch (value.type()) {
          case "file" -> EntryType.FILE;
          case "directory" -> EntryType.DIRECTORY;
          case "symlink" -> EntryType.SYMLINK;
          default -> EntryType.OTHER;
        };
    return new Entry(
        value.path(),
        value.name(),
        type,
        value.size(),
        value.modifiedAt(),
        null,
        null,
        null,
        Map.of());
  }

  private static String normalise(String path) {
    return HaraLogicalPath.normalise(path);
  }
}
