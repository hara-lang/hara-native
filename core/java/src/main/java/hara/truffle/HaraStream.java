package hara.truffle;

import hara.lang.protocol.IStream;
import java.util.concurrent.CompletableFuture;

/** Lazy stream backed by one private coroutine. */
final class HaraStream implements IStream {
  private final HaraContext context;
  private final StdFoundationCoroutine.HaraCoroutine coroutine;
  private final Object[] initialArguments;
  private boolean started;
  private boolean pending;
  private boolean closed;

  HaraStream(HaraContext context, Object function, Object[] initialArguments) {
    this.context = context;
    this.coroutine = new StdFoundationCoroutine.HaraCoroutine(context, function);
    this.initialArguments = initialArguments.clone();
  }

  @Override public synchronized Object next() {
    if (closed) return context.completedPromise(null);
    if (pending) return context.rejectedPromise("stream/pending-pull: only one Stream/next may be pending");
    pending = true;
    CompletableFuture<Object> result = new CompletableFuture<>();
    // A guest pull advances the coroutine to its next yield boundary before
    // returning the promise.  When the callback settles immediately, as it
    // does for a portable `Promise/from`, the public promise is settled too.
    pull(result);
    return context.promiseValue(result);
  }

  private void pull(CompletableFuture<Object> result) {
    try {
      Object value;
      synchronized (this) {
        if (closed) { pending = false; result.complete(null); return; }
        value = started ? coroutine.resume() : coroutine.resume(initialArguments);
        started = true;
        if (coroutine.status() == StdFoundationCoroutine.STATUS_DEAD) {
          closed = true; pending = false; result.complete(null); return;
        }
        if (value == null) {
          closed = true; pending = false; coroutine.closeCoroutine();
          result.completeExceptionally(new HaraException("stream/nil-item: a stream coroutine may not yield nil"));
          return;
        }
        pending = false;
      }
      result.complete(value);
    } catch (Throwable error) {
      synchronized (this) { pending = false; closed = true; }
      result.completeExceptionally(error);
    }
  }

  @Override public synchronized void close() {
    if (closed) return;
    closed = true;
    if (!pending) coroutine.closeCoroutine();
  }

  @Override public synchronized String toString() {
    return "#<stream " + (closed ? "closed" : pending ? "pending" : "ready") + ">";
  }
}
