package hara.truffle;

import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Per-owner runtime binding for one opened filesystem capability.
 *
 * <p>The mount table owns provider lifetime. This binding owns only calls issued by one attached
 * Session or Sandbox: detach closes the binding, cancels its pending contexts, rejects every
 * unsettled result exactly once, and prevents later use without closing a provider shared by other
 * owners.
 */
final class FilesystemRuntimeBinding implements AutoCloseable {
  record Pending<T>(CompletableFuture<T> future, java.util.function.BooleanSupplier cancellation) {
    Pending {
      Objects.requireNonNull(future, "filesystem pending future");
      Objects.requireNonNull(cancellation, "filesystem pending cancellation");
    }

    boolean cancel() {
      return cancellation.getAsBoolean();
    }
  }

  private static final class ActiveCall {
    final IFilesystem.CallContext context;
    final CompletableFuture<?> result;
    final String operation;
    final String path;
    final String target;

    ActiveCall(
        IFilesystem.CallContext context,
        CompletableFuture<?> result,
        String operation,
        String path,
        String target) {
      this.context = context;
      this.result = result;
      this.operation = operation;
      this.path = path;
      this.target = target;
    }
  }

  @FunctionalInterface
  private interface Invocation<T> {
    CompletionStage<T> invoke(IFilesystem.CallContext context);
  }

  private final IFilesystem filesystem;
  private final IFilesystem.Descriptor admittedDescriptor;
  private final Set<ActiveCall> active = ConcurrentHashMap.newKeySet();
  private final AtomicBoolean closed = new AtomicBoolean();
  private final AtomicLong sequence = new AtomicLong(1);
  private final Object lifecycle = new Object();

  FilesystemRuntimeBinding(IFilesystem filesystem) {
    this.filesystem = Objects.requireNonNull(filesystem, "filesystem");
    this.admittedDescriptor =
        Objects.requireNonNull(filesystem.descriptor(), "filesystem descriptor");
  }

  IFilesystem.Descriptor descriptor() {
    synchronized (lifecycle) {
      return currentDescriptor();
    }
  }

  IFilesystem filesystem() {
    return filesystem;
  }

  boolean closed() {
    return closed.get();
  }

  int pendingCount() {
    return active.size();
  }

  Pending<IFilesystem.Entry> stat(String path) {
    String logical = HaraLogicalPath.normalise(path);
    return submit(
        "stat",
        logical,
        null,
        List.of(IFilesystem.Capability.READ),
        context -> filesystem.stat(context, logical));
  }

  Pending<byte[]> read(String path) {
    String logical = HaraLogicalPath.normalise(path);
    return submit(
        "read",
        logical,
        null,
        List.of(IFilesystem.Capability.READ),
        context -> filesystem.read(context, logical));
  }

  Pending<IFilesystem.Mutation> write(
      String path,
      byte[] bytes,
      IFilesystem.WriteOptions options,
      IFilesystem.MutationContext mutation) {
    String logical = HaraLogicalPath.normalise(path);
    byte[] frozen = Objects.requireNonNull(bytes, "filesystem bytes").clone();
    IFilesystem.WriteOptions validated = Objects.requireNonNull(options, "write options");
    IFilesystem.MutationContext expected =
        mutation == null ? IFilesystem.MutationContext.none() : mutation;
    ArrayList<IFilesystem.Capability> required =
        new ArrayList<>(List.of(IFilesystem.Capability.WRITE));
    if (validated.mode() == IFilesystem.WriteMode.APPEND) {
      required.add(IFilesystem.Capability.APPEND);
    }
    requireRevisionCapability(required, expected);
    return submit(
        "write",
        logical,
        null,
        required,
        context -> filesystem.write(context, logical, frozen, validated, expected));
  }

  Pending<Boolean> exists(String path) {
    String logical = HaraLogicalPath.normalise(path);
    return submit(
        "exists?",
        logical,
        null,
        List.of(IFilesystem.Capability.READ),
        context -> FilesystemEffects.exists(filesystem, context, logical));
  }

  Pending<List<IFilesystem.Entry>> entries(String path) {
    String logical = HaraLogicalPath.normalise(path);
    return submit(
        "entries",
        logical,
        null,
        List.of(IFilesystem.Capability.ENTRIES),
        context -> FilesystemEffects.entries(filesystem, context, logical));
  }

  Pending<List<String>> list(String path) {
    String logical = HaraLogicalPath.normalise(path);
    return submit(
        "list",
        logical,
        null,
        List.of(IFilesystem.Capability.ENTRIES),
        context -> FilesystemEffects.list(filesystem, context, logical));
  }

  Pending<List<String>> walk(String path) {
    String logical = HaraLogicalPath.normalise(path);
    return submit(
        "walk",
        logical,
        null,
        List.of(IFilesystem.Capability.READ, IFilesystem.Capability.ENTRIES),
        context -> FilesystemEffects.walk(filesystem, context, logical));
  }

  Pending<IFilesystem.Mutation> mkdir(
      String path,
      IFilesystem.MkdirOptions options,
      IFilesystem.MutationContext mutation) {
    String logical = HaraLogicalPath.normalise(path);
    IFilesystem.MkdirOptions validated = Objects.requireNonNull(options, "mkdir options");
    IFilesystem.MutationContext expected =
        mutation == null ? IFilesystem.MutationContext.none() : mutation;
    ArrayList<IFilesystem.Capability> required =
        new ArrayList<>(List.of(IFilesystem.Capability.MKDIR));
    requireRevisionCapability(required, expected);
    return submit(
        "mkdir",
        logical,
        null,
        required,
        context -> filesystem.mkdir(context, logical, validated, expected));
  }

  Pending<IFilesystem.Mutation> delete(
      String path,
      IFilesystem.DeleteOptions options,
      IFilesystem.MutationContext mutation) {
    String logical = HaraLogicalPath.normalise(path);
    IFilesystem.DeleteOptions validated = Objects.requireNonNull(options, "delete options");
    IFilesystem.MutationContext expected =
        mutation == null ? IFilesystem.MutationContext.none() : mutation;
    ArrayList<IFilesystem.Capability> required =
        new ArrayList<>(List.of(IFilesystem.Capability.DELETE));
    requireRevisionCapability(required, expected);
    return submit(
        "delete",
        logical,
        null,
        required,
        context -> filesystem.delete(context, logical, validated, expected));
  }

  Pending<IFilesystem.Mutation> copy(
      String source,
      String target,
      IFilesystem.CopyOptions options,
      IFilesystem.MutationContext mutation) {
    String logicalSource = HaraLogicalPath.normalise(source);
    String logicalTarget = HaraLogicalPath.normalise(target);
    IFilesystem.CopyOptions validated = Objects.requireNonNull(options, "copy options");
    IFilesystem.MutationContext expected =
        mutation == null ? IFilesystem.MutationContext.none() : mutation;
    ArrayList<IFilesystem.Capability> required =
        new ArrayList<>(List.of(IFilesystem.Capability.COPY));
    if (validated.preserveModified()) {
      required.add(IFilesystem.Capability.PRESERVE_MODIFIED);
    }
    requireRevisionCapability(required, expected);
    return submit(
        "copy",
        logicalSource,
        logicalTarget,
        required,
        context ->
            filesystem.copy(
                context,
                logicalSource,
                logicalTarget,
                validated,
                expected));
  }

  Pending<IFilesystem.Mutation> move(
      String source,
      String target,
      IFilesystem.MoveOptions options,
      IFilesystem.MutationContext mutation) {
    String logicalSource = HaraLogicalPath.normalise(source);
    String logicalTarget = HaraLogicalPath.normalise(target);
    IFilesystem.MoveOptions validated = Objects.requireNonNull(options, "move options");
    IFilesystem.MutationContext expected =
        mutation == null ? IFilesystem.MutationContext.none() : mutation;
    ArrayList<IFilesystem.Capability> required =
        new ArrayList<>(List.of(IFilesystem.Capability.MOVE));
    if (validated.atomic()) required.add(IFilesystem.Capability.ATOMIC_MOVE);
    requireRevisionCapability(required, expected);
    return submit(
        "move",
        logicalSource,
        logicalTarget,
        required,
        context ->
            filesystem.move(
                context,
                logicalSource,
                logicalTarget,
                validated,
                expected));
  }

  Pending<String> tempFile(String parent, String prefix, String suffix) {
    String logical = HaraLogicalPath.normalise(parent);
    return submit(
        "temp-file",
        logical,
        null,
        List.of(IFilesystem.Capability.READ, IFilesystem.Capability.WRITE),
        context ->
            FilesystemEffects.tempFile(filesystem, context, logical, prefix, suffix));
  }

  Pending<String> tempDirectory(String parent, String prefix) {
    String logical = HaraLogicalPath.normalise(parent);
    return submit(
        "temp-directory",
        logical,
        null,
        List.of(IFilesystem.Capability.READ, IFilesystem.Capability.MKDIR),
        context ->
            FilesystemEffects.tempDirectory(filesystem, context, logical, prefix));
  }

  private <T> Pending<T> submit(
      String operation,
      String path,
      String target,
      List<IFilesystem.Capability> capabilities,
      Invocation<T> invocation) {
    Objects.requireNonNull(capabilities, "filesystem required capabilities");
    Objects.requireNonNull(invocation, "filesystem invocation");
    CompletableFuture<T> result = new CompletableFuture<>();
    IFilesystem.CallContext context =
        IFilesystem.CallContext.create()
            .withTraceId(
                "filesystem/"
                    + admittedDescriptor.kind()
                    + "/"
                    + operation
                    + "/"
                    + sequence.getAndIncrement());
    ActiveCall call = new ActiveCall(context, result, operation, path, target);
    CompletionStage<T> stage;

    synchronized (lifecycle) {
      if (closed.get()) {
        result.completeExceptionally(closedFailure(operation, path, target));
        return new Pending<>(result, () -> false);
      }
      try {
        requireCapabilities(capabilities, operation, path, target);
        active.add(call);
        result.whenComplete((value, error) -> active.remove(call));
        stage =
            Objects.requireNonNull(
                invocation.invoke(context), "filesystem operation stage");
      } catch (Throwable error) {
        result.completeExceptionally(mapFailure(error, operation, path, target));
        return new Pending<>(result, () -> false);
      }
    }

    stage.whenComplete(
        (value, error) -> {
          synchronized (lifecycle) {
            if (result.isDone()) return;
            if (closed.get()) {
              result.completeExceptionally(closedFailure(operation, path, target));
            } else if (error == null) {
              result.complete(value);
            } else {
              result.completeExceptionally(mapFailure(error, operation, path, target));
            }
          }
        });
    return new Pending<>(result, () -> cancel(call));
  }

  private boolean cancel(ActiveCall call) {
    synchronized (lifecycle) {
      if (call.result.isDone()) return false;
      boolean requested = call.context.cancel();
      boolean settled =
          call.result.completeExceptionally(
              FilesystemException.cancelled(
                  admittedDescriptor.kind(), call.operation, call.path, call.target));
      return requested || settled;
    }
  }

  @Override
  public void close() {
    synchronized (lifecycle) {
      if (!closed.compareAndSet(false, true)) return;
      for (ActiveCall call : List.copyOf(active)) {
        call.context.cancel();
        call.result.completeExceptionally(
            closedFailure(call.operation, call.path, call.target));
      }
      active.clear();
    }
  }

  private void requireCapabilities(
      List<IFilesystem.Capability> required,
      String operation,
      String path,
      String target) {
    IFilesystem.Descriptor current = currentDescriptor();
    for (IFilesystem.Capability capability : required) {
      if (current.capabilities().contains(capability)) continue;
      throw new FilesystemException(
          "unsupported",
          "filesystem provider does not advertise " + capability.keyword(),
          admittedDescriptor.kind(),
          operation,
          path,
          target,
          "capability-unavailable:" + capability.keyword(),
          false,
          null);
    }
  }

  private IFilesystem.Descriptor currentDescriptor() {
    IFilesystem.Descriptor current =
        Objects.requireNonNull(filesystem.descriptor(), "filesystem descriptor");
    if (!admittedDescriptor.kind().equals(current.kind())
        || admittedDescriptor.readOnly() != current.readOnly()
        || !admittedDescriptor.capabilities().equals(current.capabilities())) {
      throw new FilesystemException(
          "io",
          "filesystem provider changed its authority descriptor",
          admittedDescriptor.kind(),
          "descriptor",
          null,
          null,
          "descriptor-authority-changed",
          false,
          null);
    }
    return current;
  }

  private static void requireRevisionCapability(
      List<IFilesystem.Capability> required,
      IFilesystem.MutationContext mutation) {
    if (mutation.required()) required.add(IFilesystem.Capability.REVISION_CHECK);
  }

  private FilesystemException closedFailure(String operation, String path, String target) {
    return FilesystemException.providerClosed(
        admittedDescriptor.kind(), operation, path, target);
  }

  private FilesystemException mapFailure(
      Throwable error, String operation, String path, String target) {
    Throwable cause = unwrap(error);
    if (cause instanceof FilesystemException filesystemFailure) return filesystemFailure;
    return new FilesystemException(
        "io",
        "filesystem operation failed",
        admittedDescriptor.kind(),
        operation,
        path,
        target,
        cause.getClass().getSimpleName(),
        false,
        cause);
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
