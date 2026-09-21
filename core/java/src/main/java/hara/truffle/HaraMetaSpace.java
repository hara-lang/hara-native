package hara.truffle;

import com.oracle.truffle.api.RootCallTarget;
import hara.truffle.bytecode.HbcProgram;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Supplier;

/**
 * Context-owned storage for linked HBC call targets.
 *
 * <p>The metaspace contains only runtime link state. It does not publish guest values or namespace
 * bindings, and its entries are discarded with the owning {@link HaraContext}.
 */
public final class HaraMetaSpace implements AutoCloseable {
  private final ConcurrentHashMap<LinkKey, CompletableFuture<RootCallTarget>> links =
      new ConcurrentHashMap<>();
  private final Object lifecycleLock = new Object();
  private final AtomicBoolean closed = new AtomicBoolean();

  /**
   * Returns the linked target for a program function, waiting for an in-flight link when necessary.
   */
  public RootCallTarget get(HbcProgram program, int functionIndex) {
    ensureOpen();
    LinkKey key = key(program, functionIndex);
    CompletableFuture<RootCallTarget> link = links.get(key);
    return link == null ? null : await(link);
  }

  /**
   * Links one program function once and shares the result with concurrent callers.
   *
   * <p>A {@code null} compiler result is not retained: it means that the function is not eligible
   * for the generated tier and may become linkable after the runtime execution policy changes.
   */
  public RootCallTarget link(
      HbcProgram program, int functionIndex, Supplier<? extends RootCallTarget> compiler) {
    LinkKey key = key(program, functionIndex);
    Objects.requireNonNull(compiler, "compiler");
    CompletableFuture<RootCallTarget> created = new CompletableFuture<>();
    CompletableFuture<RootCallTarget> link;
    synchronized (lifecycleLock) {
      ensureOpen();
      link = links.putIfAbsent(key, created);
    }
    if (link != null) return await(link);
    try {
      RootCallTarget target = compiler.get();
      created.complete(target);
      if (target == null) links.remove(key, created);
      return target;
    } catch (RuntimeException | Error failure) {
      created.completeExceptionally(failure);
      links.remove(key, created);
      throw failure;
    }
  }

  /** Number of currently linked program functions. */
  public int size() {
    return links.size();
  }

  /** Discards all linked targets owned by this metaspace. */
  public void clear() {
    links.clear();
  }

  @Override
  public void close() {
    synchronized (lifecycleLock) {
      if (closed.compareAndSet(false, true)) links.clear();
    }
  }

  private void ensureOpen() {
    if (closed.get()) throw new IllegalStateException("metaspace is closed");
  }

  private static LinkKey key(HbcProgram program, int functionIndex) {
    Objects.requireNonNull(program, "program");
    if (functionIndex < 0) {
      throw new IllegalArgumentException("function index must not be negative");
    }
    return new LinkKey(program, functionIndex);
  }

  private static RootCallTarget await(CompletableFuture<RootCallTarget> link) {
    try {
      return link.join();
    } catch (CompletionException failure) {
      Throwable cause = failure.getCause();
      if (cause instanceof RuntimeException runtime) throw runtime;
      if (cause instanceof Error error) throw error;
      throw failure;
    }
  }

  private static final class LinkKey {
    private final HbcProgram program;
    private final int functionIndex;
    private final int hash;

    private LinkKey(HbcProgram program, int functionIndex) {
      this.program = program;
      this.functionIndex = functionIndex;
      this.hash = 31 * System.identityHashCode(program) + functionIndex;
    }

    @Override
    public boolean equals(Object other) {
      return other instanceof LinkKey key
          && program == key.program
          && functionIndex == key.functionIndex;
    }

    @Override
    public int hashCode() {
      return hash;
    }
  }
}
