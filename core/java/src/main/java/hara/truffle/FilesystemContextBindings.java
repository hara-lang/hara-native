package hara.truffle;

import java.nio.file.Path;
import java.util.Objects;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.atomic.AtomicLong;

/**
 * One-use trusted handoff of an exact Session filesystem binding into a Truffle context.
 *
 * <p>The token is an embedding implementation detail. It is never a mount identifier, descriptor,
 * provider credential, or guest value. A context claims the binding exactly once; an uninitialized
 * context discards its token when the Session replaces or closes it.
 */
final class FilesystemContextBindings {
  private static final ConcurrentHashMap<String, FilesystemRuntimeBinding> PENDING =
      new ConcurrentHashMap<>();
  private static final ExecutorService FILESYSTEM_IO =
      Executors.newCachedThreadPool(daemonThreadFactory("hara-context-filesystem-io-"));
  private static final ScheduledExecutorService FILESYSTEM_SCHEDULER =
      Executors.newSingleThreadScheduledExecutor(
          daemonThreadFactory("hara-context-filesystem-deadline-"));

  private FilesystemContextBindings() {}

  static String publish(FilesystemRuntimeBinding binding) {
    Objects.requireNonNull(binding, "filesystem runtime binding");
    while (true) {
      String token = UUID.randomUUID().toString();
      if (PENDING.putIfAbsent(token, binding) == null) return token;
    }
  }

  static FilesystemRuntimeBinding claim(String token) {
    if (token == null || token.isBlank()) return null;
    FilesystemRuntimeBinding binding = PENDING.remove(token);
    if (binding == null) {
      throw new IllegalArgumentException("FILESYSTEM_CONTEXT_BINDING_UNAVAILABLE");
    }
    return binding;
  }

  static void discard(String token) {
    if (token != null && !token.isBlank()) PENDING.remove(token);
  }

  static NativeFilesystem nativeFilesystem(Path root) {
    return new NativeFilesystem(
        new HaraFileProvider(Objects.requireNonNull(root, "filesystem root")),
        "native context",
        FILESYSTEM_IO,
        FILESYSTEM_SCHEDULER);
  }

  static int pendingCount() {
    return PENDING.size();
  }

  private static ThreadFactory daemonThreadFactory(String prefix) {
    AtomicLong sequence = new AtomicLong(1);
    return task -> {
      Thread thread = new Thread(task, prefix + sequence.getAndIncrement());
      thread.setDaemon(true);
      return thread;
    };
  }
}
