use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use super::{
    Capability, EventKind, EventMask, InstrumentHandle, InstrumentMode, InstrumentRegistration,
    RuntimeBackend, TargetDescriptor, TargetHandle,
};

#[path = "hub/delivery.rs"]
mod delivery;
pub use delivery::{
    DeliveredEvent, DispatchReport, EventAccess, EventBatch, EventProjection, PortableProjection,
    ProducerEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentationAttachment {
    pub instrument: InstrumentHandle,
    pub target: TargetHandle,
    pub granted_capabilities: BTreeSet<Capability>,
    pub registration_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlLease {
    instrument: InstrumentHandle,
    target: TargetHandle,
}

impl ControlLease {
    pub fn instrument(&self) -> &InstrumentHandle {
        &self.instrument
    }

    pub fn target(&self) -> &TargetHandle {
        &self.target
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCleanup {
    pub instruments: usize,
    pub targets: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstrumentationError {
    InvalidRegistration(String),
    InvalidTarget(String),
    Execution(String),
    DuplicateInstrument(String),
    DuplicateTarget(String),
    DuplicateAttachment {
        instrument_id: String,
        target_id: String,
    },
    UnknownInstrument(String),
    UnknownTarget(String),
    StaleInstrumentHandle {
        instrument_id: String,
        generation: u64,
    },
    StaleTargetHandle {
        target_id: String,
        generation: u64,
    },
    SessionMismatch {
        instrument_session: String,
        target_session: String,
    },
    FilterMismatch {
        instrument_id: String,
        target_id: String,
    },
    UnsupportedCapabilities {
        target_id: String,
        backend: RuntimeBackend,
        missing: BTreeSet<Capability>,
    },
    UnsupportedEvents {
        target_id: String,
        backend: RuntimeBackend,
        events: BTreeSet<EventKind>,
    },
    ControlModeRequired(String),
    AttachmentRequired {
        instrument_id: String,
        target_id: String,
    },
    ControlLeaseHeld {
        target_id: String,
        holder: String,
    },
    InvalidControlLease {
        target_id: String,
        instrument_id: String,
    },
}

impl fmt::Display for InstrumentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegistration(message) => {
                write!(formatter, "instrumentation/invalid-registration: {message}")
            }
            Self::InvalidTarget(message) => {
                write!(formatter, "instrumentation/invalid-target: {message}")
            }
            Self::Execution(message) => {
                write!(formatter, "instrumentation/execution: {message}")
            }
            Self::DuplicateInstrument(id) => {
                write!(formatter, "instrumentation/duplicate-instrument: {id}")
            }
            Self::DuplicateTarget(id) => {
                write!(formatter, "instrumentation/duplicate-target: {id}")
            }
            Self::DuplicateAttachment {
                instrument_id,
                target_id,
            } => write!(
                formatter,
                "instrumentation/duplicate-attachment: {instrument_id} -> {target_id}"
            ),
            Self::UnknownInstrument(id) => {
                write!(formatter, "instrumentation/unknown-instrument: {id}")
            }
            Self::UnknownTarget(id) => {
                write!(formatter, "instrumentation/unknown-target: {id}")
            }
            Self::StaleInstrumentHandle {
                instrument_id,
                generation,
            } => write!(
                formatter,
                "instrumentation/stale-instrument: {instrument_id}@{generation}"
            ),
            Self::StaleTargetHandle {
                target_id,
                generation,
            } => write!(
                formatter,
                "instrumentation/stale-target: {target_id}@{generation}"
            ),
            Self::SessionMismatch {
                instrument_session,
                target_session,
            } => write!(
                formatter,
                "instrumentation/session-mismatch: instrument {instrument_session}, target {target_session}"
            ),
            Self::FilterMismatch {
                instrument_id,
                target_id,
            } => write!(
                formatter,
                "instrumentation/filter-mismatch: {instrument_id} -> {target_id}"
            ),
            Self::UnsupportedCapabilities {
                target_id,
                backend,
                missing,
            } => write!(
                formatter,
                "instrumentation/unsupported-capabilities: target {target_id}, backend {}, missing {missing:?}",
                backend.as_str()
            ),
            Self::UnsupportedEvents {
                target_id,
                backend,
                events,
            } => write!(
                formatter,
                "instrumentation/unsupported-events: target {target_id}, backend {}, events {events:?}",
                backend.as_str()
            ),
            Self::ControlModeRequired(id) => {
                write!(formatter, "instrumentation/control-mode-required: {id}")
            }
            Self::AttachmentRequired {
                instrument_id,
                target_id,
            } => write!(
                formatter,
                "instrumentation/attachment-required: {instrument_id} -> {target_id}"
            ),
            Self::ControlLeaseHeld { target_id, holder } => write!(
                formatter,
                "instrumentation/control-lease-held: target {target_id}, holder {holder}"
            ),
            Self::InvalidControlLease {
                target_id,
                instrument_id,
            } => write!(
                formatter,
                "instrumentation/invalid-control-lease: {instrument_id} -> {target_id}"
            ),
        }
    }
}

impl Error for InstrumentationError {}

#[derive(Debug, Clone)]
struct InstrumentRecord {
    handle: InstrumentHandle,
    registration: InstrumentRegistration,
    order: u64,
}

#[derive(Debug, Clone)]
struct TargetRecord {
    handle: TargetHandle,
    descriptor: TargetDescriptor,
}

#[derive(Debug, Default)]
pub struct InstrumentationHub {
    registrations: BTreeMap<u64, InstrumentRecord>,
    instrument_orders: BTreeMap<String, u64>,
    instrument_generations: BTreeMap<String, u64>,
    targets: BTreeMap<String, TargetRecord>,
    target_generations: BTreeMap<String, u64>,
    attachments: BTreeMap<(InstrumentHandle, TargetHandle), InstrumentationAttachment>,
    enabled_events: EventMask,
    control_leases: BTreeMap<TargetHandle, InstrumentHandle>,
    next_registration_order: u64,
    delivery: delivery::DeliveryState,
}

impl InstrumentationHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn enabled_events(&self) -> EventMask {
        self.enabled_events
    }

    pub fn registration_count(&self) -> usize {
        self.registrations.len()
    }

    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    pub fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    pub fn registrations(
        &self,
    ) -> impl Iterator<Item = (&InstrumentHandle, &InstrumentRegistration)> {
        self.registrations
            .values()
            .map(|record| (&record.handle, &record.registration))
    }

    pub fn register(
        &mut self,
        registration: InstrumentRegistration,
    ) -> Result<InstrumentHandle, InstrumentationError> {
        registration
            .validate()
            .map_err(|message| InstrumentationError::InvalidRegistration(message.into()))?;
        if self
            .instrument_orders
            .contains_key(&registration.instrument_id)
        {
            return Err(InstrumentationError::DuplicateInstrument(
                registration.instrument_id,
            ));
        }
        let generation = next_generation(
            &mut self.instrument_generations,
            &registration.instrument_id,
        );
        let handle = InstrumentHandle::new(registration.instrument_id.clone(), generation);
        let order = self.next_registration_order;
        self.next_registration_order = self.next_registration_order.saturating_add(1);
        self.instrument_orders
            .insert(registration.instrument_id.clone(), order);
        self.registrations.insert(
            order,
            InstrumentRecord {
                handle: handle.clone(),
                registration,
                order,
            },
        );
        Ok(handle)
    }

    pub fn register_target(
        &mut self,
        descriptor: TargetDescriptor,
    ) -> Result<TargetHandle, InstrumentationError> {
        descriptor
            .validate()
            .map_err(|message| InstrumentationError::InvalidTarget(message.into()))?;
        if self.targets.contains_key(&descriptor.target_id) {
            return Err(InstrumentationError::DuplicateTarget(descriptor.target_id));
        }
        let generation = next_generation(&mut self.target_generations, &descriptor.target_id);
        let handle = TargetHandle::new(descriptor.target_id.clone(), generation);
        self.targets.insert(
            descriptor.target_id.clone(),
            TargetRecord {
                handle: handle.clone(),
                descriptor,
            },
        );
        Ok(handle)
    }

    pub fn attach(
        &mut self,
        instrument: &InstrumentHandle,
        target: &TargetHandle,
    ) -> Result<InstrumentationAttachment, InstrumentationError> {
        let instrument_record = self.resolve_instrument(instrument)?;
        let target_record = self.resolve_target(target)?;
        if instrument_record.registration.session_id != target_record.descriptor.session_id {
            return Err(InstrumentationError::SessionMismatch {
                instrument_session: instrument_record.registration.session_id.clone(),
                target_session: target_record.descriptor.session_id.clone(),
            });
        }
        if !instrument_record
            .registration
            .filter
            .matches(&target_record.descriptor)
        {
            return Err(InstrumentationError::FilterMismatch {
                instrument_id: instrument.instrument_id().into(),
                target_id: target.target_id().into(),
            });
        }
        let unsupported_events = instrument_record
            .registration
            .events
            .iter()
            .copied()
            .filter(|event| !event.supports_target(target_record.descriptor.kind))
            .collect::<BTreeSet<_>>();
        if !unsupported_events.is_empty() {
            return Err(InstrumentationError::UnsupportedEvents {
                target_id: target.target_id().into(),
                backend: target_record.descriptor.backend.clone(),
                events: unsupported_events,
            });
        }
        let missing = instrument_record
            .registration
            .capabilities
            .difference(&target_record.descriptor.capabilities)
            .copied()
            .collect::<BTreeSet<_>>();
        if !missing.is_empty() {
            return Err(InstrumentationError::UnsupportedCapabilities {
                target_id: target.target_id().into(),
                backend: target_record.descriptor.backend.clone(),
                missing,
            });
        }
        let key = (instrument.clone(), target.clone());
        if self.attachments.contains_key(&key) {
            return Err(InstrumentationError::DuplicateAttachment {
                instrument_id: instrument.instrument_id().into(),
                target_id: target.target_id().into(),
            });
        }
        let attachment = InstrumentationAttachment {
            instrument: instrument.clone(),
            target: target.clone(),
            granted_capabilities: instrument_record.registration.capabilities.clone(),
            registration_order: instrument_record.order,
        };
        self.attachments.insert(key, attachment.clone());
        self.recompute_event_mask();
        Ok(attachment)
    }

    pub fn attachments_for_target(
        &self,
        target: &TargetHandle,
    ) -> Result<Vec<&InstrumentationAttachment>, InstrumentationError> {
        self.resolve_target(target)?;
        let mut attachments = self
            .attachments
            .values()
            .filter(|attachment| &attachment.target == target)
            .collect::<Vec<_>>();
        attachments.sort_by_key(|attachment| attachment.registration_order);
        Ok(attachments)
    }

    pub fn enabled_for_target(
        &self,
        target: &TargetHandle,
        event: EventKind,
    ) -> Result<bool, InstrumentationError> {
        self.resolve_target(target)?;
        if !self.enabled_events.contains(event) {
            return Ok(false);
        }
        Ok(self.attachments.iter().any(|((instrument, attached), _)| {
            attached == target
                && self
                    .resolve_instrument(instrument)
                    .map_or(false, |record| record.registration.events.contains(&event))
        }))
    }

    pub fn acquire_control(
        &mut self,
        instrument: &InstrumentHandle,
        target: &TargetHandle,
    ) -> Result<ControlLease, InstrumentationError> {
        let record = self.resolve_instrument(instrument)?;
        self.resolve_target(target)?;
        if record.registration.mode != InstrumentMode::Control {
            return Err(InstrumentationError::ControlModeRequired(
                instrument.instrument_id().into(),
            ));
        }
        if !self
            .attachments
            .contains_key(&(instrument.clone(), target.clone()))
        {
            return Err(InstrumentationError::AttachmentRequired {
                instrument_id: instrument.instrument_id().into(),
                target_id: target.target_id().into(),
            });
        }
        if let Some(holder) = self.control_leases.get(target) {
            if holder != instrument {
                return Err(InstrumentationError::ControlLeaseHeld {
                    target_id: target.target_id().into(),
                    holder: holder.instrument_id().into(),
                });
            }
        }
        self.control_leases
            .insert(target.clone(), instrument.clone());
        Ok(ControlLease {
            instrument: instrument.clone(),
            target: target.clone(),
        })
    }

    pub fn release_control(&mut self, lease: &ControlLease) -> Result<(), InstrumentationError> {
        self.resolve_instrument(&lease.instrument)?;
        self.resolve_target(&lease.target)?;
        match self.control_leases.get(&lease.target) {
            Some(holder) if holder == &lease.instrument => {
                self.control_leases.remove(&lease.target);
                self.delivery.remove_directive(&lease.target);
                Ok(())
            }
            _ => Err(InstrumentationError::InvalidControlLease {
                target_id: lease.target.target_id().into(),
                instrument_id: lease.instrument.instrument_id().into(),
            }),
        }
    }

    pub fn detach(&mut self, instrument: &InstrumentHandle) -> Result<(), InstrumentationError> {
        let order = self.resolve_instrument(instrument)?.order;
        self.registrations.remove(&order);
        self.instrument_orders.remove(instrument.instrument_id());
        self.attachments
            .retain(|(candidate, _), _| candidate != instrument);
        let controlled_targets = self
            .control_leases
            .iter()
            .filter(|(_, holder)| *holder == instrument)
            .map(|(target, _)| (*target).clone())
            .collect::<Vec<_>>();
        self.control_leases.retain(|_, holder| holder != instrument);
        for target in &controlled_targets {
            self.delivery.remove_directive(target);
        }
        self.delivery.remove_instrument(instrument);
        self.recompute_event_mask();
        Ok(())
    }

    pub fn remove_target(&mut self, target: &TargetHandle) -> Result<(), InstrumentationError> {
        self.resolve_target(target)?;
        self.targets.remove(target.target_id());
        self.attachments
            .retain(|(_, candidate), _| candidate != target);
        self.control_leases.remove(target);
        self.delivery.remove_target(target);
        self.recompute_event_mask();
        Ok(())
    }

    pub fn detach_session(&mut self, session_id: &str) -> SessionCleanup {
        let instruments = self
            .registrations
            .values()
            .filter(|record| record.registration.session_id == session_id)
            .map(|record| record.handle.clone())
            .collect::<Vec<_>>();
        let targets = self
            .targets
            .values()
            .filter(|record| record.descriptor.session_id == session_id)
            .map(|record| record.handle.clone())
            .collect::<Vec<_>>();
        for instrument in &instruments {
            self.detach(instrument)
                .expect("session cleanup collected a live instrument handle");
        }
        for target in &targets {
            self.remove_target(target)
                .expect("session cleanup collected a live target handle");
        }
        SessionCleanup {
            instruments: instruments.len(),
            targets: targets.len(),
        }
    }

    pub fn clear(&mut self) {
        self.registrations.clear();
        self.instrument_orders.clear();
        self.targets.clear();
        self.attachments.clear();
        self.control_leases.clear();
        self.enabled_events = EventMask::empty();
        self.delivery.clear();
    }

    fn resolve_instrument(
        &self,
        handle: &InstrumentHandle,
    ) -> Result<&InstrumentRecord, InstrumentationError> {
        let Some(order) = self.instrument_orders.get(handle.instrument_id()) else {
            return if self
                .instrument_generations
                .contains_key(handle.instrument_id())
            {
                Err(InstrumentationError::StaleInstrumentHandle {
                    instrument_id: handle.instrument_id().into(),
                    generation: handle.generation(),
                })
            } else {
                Err(InstrumentationError::UnknownInstrument(
                    handle.instrument_id().into(),
                ))
            };
        };
        let record = self
            .registrations
            .get(order)
            .expect("instrument index and registry must remain consistent");
        if &record.handle == handle {
            Ok(record)
        } else {
            Err(InstrumentationError::StaleInstrumentHandle {
                instrument_id: handle.instrument_id().into(),
                generation: handle.generation(),
            })
        }
    }

    fn resolve_target(&self, handle: &TargetHandle) -> Result<&TargetRecord, InstrumentationError> {
        let Some(record) = self.targets.get(handle.target_id()) else {
            return if self.target_generations.contains_key(handle.target_id()) {
                Err(InstrumentationError::StaleTargetHandle {
                    target_id: handle.target_id().into(),
                    generation: handle.generation(),
                })
            } else {
                Err(InstrumentationError::UnknownTarget(
                    handle.target_id().into(),
                ))
            };
        };
        if &record.handle == handle {
            Ok(record)
        } else {
            Err(InstrumentationError::StaleTargetHandle {
                target_id: handle.target_id().into(),
                generation: handle.generation(),
            })
        }
    }

    fn recompute_event_mask(&mut self) {
        let mut mask = EventMask::empty();
        for (instrument, _) in self.attachments.keys() {
            if let Ok(record) = self.resolve_instrument(instrument) {
                for event in &record.registration.events {
                    mask.insert(*event);
                }
            }
        }
        self.enabled_events = mask;
    }
}

fn next_generation(generations: &mut BTreeMap<String, u64>, id: &str) -> u64 {
    let next = generations.entry(id.into()).or_insert(0);
    let generation = *next;
    *next = next.saturating_add(1);
    generation
}
