package hara.truffle;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import hara.truffle.InstrumentationModel.EventKind;
import org.junit.Test;

/** Verifies ownership and reset behavior for context-scoped instrumentation state. */
public class HaraInstrumentationRuntimeTest {
  @Test
  public void interpreterRootDepthTracksOutermostExecutionAndResetsOnClose() {
    HaraInstrumentationRuntime runtime = new HaraInstrumentationRuntime(null, null);
    assertTrue(runtime.enterInterpreterRoot());
    assertFalse(runtime.enterInterpreterRoot());
    runtime.exitInterpreterRoot();
    assertFalse(runtime.enterInterpreterRoot());
    runtime.exitInterpreterRoot();
    runtime.exitInterpreterRoot();
    assertTrue(runtime.enterInterpreterRoot());
    runtime.close();
    assertTrue(runtime.enterInterpreterRoot());
    runtime.exitInterpreterRoot();
  }

  @Test
  public void unavailableSessionDisablesInstrumentationWithoutThrowing() {
    HaraInstrumentationRuntime runtime = new HaraInstrumentationRuntime(null, null);
    assertFalse(runtime.hbcInstrumentationEnabled(EventKind.MACHINE_RESUME));
    assertNull(runtime.pollHbcDirective());
    runtime.publishInterpreterTerminal(null, "return");
    runtime.publishHbcEvent(EventKind.MACHINE_RESUME, 0, "main", "source", java.util.Map.of());
    runtime.close();
  }
}
