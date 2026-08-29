package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;

public class FilesystemPromiseBridgeTest {
  @Test
  public void bridgeUsesTheExistingPromiseFactoryAndTransformsProviderResults() {
    CompletableFuture<String> provider = new CompletableFuture<>();
    AtomicInteger cancellations = new AtomicInteger();
    FilesystemRuntimeBinding.Pending<String> pending =
        new FilesystemRuntimeBinding.Pending<>(
            provider,
            () -> {
              cancellations.incrementAndGet();
              return true;
            });
    AtomicReference<CompletableFuture<Object>> observedFuture = new AtomicReference<>();
    AtomicReference<Runnable> observedCancellation = new AtomicReference<>();
    Object promiseToken = new Object();

    Object returned =
        FilesystemPromiseBridge.bind(
            (future, cancellation) -> {
              observedFuture.set(future);
              observedCancellation.set(cancellation);
              return promiseToken;
            },
            pending,
            String::toUpperCase);

    assertSame(promiseToken, returned);
    provider.complete("value");
    assertEquals("VALUE", observedFuture.get().join());
    observedCancellation.get().run();
    assertEquals(1, cancellations.get());
  }

  @Test
  public void transformFailuresRemainExceptionalPromiseResults() {
    CompletableFuture<String> provider = CompletableFuture.completedFuture("value");
    FilesystemRuntimeBinding.Pending<String> pending =
        new FilesystemRuntimeBinding.Pending<>(provider, () -> false);
    AtomicReference<CompletableFuture<Object>> observed = new AtomicReference<>();

    FilesystemPromiseBridge.bind(
        (future, cancellation) -> {
          observed.set(future);
          return new Object();
        },
        pending,
        ignored -> {
          throw new IllegalArgumentException("result conversion failed");
        });

    assertTrue(observed.get().isCompletedExceptionally());
    java.util.concurrent.CompletionException error =
        org.junit.Assert.assertThrows(
            java.util.concurrent.CompletionException.class,
            () -> observed.get().join());
    assertTrue(error.getCause() instanceof IllegalArgumentException);
  }
}
