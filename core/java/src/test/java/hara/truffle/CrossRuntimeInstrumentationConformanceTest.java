package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.truffle.InstrumentationException.Code;
import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentHandle;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.InstrumentationModel.TargetKind;
import java.util.List;
import java.util.Set;
import java.util.TreeSet;
import org.junit.Test;

/**
 * Portable instrumentation conformance corpus — Java provider.
 *
 * <p>Each test case is labeled {@code // CONFORMS: instrum/<case-name>} and has a matching
 * case in {@code core/rust/src/instrumentation/conformance_tests.rs}. Both providers assert the
 * same portable invariants. Documented differences (e.g., backend identifier, Rust-only
 * {@code enabled_events()} surface) are noted per case.
 *
 * <p>This is the first delivery slice of issue #937. The corpus covers:
 * <ol>
 *   <li>{@code instrum/fresh-hub-zero-state}</li>
 *   <li>{@code instrum/registration-order}</li>
 *   <li>{@code instrum/unsupported-capability}</li>
 *   <li>{@code instrum/exclusive-control-lease}</li>
 *   <li>{@code instrum/stale-handle-after-detach}</li>
 *   <li>{@code instrum/session-cleanup}</li>
 *   <li>{@code instrum/zero-attachment-no-events}</li>
 * </ol>
 */
public class CrossRuntimeInstrumentationConformanceTest {

  // CONFORMS: instrum/fresh-hub-zero-state
  // A freshly created hub has no registered instruments, targets, or attachments.
  // Rust additionally verifies enabled_events() is empty; Java exposes count APIs only.
  @Test
  public void freshHubHasZeroRegisteredState() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      assertEquals(0, hub.instrumentCount());
      assertEquals(0, hub.targetCount());
      assertEquals(0, hub.attachmentCount());
    }
  }

  // CONFORMS: instrum/registration-order
  // Instruments registered first are delivered first; registration insertion order is preserved.
  @Test
  public void attachmentsFollowRegistrationOrder() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target = hub.registerTarget(interpreterTarget("t", "s"));
      InstrumentHandle first =
          hub.registerInstrument(passive("first", "s", Set.of(Capability.EVENT_LIFECYCLE)));
      InstrumentHandle second =
          hub.registerInstrument(passive("second", "s", Set.of(Capability.EVENT_LIFECYCLE)));
      hub.attach(first, target);
      hub.attach(second, target);
      List<String> order =
          hub.registrations().stream().map(InstrumentRegistration::instrumentId).toList();
      assertEquals(List.of("first", "second"), order);
    }
  }

  // CONFORMS: instrum/unsupported-capability
  // Requesting an event capability the target does not advertise fails with structured evidence
  // identifying the target, backend, requested capabilities, potential capabilities, and missing set.
  // Rust: InstrumentationError::UnsupportedCapabilities { target_id, backend, missing }
  // Java: InstrumentationException(Code.UNSUPPORTED_CAPABILITIES) with evidence map
  @Test
  public void unsupportedCapabilityProducesExactEvidence() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "execution",
                  "s",
                  TargetKind.INTERPRETER,
                  new RuntimeBackend("java"),
                  Set.of()));
      InstrumentHandle trace =
          hub.registerInstrument(passive("trace", "s", Set.of(Capability.EVENT_LIFECYCLE)));
      InstrumentationException error =
          assertThrows(InstrumentationException.class, () -> hub.attach(trace, target));
      assertEquals(Code.UNSUPPORTED_CAPABILITIES, error.code());
      assertEquals("execution", error.evidence().get("target"));
      @SuppressWarnings("unchecked")
      Set<Capability> missing = (Set<Capability>) error.evidence().get("missing");
      assertFalse(missing.isEmpty());
      assertTrue(missing.contains(Capability.EVENT_LIFECYCLE));
    }
  }

  // CONFORMS: instrum/exclusive-control-lease
  // Only one controller may hold the control lease for a target at a time. A second request while
  // a lease is held fails with a deterministic conflict error identifying the current holder.
  // Rust: InstrumentationError::ControlLeaseHeld { target_id, holder }
  // Java: InstrumentationException(Code.CONTROL_LEASE_CONFLICT) with evidence["holder"]
  @Test
  public void exclusiveControlLeaseConflict() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "execution",
                  "s",
                  TargetKind.HBC,
                  new RuntimeBackend("java"),
                  Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE)));
      InstrumentHandle first =
          hub.registerInstrument(
              control("debugger-a", "s", Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE)));
      InstrumentHandle second =
          hub.registerInstrument(
              control("debugger-b", "s", Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE)));
      hub.attach(first, target);
      hub.attach(second, target);
      hub.acquireControlLease(first, target);
      InstrumentationException conflict =
          assertThrows(
              InstrumentationException.class, () -> hub.acquireControlLease(second, target));
      assertEquals(Code.CONTROL_LEASE_CONFLICT, conflict.code());
      assertEquals("debugger-a", conflict.evidence().get("holder"));
    }
  }

  // CONFORMS: instrum/stale-handle-after-detach
  // After an instrument is removed, the old handle has generation 0. A replacement registered
  // under the same instrument ID gets generation 1. Using the old handle fails with a stale error.
  // Rust: InstrumentationError::StaleInstrumentHandle { instrument_id, generation: 0 }
  // Java: InstrumentationException(Code.STALE_INSTRUMENT)
  @Test
  public void staleHandleAfterIdReuse() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      InstrumentHandle original =
          hub.registerInstrument(passive("trace", "s", Set.of(Capability.EVENT_LIFECYCLE)));
      hub.removeInstrument(original);
      InstrumentHandle replacement =
          hub.registerInstrument(passive("trace", "s", Set.of(Capability.EVENT_LIFECYCLE)));
      assertEquals(0L, original.generation());
      assertEquals(1L, replacement.generation());
      InstrumentationException stale =
          assertThrows(InstrumentationException.class, () -> hub.removeInstrument(original));
      assertEquals(Code.STALE_INSTRUMENT, stale.code());
    }
  }

  // CONFORMS: instrum/session-cleanup
  // Session cleanup removes all instruments, targets, attachments, and leases that belong to
  // the session. All counts reach zero after cleanup.
  // Rust additionally verifies enabled_events() becomes empty; Java verifies counts only.
  @Test
  public void sessionCleanupRemovesAllState() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "execution",
                  "s",
                  TargetKind.HBC,
                  new RuntimeBackend("java"),
                  Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE)));
      InstrumentHandle ctrl =
          hub.registerInstrument(
              control("d", "s", Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE)));
      hub.attach(ctrl, target);
      hub.acquireControlLease(ctrl, target);
      assertEquals(1, hub.instrumentCount());
      assertEquals(1, hub.targetCount());
      assertEquals(1, hub.attachmentCount());
      hub.cleanupSession("s");
      assertEquals(0, hub.instrumentCount());
      assertEquals(0, hub.targetCount());
      assertEquals(0, hub.attachmentCount());
    }
  }

  // CONFORMS: instrum/zero-attachment-no-events
  // Registering an instrument without attaching it to a target produces no event subscriptions
  // for that target. The target reports no subscribers for any event kind.
  // Rust: hub.enabled_for_target(..) returns false; hub.enabled_events() is empty
  // Java: hub.hasSubscribers(target, event) returns false
  @Test
  public void zeroAttachmentProducesNoEvents() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target = hub.registerTarget(interpreterTarget("t", "s"));
      hub.registerInstrument(passive("trace", "s", Set.of(Capability.EVENT_LIFECYCLE)));
      assertFalse(hub.hasSubscribers(target, EventKind.EXECUTION_TERMINAL));
    }
  }

  private static TargetDescriptor interpreterTarget(String id, String session) {
    return new TargetDescriptor(
        id,
        session,
        TargetKind.INTERPRETER,
        new RuntimeBackend("java"),
        Set.of(Capability.EVENT_LIFECYCLE));
  }

  private static InstrumentRegistration passive(
      String id, String session, Set<Capability> capabilities) {
    return new InstrumentRegistration(
        id,
        session,
        InstrumentMode.PASSIVE,
        capabilities,
        new TreeSet<>(Set.of(EventKind.EXECUTION_TERMINAL)),
        new InstrumentFilter(session, Set.of(), Set.of(), Set.of()),
        ProjectionRequest.none(),
        EventDelivery.queue(8));
  }

  private static InstrumentRegistration control(
      String id, String session, Set<Capability> capabilities) {
    return new InstrumentRegistration(
        id,
        session,
        InstrumentMode.CONTROL,
        capabilities,
        new TreeSet<>(Set.of(EventKind.EXECUTION_TERMINAL)),
        new InstrumentFilter(session, Set.of(), Set.of(), Set.of()),
        ProjectionRequest.none(),
        EventDelivery.queue(8));
  }
}
