package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/** Conformance-only provider. In-process isolation is explicitly non-secure. */
final class InProcessSandboxProvider implements SandboxProvider {
  static final InProcessSandboxProvider INSTANCE = new InProcessSandboxProvider();

  private InProcessSandboxProvider() {}

  @Override
  public String name() {
    return "in-process";
  }

  @Override
  public boolean secure() {
    return false;
  }

  @Override
  public SandboxInstance open(ResolvedSpec resolved) {
    SandboxModel.SandboxSpec spec = resolved.spec();
    SessionKernel.Session session = SessionKernel.Session.privateSandbox(spec.entryNamespace());
    if (spec.mount() != null) {
      session.attachSandboxFilesystem(
          spec.mount(), resolved.mountProvider(), resolved.mountRuntime());
    }
    return new Instance(spec, session);
  }

  private static final class ActiveEvaluation {
    final SandboxModel.EvaluationId id;
    final CompletableFuture<Object> result = new CompletableFuture<>();
    volatile boolean cancelled;
    volatile boolean timedOut;

    ActiveEvaluation(SandboxModel.EvaluationId id) {
      this.id = id;
    }
  }

  private static final class Instance implements SandboxInstance {
    private final SandboxModel.SandboxSpec spec;
    private final SessionKernel.Session session;
    private final ExecutorService worker =
        Executors.newSingleThreadExecutor(
            task -> {
              Thread thread = new Thread(task, "hara-in-process-sandbox");
              thread.setDaemon(true);
              return thread;
            });
    private final ScheduledExecutorService deadlines =
        Executors.newSingleThreadScheduledExecutor(
            task -> {
              Thread thread = new Thread(task, "hara-in-process-sandbox-deadline");
              thread.setDaemon(true);
              return thread;
            });
    private volatile SandboxModel.SandboxState state = SandboxModel.SandboxState.OPEN;
    private volatile SandboxModel.SandboxError lastError;
    private ActiveEvaluation active;
    private boolean closed;

    private Instance(SandboxModel.SandboxSpec spec, SessionKernel.Session session) {
      this.spec = spec;
      this.session = session;
    }

    private synchronized ActiveEvaluation begin(SandboxModel.EvaluationId evaluation) {
      if (active != null) {
        throw new SandboxModel.SandboxException(SandboxModel.ErrorCode.BUSY, "sandbox is busy");
      }
      if (closed || state != SandboxModel.SandboxState.OPEN) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.CLOSED, "sandbox is terminal and cannot be reused");
      }
      ActiveEvaluation started = new ActiveEvaluation(evaluation);
      active = started;
      state = SandboxModel.SandboxState.RUNNING;
      lastError = null;
      deadlines.schedule(
          () -> timeout(evaluation), spec.limits().evaluationMillis(), TimeUnit.MILLISECONDS);
      return started;
    }

    @Override
    public Pending<Object> eval(SandboxModel.EvaluationId evaluation, String source) {
      if (source.getBytes(StandardCharsets.UTF_8).length > spec.limits().sourceBytes()) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.LIMIT_EXCEEDED, "sandbox source limit exceeded");
      }
      ActiveEvaluation started = begin(evaluation);
      worker.execute(() -> run(started, () -> session.evalTransfer(source)));
      return new Pending<>(evaluation, started.result, this::cancel);
    }

    @Override
    public Pending<Object> call(
        SandboxModel.EvaluationId evaluation, String callable, List<Object> arguments) {
      if (callable == null || !callable.matches("[A-Za-z0-9._-]+/[A-Za-z0-9._?!*+-]+")) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.INVALID_SPEC, "invalid sandbox callable");
      }
      ActiveEvaluation started = begin(evaluation);
      List<Object> frozen = List.copyOf(arguments);
      worker.execute(() -> run(started, () -> session.callTransfer(callable, frozen)));
      return new Pending<>(evaluation, started.result, this::cancel);
    }

    private void run(ActiveEvaluation evaluation, java.util.concurrent.Callable<Object> operation) {
      try {
        if (evaluation.cancelled) throw terminalError(evaluation);
        Object result = operation.call();
        if (String.valueOf(result).getBytes(StandardCharsets.UTF_8).length
            > spec.limits().resultBytes()) {
          throw new SandboxModel.SandboxException(
              SandboxModel.ErrorCode.LIMIT_EXCEEDED, "sandbox result limit exceeded");
        }
        finish(evaluation, result, null);
      } catch (Throwable error) {
        SandboxModel.SandboxException failure =
            evaluation.cancelled
                ? terminalError(evaluation)
                : error instanceof SandboxModel.SandboxException sandboxError
                    ? sandboxError
                    : error.getMessage() != null
                            && error.getMessage().contains("SESSION_TRANSFER_REJECTED")
                        ? new SandboxModel.SandboxException(
                            SandboxModel.ErrorCode.RESULT_NOT_TRANSFERABLE,
                            "sandbox result is not transferable")
                    : new SandboxModel.SandboxException(
                        SandboxModel.ErrorCode.EVALUATION_FAILED,
                        error.getMessage() == null ? error.getClass().getName() : error.getMessage());
        finish(evaluation, null, failure);
      }
    }

    private SandboxModel.SandboxException terminalError(ActiveEvaluation evaluation) {
      return new SandboxModel.SandboxException(
          evaluation.timedOut
              ? SandboxModel.ErrorCode.TIMEOUT
              : SandboxModel.ErrorCode.CANCELLED,
          evaluation.timedOut
              ? "sandbox evaluation timed out"
              : "sandbox evaluation cancelled");
    }

    private synchronized void finish(
        ActiveEvaluation evaluation, Object result, SandboxModel.SandboxException error) {
      if (active != evaluation) return;
      active = null;
      if (error == null) {
        state = SandboxModel.SandboxState.OPEN;
        evaluation.result.complete(result);
      } else {
        lastError = new SandboxModel.SandboxError(error.code(), error.getMessage());
        state =
            error.code() == SandboxModel.ErrorCode.CANCELLED
                ? SandboxModel.SandboxState.CANCELLED
                : SandboxModel.SandboxState.FAILED;
        evaluation.result.completeExceptionally(error);
      }
    }

    private synchronized void timeout(SandboxModel.EvaluationId evaluation) {
      if (active == null || !active.id.equals(evaluation)) return;
      active.timedOut = true;
      cancel(evaluation);
    }

    @Override
    public synchronized boolean cancel(SandboxModel.EvaluationId evaluation) {
      if (active == null || !active.id.equals(evaluation)) return false;
      if (active.cancelled) return true;
      active.cancelled = true;
      state = SandboxModel.SandboxState.CANCELLING;
      session.cancelEvaluation();
      return true;
    }

    @Override
    public synchronized SandboxModel.EvaluationId activeEvaluation() {
      return active == null ? null : active.id;
    }

    @Override
    public SandboxModel.SandboxState state() {
      return state;
    }

    @Override
    public SandboxModel.SandboxError error() {
      return lastError;
    }

    @Override
    public void close() {
      ActiveEvaluation evaluation;
      synchronized (this) {
        if (closed) return;
        closed = true;
        evaluation = active;
        if (evaluation != null) cancel(evaluation.id);
      }
      if (evaluation != null) {
        try {
          evaluation.result.join();
        } catch (RuntimeException ignored) {
          // Cancellation settlement is observed by the caller's pending handle.
        }
      }
      session.close();
      worker.shutdownNow();
      deadlines.shutdownNow();
    }
  }
}
