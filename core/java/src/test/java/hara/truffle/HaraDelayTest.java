package hara.truffle;

import static org.junit.Assert.*;

import hara.lang.base.primitive.Delay;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HaraDelayTest {
  @Test
  public void concurrentRealizationInvokesTheSupplierOnce() throws Exception {
    AtomicInteger calls = new AtomicInteger();
    Object expected = new Object();
    Delay<Object> delay = new Delay<>(() -> { calls.incrementAndGet(); return expected; });
    var pool = java.util.concurrent.Executors.newFixedThreadPool(4);
    try {
      var tasks = new java.util.ArrayList<java.util.concurrent.Callable<Object>>();
      for (int i = 0; i < 16; i++) tasks.add(delay::deref);
      for (var result : pool.invokeAll(tasks)) assertSame(expected, result.get());
      assertEquals(1, calls.get());
      assertTrue(delay.isRealized());
    } finally {
      pool.shutdownNow();
      assertTrue(pool.awaitTermination(5, java.util.concurrent.TimeUnit.SECONDS));
    }
  }

  @Test
  public void primitiveCachesNullFalseAndFailures() {
    for (Object expected : new Object[] {null, false, 42L}) {
      AtomicInteger calls = new AtomicInteger();
      Delay<Object> delay = new Delay<>(() -> { calls.incrementAndGet(); return expected; });
      assertFalse(delay.isRealized());
      assertEquals(0, calls.get());
      assertSame(expected, delay.deref());
      assertSame(expected, delay.realize());
      assertTrue(delay.isRealized());
      assertEquals(1, calls.get());
    }
    AtomicInteger calls = new AtomicInteger();
    RuntimeException failure = new RuntimeException("original failure");
    Delay<Object> delay = new Delay<>(() -> { calls.incrementAndGet(); throw failure; });
    for (int attempt = 0; attempt < 2; attempt++) {
      try { delay.deref(); fail("failure must be rethrown"); }
      catch (RuntimeException error) { assertSame(failure, error); }
    }
    assertEquals(1, calls.get());
    assertTrue(delay.isRealized());
  }

  @Test
  public void recursiveRealizationCachesFailure() {
    AtomicReference<Delay<Object>> slot = new AtomicReference<>();
    Delay<Object> delay = new Delay<>(() -> slot.get().deref());
    slot.set(delay);
    try {
      for (int attempt = 0; attempt < 2; attempt++) {
        try { delay.deref(); fail("recursive realization must fail"); }
        catch (IllegalStateException error) {
          assertEquals("delay realization is recursive", error.getMessage());
        }
      }
      assertTrue(delay.isRealized());
    } finally { slot.set(null); }
  }

  @Test
  public void languageConstructorUsesDerefAndRealizeProtocols() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(context.eval(HaraLanguage.ID,
          "(let [calls (Base/atom 0)"
          + "      lazy (Base/delay (fn [] (IReset/reset calls (+ 1 (IDeref/deref calls))) false))"
          + "      before [(IDeref/deref calls) (IRealize/realized? lazy)]]"
          + " (= [before (IDeref/deref lazy) (IRealize/realize lazy)"
          + "     (IDeref/deref calls) (IRealize/realized? lazy) (Base/type lazy)]"
          + "    [[0 false] false false 1 true :std.native.Delay]))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID,
          "(let [calls (Base/atom 0)"
          + "      lazy (Base/delay (fn [] (IReset/reset calls (+ 1 (IDeref/deref calls)))"
          + "                             (throw \"delay boom\")))"
          + "      first (try (IDeref/deref lazy) (catch e e))"
          + "      second (try (IRealize/realize lazy) (catch e e))]"
          + " (= [(= first second) (IDeref/deref calls) (IRealize/realized? lazy)] [true 1 true]))")
          .asBoolean());
    }
  }
}
