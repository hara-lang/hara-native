package hara.truffle;

import hara.lang.protocol.IStream;
import java.util.ArrayDeque;
import java.util.concurrent.CompletableFuture;

/** Bounded asynchronous byte stream fed by one host I/O drainer. */
final class HaraByteStream implements IStream {
  private final HaraContext context;
  private final Runnable closeAction;
  private final ArrayDeque<byte[]> chunks = new ArrayDeque<>();
  private CompletableFuture<Object> waiting;
  private int queuedBytes;
  private boolean closed;

  HaraByteStream(HaraContext context, Runnable closeAction) {
    this.context = context;
    this.closeAction = closeAction;
  }

  synchronized void publish(byte[] bytes) {
    if (closed || bytes.length == 0) return;
    if (waiting != null) {
      CompletableFuture<Object> target = waiting;
      waiting = null;
      target.complete(bytes);
      return;
    }
    if (chunks.size() >= 256 || queuedBytes + bytes.length > 1_048_576) {
      fail(new HaraException("stream/overflow: byte stream exceeded its bounded buffer"));
      return;
    }
    chunks.addLast(bytes);
    queuedBytes += bytes.length;
  }

  synchronized void finish() {
    if (closed) return;
    closed = true;
    if (waiting != null) {
      waiting.complete(null);
      waiting = null;
    }
  }

  synchronized void fail(Throwable error) {
    if (closed) return;
    closed = true;
    if (waiting != null) {
      waiting.completeExceptionally(error);
      waiting = null;
    }
    closeAction.run();
  }

  @Override
  public synchronized Object next() {
    if (!chunks.isEmpty()) {
      byte[] bytes = chunks.removeFirst();
      queuedBytes -= bytes.length;
      return context.completedPromise(bytes);
    }
    if (closed) return context.completedPromise(null);
    if (waiting != null) {
      return context.rejectedPromise("stream/pending-pull: only one Stream/next may be pending");
    }
    waiting = new CompletableFuture<>();
    return context.promiseValue(waiting);
  }

  @Override
  public synchronized void close() {
    if (closed) return;
    closed = true;
    chunks.clear();
    queuedBytes = 0;
    if (waiting != null) {
      waiting.complete(null);
      waiting = null;
    }
    closeAction.run();
  }

  @Override
  public synchronized String toString() {
    return "#<byte-stream " + (closed ? "closed" : "open") + ">";
  }
}
