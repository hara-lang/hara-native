package hara.truffle;

import java.util.Map;
import java.util.Objects;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Executor;
import java.util.concurrent.ScheduledExecutorService;

/** Trusted factory for opening one validated and scoped filesystem mount. */
public interface IFilesystemFactory {
  @FunctionalInterface
  interface CredentialResolver {
    Object resolve(String reference);
  }

  record OpenContext(
      Executor ioExecutor,
      ScheduledExecutorService scheduler,
      CredentialResolver credentials) {
    public OpenContext {
      ioExecutor = Objects.requireNonNull(ioExecutor, "filesystem I/O executor");
      scheduler = Objects.requireNonNull(scheduler, "filesystem deadline scheduler");
      credentials = Objects.requireNonNull(credentials, "filesystem credential resolver");
    }
  }

  String kind();

  default void validate(Map<String, ?> configuration) {
    Objects.requireNonNull(configuration, "filesystem configuration");
  }

  CompletionStage<IFilesystem> open(OpenContext context, Map<String, ?> configuration);
}
