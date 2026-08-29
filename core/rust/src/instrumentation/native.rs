use std::cell::RefCell;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::rc::{Rc, Weak};

use super::{
    Capability, ControlLease, EventBatch, EventDelivery, InstrumentDirective, InstrumentHandle,
    InstrumentMode, InstrumentRegistration, InstrumentationAttachment, InstrumentationError,
    InstrumentationHub, RuntimeBackend, TargetDescriptor, TargetHandle,
};

/// Embedding-only access to the instrumentation hub owned by one live Runtime.
///
/// The service retains only a weak Runtime identity. It is never a Hara value,
/// is not installed in a namespace, and cannot keep a Runtime or Session alive.
#[derive(Clone)]
pub struct NativeInstrumentation {
    session_id: String,
    hub: Weak<RefCell<InstrumentationHub>>,
}

/// Opaque, generation-fenced identity for one trusted instrument registration.
#[derive(Clone)]
pub struct NativeInstrumentHandle {
    session_id: String,
    hub: Weak<RefCell<InstrumentationHub>>,
    handle: InstrumentHandle,
}

/// Opaque, generation-fenced identity for one authoritative execution target.
#[derive(Clone)]
pub struct NativeTargetHandle {
    session_id: String,
    hub: Weak<RefCell<InstrumentationHub>>,
    handle: TargetHandle,
}

/// Opaque proof that one trusted controller owns one target's exclusive lease.
#[derive(Clone)]
pub struct NativeControlLease {
    session_id: String,
    hub: Weak<RefCell<InstrumentationHub>>,
    lease: ControlLease,
}

/// Bounded metadata about one successful native attachment. The native handles
/// remain opaque and cannot be converted into a Hara value.
#[derive(Clone)]
pub struct NativeAttachment {
    instrument: NativeInstrumentHandle,
    target: NativeTargetHandle,
    granted_capabilities: BTreeSet<Capability>,
    registration_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeInstrumentationError {
    RuntimeClosed {
        session_id: String,
    },
    UnknownSession(String),
    SessionClosed(String),
    CrossRuntimeHandle {
        kind: &'static str,
    },
    CrossSessionHandle {
        kind: &'static str,
        expected: String,
        actual: String,
    },
    UnsupportedMode(InstrumentMode),
    UnsupportedDelivery(&'static str),
    UnsupportedCapabilities {
        target_id: String,
        backend: RuntimeBackend,
        requested: BTreeSet<Capability>,
        potential: BTreeSet<Capability>,
        missing: BTreeSet<Capability>,
    },
    Hub(InstrumentationError),
}

impl fmt::Display for NativeInstrumentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeClosed { session_id } => write!(
                formatter,
                "instrumentation/native-runtime-closed: session {session_id}"
            ),
            Self::UnknownSession(session_id) => {
                write!(formatter, "instrumentation/native-unknown-session: {session_id}")
            }
            Self::SessionClosed(session_id) => {
                write!(formatter, "instrumentation/native-session-closed: {session_id}")
            }
            Self::CrossRuntimeHandle { kind } => {
                write!(formatter, "instrumentation/native-cross-runtime-handle: {kind}")
            }
            Self::CrossSessionHandle {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "instrumentation/native-cross-session-handle: {kind}, expected {expected}, actual {actual}"
            ),
            Self::UnsupportedMode(mode) => write!(
                formatter,
                "instrumentation/native-unsupported-mode: {mode:?}"
            ),
            Self::UnsupportedDelivery(delivery) => write!(
                formatter,
                "instrumentation/native-unsupported-delivery: {delivery}"
            ),
            Self::UnsupportedCapabilities {
                target_id,
                backend,
                requested,
                potential,
                missing,
            } => write!(
                formatter,
                "instrumentation/native-unsupported-capabilities: target {target_id}, backend {}, requested {requested:?}, potential {potential:?}, missing {missing:?}",
                backend.as_str()
            ),
            Self::Hub(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl Error for NativeInstrumentationError {}

impl From<InstrumentationError> for NativeInstrumentationError {
    fn from(error: InstrumentationError) -> Self {
        Self::Hub(error)
    }
}

impl fmt::Debug for NativeInstrumentation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeInstrumentation")
            .field("session_id", &self.session_id)
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NativeInstrumentHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeInstrumentHandle")
            .field("session_id", &self.session_id)
            .field("instrument_id", &self.handle.instrument_id())
            .field("generation", &self.handle.generation())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NativeTargetHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTargetHandle")
            .field("session_id", &self.session_id)
            .field("target_id", &self.handle.target_id())
            .field("generation", &self.handle.generation())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NativeControlLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeControlLease")
            .field("session_id", &self.session_id)
            .field("instrument_id", &self.lease.instrument().instrument_id())
            .field("target_id", &self.lease.target().target_id())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NativeAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeAttachment")
            .field("instrument", &self.instrument)
            .field("target", &self.target)
            .field("granted_capabilities", &self.granted_capabilities)
            .field("registration_order", &self.registration_order)
            .finish()
    }
}

impl NativeInstrumentation {
    pub(crate) fn new(session_id: impl Into<String>, hub: Rc<RefCell<InstrumentationHub>>) -> Self {
        Self {
            session_id: session_id.into(),
            hub: Rc::downgrade(&hub),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn is_active(&self) -> bool {
        self.hub.upgrade().is_some()
    }

    /// Registers one trusted queued instrument. Transform mode and callback
    /// binding are intentionally rejected until their later lifecycle seams are
    /// implemented; they never degrade silently to another mode or delivery.
    pub fn register(
        &self,
        registration: InstrumentRegistration,
    ) -> Result<NativeInstrumentHandle, NativeInstrumentationError> {
        if registration.session_id != self.session_id {
            return Err(self.cross_session("registration", &registration.session_id));
        }
        if registration.mode == InstrumentMode::Transform {
            return Err(NativeInstrumentationError::UnsupportedMode(
                InstrumentMode::Transform,
            ));
        }
        if registration.delivery == EventDelivery::Callback {
            return Err(NativeInstrumentationError::UnsupportedDelivery(
                "callback binding is not available in this tranche",
            ));
        }
        let hub = self.hub()?;
        let handle = hub.borrow_mut().register(registration)?;
        Ok(self.instrument_handle(handle))
    }

    /// Fences a target identity received from a trusted target-producing host
    /// seam. Target implementation objects remain private to Runtime/Session.
    pub fn bind_target(
        &self,
        target: &TargetHandle,
    ) -> Result<NativeTargetHandle, NativeInstrumentationError> {
        let hub = self.hub()?;
        let descriptor = {
            let hub = hub.borrow();
            hub.target_descriptor(target)?.clone()
        };
        if descriptor.session_id != self.session_id {
            return Err(self.cross_session("target", &descriptor.session_id));
        }
        Ok(self.target_handle(target.clone()))
    }

    pub fn registration(
        &self,
        instrument: &NativeInstrumentHandle,
    ) -> Result<InstrumentRegistration, NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        let hub = self.hub()?;
        let registration = {
            let hub = hub.borrow();
            hub.instrument_registration(&instrument.handle)?.clone()
        };
        Ok(registration)
    }

    pub fn target_descriptor(
        &self,
        target: &NativeTargetHandle,
    ) -> Result<TargetDescriptor, NativeInstrumentationError> {
        self.ensure_target(target)?;
        let hub = self.hub()?;
        let descriptor = {
            let hub = hub.borrow();
            hub.target_descriptor(&target.handle)?.clone()
        };
        Ok(descriptor)
    }

    pub fn attach(
        &self,
        instrument: &NativeInstrumentHandle,
        target: &NativeTargetHandle,
    ) -> Result<NativeAttachment, NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        self.ensure_target(target)?;
        let hub = self.hub()?;
        let (requested, potential) = {
            let hub = hub.borrow();
            (
                hub.instrument_registration(&instrument.handle)?
                    .capabilities
                    .clone(),
                hub.target_descriptor(&target.handle)?.capabilities.clone(),
            )
        };
        let attachment = match hub.borrow_mut().attach(&instrument.handle, &target.handle) {
            Ok(attachment) => attachment,
            Err(InstrumentationError::UnsupportedCapabilities {
                target_id,
                backend,
                missing,
            }) => {
                return Err(NativeInstrumentationError::UnsupportedCapabilities {
                    target_id,
                    backend,
                    requested,
                    potential,
                    missing,
                });
            }
            Err(error) => return Err(error.into()),
        };
        Ok(self.attachment(attachment))
    }

    pub fn granted_capabilities(
        &self,
        instrument: &NativeInstrumentHandle,
        target: &NativeTargetHandle,
    ) -> Result<BTreeSet<Capability>, NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        self.ensure_target(target)?;
        let hub = self.hub()?;
        let capabilities = {
            let hub = hub.borrow();
            hub.attachments_for_target(&target.handle)?
                .into_iter()
                .find(|attachment| attachment.instrument == instrument.handle)
                .map(|attachment| attachment.granted_capabilities.clone())
                .ok_or_else(|| InstrumentationError::AttachmentRequired {
                    instrument_id: instrument.instrument_id().into(),
                    target_id: target.target_id().into(),
                })?
        };
        Ok(capabilities)
    }

    pub fn queued_event_count(
        &self,
        instrument: &NativeInstrumentHandle,
    ) -> Result<usize, NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        let hub = self.hub()?;
        let count = hub.borrow().queued_event_count(&instrument.handle)?;
        Ok(count)
    }

    pub fn drain_events(
        &self,
        instrument: &NativeInstrumentHandle,
    ) -> Result<EventBatch, NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        let hub = self.hub()?;
        let batch = hub.borrow_mut().drain_events(&instrument.handle)?;
        Ok(batch)
    }

    pub fn detach(
        &self,
        instrument: &NativeInstrumentHandle,
    ) -> Result<(), NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        let hub = self.hub()?;
        hub.borrow_mut().detach(&instrument.handle)?;
        Ok(())
    }

    pub fn acquire_control(
        &self,
        instrument: &NativeInstrumentHandle,
        target: &NativeTargetHandle,
    ) -> Result<NativeControlLease, NativeInstrumentationError> {
        self.ensure_instrument(instrument)?;
        self.ensure_target(target)?;
        let hub = self.hub()?;
        let lease = hub
            .borrow_mut()
            .acquire_control(&instrument.handle, &target.handle)?;
        Ok(NativeControlLease {
            session_id: self.session_id.clone(),
            hub: self.hub.clone(),
            lease,
        })
    }

    pub fn release_control(
        &self,
        lease: &NativeControlLease,
    ) -> Result<(), NativeInstrumentationError> {
        self.ensure_lease(lease)?;
        let hub = self.hub()?;
        hub.borrow_mut().release_control(&lease.lease)?;
        Ok(())
    }

    pub fn request_directive(
        &self,
        lease: &NativeControlLease,
        directive: InstrumentDirective,
    ) -> Result<(), NativeInstrumentationError> {
        self.ensure_lease(lease)?;
        let hub = self.hub()?;
        hub.borrow_mut()
            .request_directive(&lease.lease, directive)?;
        Ok(())
    }

    fn hub(&self) -> Result<Rc<RefCell<InstrumentationHub>>, NativeInstrumentationError> {
        self.hub
            .upgrade()
            .ok_or_else(|| NativeInstrumentationError::RuntimeClosed {
                session_id: self.session_id.clone(),
            })
    }

    fn ensure_instrument(
        &self,
        instrument: &NativeInstrumentHandle,
    ) -> Result<(), NativeInstrumentationError> {
        self.ensure_owner("instrument", &instrument.session_id, &instrument.hub)
    }

    fn ensure_target(&self, target: &NativeTargetHandle) -> Result<(), NativeInstrumentationError> {
        self.ensure_owner("target", &target.session_id, &target.hub)
    }

    fn ensure_lease(&self, lease: &NativeControlLease) -> Result<(), NativeInstrumentationError> {
        self.ensure_owner("control lease", &lease.session_id, &lease.hub)
    }

    fn ensure_owner(
        &self,
        kind: &'static str,
        session_id: &str,
        hub: &Weak<RefCell<InstrumentationHub>>,
    ) -> Result<(), NativeInstrumentationError> {
        if !Weak::ptr_eq(&self.hub, hub) {
            return Err(NativeInstrumentationError::CrossRuntimeHandle { kind });
        }
        if session_id != self.session_id {
            return Err(self.cross_session(kind, session_id));
        }
        self.hub()?;
        Ok(())
    }

    fn cross_session(&self, kind: &'static str, actual: &str) -> NativeInstrumentationError {
        NativeInstrumentationError::CrossSessionHandle {
            kind,
            expected: self.session_id.clone(),
            actual: actual.into(),
        }
    }

    fn instrument_handle(&self, handle: InstrumentHandle) -> NativeInstrumentHandle {
        NativeInstrumentHandle {
            session_id: self.session_id.clone(),
            hub: self.hub.clone(),
            handle,
        }
    }

    fn target_handle(&self, handle: TargetHandle) -> NativeTargetHandle {
        NativeTargetHandle {
            session_id: self.session_id.clone(),
            hub: self.hub.clone(),
            handle,
        }
    }

    fn attachment(&self, attachment: InstrumentationAttachment) -> NativeAttachment {
        NativeAttachment {
            instrument: self.instrument_handle(attachment.instrument),
            target: self.target_handle(attachment.target),
            granted_capabilities: attachment.granted_capabilities,
            registration_order: attachment.registration_order,
        }
    }
}

impl NativeInstrumentHandle {
    pub fn instrument_id(&self) -> &str {
        self.handle.instrument_id()
    }

    pub fn generation(&self) -> u64 {
        self.handle.generation()
    }
}

impl NativeTargetHandle {
    pub fn target_id(&self) -> &str {
        self.handle.target_id()
    }

    pub fn generation(&self) -> u64 {
        self.handle.generation()
    }
}

impl NativeControlLease {
    pub fn instrument_id(&self) -> &str {
        self.lease.instrument().instrument_id()
    }

    pub fn target_id(&self) -> &str {
        self.lease.target().target_id()
    }
}

impl NativeAttachment {
    pub fn instrument(&self) -> &NativeInstrumentHandle {
        &self.instrument
    }

    pub fn target(&self) -> &NativeTargetHandle {
        &self.target
    }

    pub fn granted_capabilities(&self) -> &BTreeSet<Capability> {
        &self.granted_capabilities
    }

    pub fn registration_order(&self) -> u64 {
        self.registration_order
    }
}

#[cfg(test)]
#[path = "native/tests.rs"]
mod tests;
