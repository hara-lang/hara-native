package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import hara.lang.declaration.HaraAvailability;
import hara.work.WorkPlan;
import hara.work.WorkRegistry;
import hara.work.WorkRuntime;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import org.junit.Test;

/** Direct JVM conformance for the portable native Work plan, runtime, registry, and host. */
public class HaraNativeWorkPlanTest {
  @Test
  public void workBindingIsPortableAndDeclaresTheLifecycleSurface() {
    assertEquals(HaraAvailability.PORTABLE, HaraNativeDeclarations.binding("Work").availability());
    assertTrue(
        HaraNativeDeclarations.methods("Work")
            .containsAll(
                List.of(
                    "default-host",
                    "reset-host",
                    "pure",
                    "chain",
                    "encode-hta",
                    "decode-hta",
                    "new-registry",
                    "new-runtime",
                    "submit-plan")));
  }

  @Test
  public void portablePlansRoundTripThroughHtaAndExecuteRegisteredTargets() {
    WorkRegistry registry = new WorkRegistry();
    registry.bind(
        "fixture/increment",
        (input, context) ->
            CompletableFuture.completedFuture((Object) (((Long) input) + 1L)));
    registry.bind(
        "fixture/double",
        (input, context) ->
            CompletableFuture.completedFuture((Object) (((Long) input) * 2L)));

    WorkPlan plan =
        WorkPlan.chain(
            List.of(WorkPlan.pure("fixture/increment"), WorkPlan.step("fixture/double")));
    WorkPlan decoded = WorkPlan.decodeHta(plan.encodeHta());
    WorkRuntime runtime = new WorkRuntime(registry);

    assertEquals(WorkPlan.Operation.CHAIN, decoded.operation());
    assertTrue(Arrays.equals(plan.encodeHta(), decoded.encodeHta()));
    assertEquals(8L, runtime.evaluate(decoded, 3L).toCompletableFuture().join());
  }

  @Test
  public void registryRuntimeAndHostResetRestoreTheSameEmptyAcceptingBaseline() {
    WorkRegistry registry = new WorkRegistry();
    WorkRuntime runtime = new WorkRuntime(registry);
    registry.bind(
        "fixture/value",
        (input, context) -> CompletableFuture.completedFuture(input));

    assertEquals(List.of("fixture/value"), registry.targetNames());
    runtime.reset();
    runtime.reset();
    assertEquals(List.of(), registry.targetNames());

    HaraWorkHost host = HaraWorkHost.instance();
    try {
      host.stop();
      assertTrue(host.isStopped());
      assertSame(host, host.reset());
      assertTrue(host.isStarted());
      assertSame(host, host.reset());
      assertTrue(host.isStarted());
    } finally {
      host.reset();
    }
  }
}
