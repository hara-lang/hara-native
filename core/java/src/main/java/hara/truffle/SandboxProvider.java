package hara.truffle;

import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;

/** Trusted host SPI for a sandbox backend. */
interface SandboxProvider {
  String name();

  boolean secure();

  record ResolvedSpec(
      SandboxModel.SandboxSpec spec,
      java.util.Map<String, byte[]> bundles,
      HaraMountedFileSystem mountProvider,
      FilesystemRuntimeBinding mountRuntime) {}

  SandboxInstance open(ResolvedSpec spec);

  interface SandboxInstance extends AutoCloseable {
    Pending<Object> eval(SandboxModel.EvaluationId evaluation, String source);

    Pending<Object> call(
        SandboxModel.EvaluationId evaluation, String callable, List<Object> arguments);

    boolean cancel(SandboxModel.EvaluationId evaluation);

    SandboxModel.EvaluationId activeEvaluation();

    SandboxModel.SandboxState state();

    SandboxModel.SandboxError error();

    @Override
    void close();
  }

  record Pending<T>(
      SandboxModel.EvaluationId evaluation,
      CompletableFuture<T> future,
      java.util.function.Predicate<SandboxModel.EvaluationId> cancellation) {
    T await() {
      try {
        return future.join();
      } catch (CompletionException error) {
        Throwable cause = error.getCause() == null ? error : error.getCause();
        if (cause instanceof RuntimeException runtime) throw runtime;
        throw error;
      }
    }

    boolean cancel() {
      return cancellation.test(evaluation);
    }
  }
}
