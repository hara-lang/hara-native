package hara.truffle;

import hara.truffle.InstrumentationException.Code;
import hara.truffle.InstrumentationModel.ControlLease;
import hara.truffle.InstrumentationModel.EventBatch;
import hara.truffle.InstrumentationModel.InstrumentDirective;
import hara.truffle.InstrumentationModel.InstrumentHandle;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import java.lang.ref.WeakReference;
import java.util.Map;
import java.util.Objects;

/** Session-scoped trusted host facade over the Java Runtime-owned hub. */
final class NativeInstrumentation {
  static final class NativeInstrumentHandle {
    private final String runtimeId;
    private final String sessionId;
    private final InstrumentHandle handle;

    private NativeInstrumentHandle(
        String runtimeId, String sessionId, InstrumentHandle handle) {
      this.runtimeId = runtimeId;
      this.sessionId = sessionId;
      this.handle = handle;
    }

    String instrumentId() {
      return handle.instrumentId();
    }

    long generation() {
      return handle.generation();
    }
  }

  static final class NativeTargetHandle {
    private final String runtimeId;
    private final String sessionId;
    private final TargetHandle handle;

    private NativeTargetHandle(
        String runtimeId, String sessionId, TargetHandle handle) {
      this.runtimeId = runtimeId;
      this.sessionId = sessionId;
      this.handle = handle;
    }

    String targetId() {
      return handle.targetId();
    }

    long generation() {
      return handle.generation();
    }
  }

  static final class NativeAttachment {
    private final NativeInstrumentHandle instrument;
    private final NativeTargetHandle target;

    private NativeAttachment(
        NativeInstrumentHandle instrument, NativeTargetHandle target) {
      this.instrument = instrument;
      this.target = target;
    }

    String instrumentId() {
      return instrument.instrumentId();
    }

    String targetId() {
      return target.targetId();
    }
  }

  static final class NativeControlLease {
    private final String runtimeId;
    private final String sessionId;
    private final ControlLease lease;

    private NativeControlLease(
        String runtimeId, String sessionId, ControlLease lease) {
      this.runtimeId = runtimeId;
      this.sessionId = sessionId;
      this.lease = lease;
    }

    String instrumentId() {
      return lease.instrument().instrumentId();
    }

    String targetId() {
      return lease.target().targetId();
    }

    long generation() {
      return lease.generation();
    }
  }

  private final WeakReference<SessionKernel> kernel;
  private final WeakReference<SessionKernel.Session> session;
  private final WeakReference<InstrumentationHub> hub;
  private final String runtimeId;
  private final String sessionId;

  NativeInstrumentation(
      SessionKernel kernel, SessionKernel.Session session, InstrumentationHub hub) {
    this.kernel = new WeakReference<>(Objects.requireNonNull(kernel, "kernel"));
    this.session = new WeakReference<>(Objects.requireNonNull(session, "session"));
    this.hub = new WeakReference<>(Objects.requireNonNull(hub, "hub"));
    this.runtimeId = hub.runtimeId();
    this.sessionId = session.id().value();
  }

  String sessionId() {
    return sessionId;
  }

  NativeInstrumentHandle register(InstrumentRegistration registration) {
    InstrumentationHub current = requireActive();
    Objects.requireNonNull(registration, "registration");
    requireSession(registration.sessionId(), "instrument registration");
    if (registration.filter().sessionId() != null) {
      requireSession(registration.filter().sessionId(), "instrument filter");
    }
    return instrument(current.registerInstrument(registration));
  }

  void unregister(NativeInstrumentHandle instrument) {
    InstrumentationHub current = requireActive();
    current.removeInstrument(requireInstrument(instrument));
    kernel.get().refreshTruffleInstrumentation(sessionId);
  }

  NativeTargetHandle bindTargetIdentity(String targetId, long generation) {
    InstrumentationHub current = requireActive();
    TargetHandle target = current.bindTarget(targetId, generation);
    TargetDescriptor descriptor = current.descriptor(target);
    requireSession(descriptor.sessionId(), "target");
    return target(target);
  }

  TargetDescriptor targetDescriptor(NativeTargetHandle target) {
    InstrumentationHub current = requireActive();
    TargetDescriptor descriptor = current.descriptor(requireTarget(target));
    requireSession(descriptor.sessionId(), "target");
    return descriptor;
  }

  NativeAttachment attach(
      NativeInstrumentHandle instrument, NativeTargetHandle target) {
    InstrumentationHub current = requireActive();
    InstrumentHandle checkedInstrument = requireInstrument(instrument);
    TargetHandle checkedTarget = requireTarget(target);
    current.attach(checkedInstrument, checkedTarget);
    if (current.descriptor(checkedTarget).kind()
        == InstrumentationModel.TargetKind.INTERPRETER) {
      kernel.get().refreshTruffleInstrumentation(sessionId);
    }
    return new NativeAttachment(instrument, target);
  }

  void detach(NativeAttachment attachment) {
    InstrumentationHub current = requireActive();
    Objects.requireNonNull(attachment, "attachment");
    current.detach(
        requireInstrument(attachment.instrument),
        requireTarget(attachment.target));
    if (current.descriptor(attachment.target.handle).kind()
        == InstrumentationModel.TargetKind.INTERPRETER) {
      kernel.get().refreshTruffleInstrumentation(sessionId);
    } else if (!current.hasAttachments(attachment.target.handle)) {
      kernel.get().clearHbcExecution(sessionId);
    }
  }

  EventBatch drainEvents(NativeInstrumentHandle instrument) {
    InstrumentationHub current = requireActive();
    return current.drain(requireInstrument(instrument));
  }

  NativeControlLease acquireControlLease(
      NativeInstrumentHandle instrument, NativeTargetHandle target) {
    InstrumentationHub current = requireActive();
    ControlLease lease =
        current.acquireControlLease(
            requireInstrument(instrument), requireTarget(target));
    return new NativeControlLease(runtimeId, sessionId, lease);
  }

  void releaseControlLease(NativeControlLease lease) {
    InstrumentationHub current = requireActive();
    ControlLease checkedLease = requireLease(lease);
    current.releaseControlLease(checkedLease);
    if (current.descriptor(checkedLease.target()).kind()
        == InstrumentationModel.TargetKind.HBC) {
      kernel.get().clearHbcExecution(sessionId);
    }
  }

  void issueDirective(
      NativeControlLease lease, InstrumentDirective directive) {
    InstrumentationHub current = requireActive();
    current.issueDirective(requireLease(lease), directive);
  }

  private InstrumentationHub requireActive() {
    InstrumentationHub currentHub = hub.get();
    SessionKernel currentKernel = kernel.get();
    SessionKernel.Session boundSession = session.get();
    if (currentHub == null || currentHub.isClosed()) {
      throw new InstrumentationException(
          Code.RUNTIME_CLOSED, "INSTRUMENTATION_RUNTIME_CLOSED");
    }
    if (currentKernel == null || boundSession == null) {
      throw sessionClosed();
    }
    SessionKernel.Session currentSession;
    try {
      currentSession = currentKernel.require(boundSession.id());
    } catch (IllegalArgumentException error) {
      throw sessionClosed();
    }
    if (currentSession != boundSession
        || currentSession.state() != SessionModel.SessionState.ACTIVE) {
      throw sessionClosed();
    }
    return currentHub;
  }

  private InstrumentHandle requireInstrument(
      NativeInstrumentHandle nativeHandle) {
    Objects.requireNonNull(nativeHandle, "instrument handle");
    requireAuthority(
        nativeHandle.runtimeId, nativeHandle.sessionId, "instrument");
    return nativeHandle.handle;
  }

  private TargetHandle requireTarget(NativeTargetHandle nativeHandle) {
    Objects.requireNonNull(nativeHandle, "target handle");
    requireAuthority(nativeHandle.runtimeId, nativeHandle.sessionId, "target");
    return nativeHandle.handle;
  }

  private ControlLease requireLease(NativeControlLease nativeLease) {
    Objects.requireNonNull(nativeLease, "control lease");
    requireAuthority(
        nativeLease.runtimeId, nativeLease.sessionId, "control lease");
    return nativeLease.lease;
  }

  private void requireAuthority(
      String handleRuntime, String handleSession, String label) {
    if (!runtimeId.equals(handleRuntime)) {
      throw new InstrumentationException(
          Code.CROSS_RUNTIME,
          "CROSS_RUNTIME_INSTRUMENTATION_HANDLE " + label,
          Map.of(
              "expectedRuntime", runtimeId, "handleRuntime", handleRuntime));
    }
    requireSession(handleSession, label);
  }

  private void requireSession(String handleSession, String label) {
    if (!sessionId.equals(handleSession)) {
      throw new InstrumentationException(
          Code.CROSS_SESSION,
          "CROSS_SESSION_INSTRUMENTATION_HANDLE " + label,
          Map.of(
              "expectedSession", sessionId, "handleSession", handleSession));
    }
  }

  private NativeInstrumentHandle instrument(InstrumentHandle handle) {
    return new NativeInstrumentHandle(runtimeId, sessionId, handle);
  }

  private NativeTargetHandle target(TargetHandle handle) {
    return new NativeTargetHandle(runtimeId, sessionId, handle);
  }

  private InstrumentationException sessionClosed() {
    return new InstrumentationException(
        Code.SESSION_CLOSED,
        "INSTRUMENTATION_SESSION_CLOSED " + sessionId,
        Map.of("session", sessionId));
  }
}
