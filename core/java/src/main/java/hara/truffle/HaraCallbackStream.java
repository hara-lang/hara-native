package hara.truffle;

import hara.lang.protocol.IStream;
import hara.lang.protocol.IPromise;

/** Native Stream backed by guest next and close callbacks. */
final class HaraCallbackStream implements IStream {
  private final HaraContext context;
  private final Object next;
  private final Object close;
  private boolean pending;
  private boolean closed;
  private IPromise pendingPromise;

  HaraCallbackStream(HaraContext context, Object next, Object close) {
    this.context = context;
    this.next = next;
    this.close = close;
  }

  @Override
  public synchronized Object next() {
    if (closed) return context.completedPromise(null);
    if (pending) return context.rejectedPromise("stream/pending-pull: only one Stream/next may be pending");
    pending = true;
    try {
      Object source = context.invokeCallable(next, new Object[0]);
      Object promise = context.callbackStreamPromise(
          source,
          () -> {
            synchronized (HaraCallbackStream.this) {
              pending = false;
              pendingPromise = null;
            }
          });
      pendingPromise = (IPromise) promise;
      return promise;
    } catch (Throwable error) {
      pending = false;
      return context.rejectedPromise(error.getMessage());
    }
  }

  @Override
  public synchronized void close() {
    if (closed) return;
    closed = true;
    if (pendingPromise != null) pendingPromise.cancel();
    if (close != null) context.invokeCallable(close, new Object[0]);
  }
}
