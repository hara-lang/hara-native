use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::instrumentation::{
    Capability, EventDelivery, EventEnvelope, EventKind, EventLocation, EventPhase,
    InstrumentDirective, InstrumentHandle, InstrumentRegistration, ProjectionLimits,
    ProjectionRequest, TargetDescriptor, TargetHandle, INSTRUMENTATION_EVENT_SCHEMA,
    INSTRUMENTATION_PROTOCOL,
};

use super::{ControlLease, InstrumentationError, InstrumentationHub};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortableProjection {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

impl PortableProjection {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            fields: BTreeMap::new(),
        }
    }

    pub fn with_field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(name.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventProjection {
    pub current_frame: Option<PortableProjection>,
    pub frames: Option<PortableProjection>,
    pub locals: Option<PortableProjection>,
    pub stack: Option<PortableProjection>,
    pub value_preview: Option<PortableProjection>,
    pub machine_snapshot: Option<PortableProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerEvent {
    pub phase: EventPhase,
    pub event: EventKind,
    pub data: BTreeMap<String, String>,
}

impl ProducerEvent {
    pub fn live(event: EventKind) -> Self {
        Self {
            phase: EventPhase::Live,
            event,
            data: BTreeMap::new(),
        }
    }

    pub fn with_data(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(name.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredEvent {
    pub envelope: EventEnvelope,
    pub projection: EventProjection,
    /// Cumulative events discarded from this instrument queue before this
    /// event was retained. Callback deliveries always report zero.
    pub dropped_before: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventBatch {
    pub events: Vec<DeliveredEvent>,
    pub dropped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchReport {
    /// Callback delivery is returned to the trusted native caller. The hub
    /// never executes arbitrary guest code on the application thread.
    pub callbacks: Vec<DeliveredEvent>,
    pub queued: usize,
    pub dropped: usize,
}

/// Borrowed, lazy inspection over one authoritative execution object.
/// Producers may return `None` for a projection not supported by their target.
pub trait EventAccess {
    fn source_location(&mut self) -> Option<EventLocation> {
        None
    }

    fn current_frame(&mut self, _limits: ProjectionLimits) -> Option<PortableProjection> {
        None
    }

    fn frames(&mut self, _limits: ProjectionLimits) -> Option<PortableProjection> {
        None
    }

    fn locals(&mut self, _limits: ProjectionLimits) -> Option<PortableProjection> {
        None
    }

    fn stack(&mut self, _limits: ProjectionLimits) -> Option<PortableProjection> {
        None
    }

    fn value_preview(&mut self, _limits: ProjectionLimits) -> Option<PortableProjection> {
        None
    }

    fn machine_snapshot(&mut self, _limits: ProjectionLimits) -> Option<PortableProjection> {
        None
    }
}

#[derive(Debug, Default)]
pub(super) struct DeliveryState {
    queues: BTreeMap<InstrumentHandle, InstrumentQueue>,
    sequences: BTreeMap<TargetHandle, u64>,
    directives: BTreeMap<TargetHandle, InstrumentDirective>,
}

#[derive(Debug)]
struct InstrumentQueue {
    capacity: usize,
    events: VecDeque<DeliveredEvent>,
    dropped: u64,
}

impl InstrumentQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            events: VecDeque::with_capacity(capacity),
            dropped: 0,
        }
    }

    fn push(&mut self, mut event: DeliveredEvent) -> bool {
        let dropped = if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
            true
        } else {
            false
        };
        event.dropped_before = self.dropped;
        self.events.push_back(event);
        dropped
    }

    fn drain(&mut self) -> EventBatch {
        let events = self.events.drain(..).collect();
        let dropped = std::mem::take(&mut self.dropped);
        EventBatch { events, dropped }
    }
}

impl DeliveryState {
    pub(super) fn remove_instrument(&mut self, instrument: &InstrumentHandle) {
        self.queues.remove(instrument);
    }

    pub(super) fn remove_directive(&mut self, target: &TargetHandle) {
        self.directives.remove(target);
    }

    pub(super) fn remove_target(&mut self, target: &TargetHandle) {
        self.sequences.remove(target);
        self.directives.remove(target);
        for queue in self.queues.values_mut() {
            queue
                .events
                .retain(|event| event.envelope.target_id != target.target_id());
        }
    }

    pub(super) fn clear(&mut self) {
        self.queues.clear();
        self.sequences.clear();
        self.directives.clear();
    }
}

impl InstrumentDirective {
    pub const fn required_capability(self) -> Option<Capability> {
        match self {
            Self::Continue => None,
            Self::Suspend => Some(Capability::ControlPause),
            Self::StepNext => Some(Capability::ControlSingleStep),
            Self::Terminate => Some(Capability::ControlTerminate),
        }
    }
}

impl ProjectionRequest {
    pub fn is_empty(&self) -> bool {
        !self.source_location
            && self.current_frame.is_none()
            && self.frames.is_none()
            && self.locals.is_none()
            && self.stack.is_none()
            && self.value_preview.is_none()
            && self.machine_snapshot.is_none()
    }

    pub fn needs_interpreter_environment(&self) -> bool {
        self.current_frame.is_some() || self.frames.is_some()
    }

    pub fn merge_from(&mut self, other: &Self) {
        self.source_location |= other.source_location;
        self.current_frame = merge_limits(self.current_frame, other.current_frame);
        self.frames = merge_limits(self.frames, other.frames);
        self.locals = merge_limits(self.locals, other.locals);
        self.stack = merge_limits(self.stack, other.stack);
        self.value_preview = merge_limits(self.value_preview, other.value_preview);
        self.machine_snapshot = merge_limits(self.machine_snapshot, other.machine_snapshot);
    }
}

impl InstrumentationHub {
    pub fn target_descriptor(
        &self,
        target: &TargetHandle,
    ) -> Result<&TargetDescriptor, InstrumentationError> {
        Ok(&self.resolve_target(target)?.descriptor)
    }

    pub fn instrument_registration(
        &self,
        instrument: &InstrumentHandle,
    ) -> Result<&InstrumentRegistration, InstrumentationError> {
        Ok(&self.resolve_instrument(instrument)?.registration)
    }

    /// Returns the aggregate projection required by matching subscriptions for
    /// one target/event. Producers use this before executing a safepoint so
    /// environment or frame capture is enabled only when it can be consumed.
    pub fn requested_projection(
        &self,
        target: &TargetHandle,
        event: EventKind,
    ) -> Result<ProjectionRequest, InstrumentationError> {
        self.resolve_target(target)?;
        if !self.enabled_events.contains(event) {
            return Ok(ProjectionRequest::default());
        }
        let mut projection = ProjectionRequest::default();
        for attachment in self.attachments_for_target(target)? {
            let record = self.resolve_instrument(&attachment.instrument)?;
            if record.registration.events.contains(&event) {
                projection.merge_from(&record.registration.projection);
            }
        }
        Ok(projection)
    }

    /// Delivers one cheap producer event to every matching attachment in
    /// deterministic registration order. Projection methods are never called
    /// until an attachment has matched both target and event.
    pub fn emit<A: EventAccess>(
        &mut self,
        target: &TargetHandle,
        event: ProducerEvent,
        access: &mut A,
    ) -> Result<DispatchReport, InstrumentationError> {
        if !self.enabled_events.contains(event.event) {
            self.resolve_target(target)?;
            return Ok(DispatchReport::default());
        }

        let descriptor = self.resolve_target(target)?.descriptor.clone();
        let subscriptions = self
            .attachments_for_target(target)?
            .into_iter()
            .filter_map(|attachment| {
                let record = self.resolve_instrument(&attachment.instrument).ok()?;
                record.registration.events.contains(&event.event).then(|| {
                    (
                        attachment.instrument.clone(),
                        record.registration.projection.clone(),
                        record.registration.delivery.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        if subscriptions.is_empty() {
            return Ok(DispatchReport::default());
        }

        let sequence = self.delivery.sequences.entry(target.clone()).or_insert(0);
        *sequence = sequence.saturating_add(1);
        let sequence = *sequence;
        let mut report = DispatchReport::default();

        for (instrument, request, delivery) in subscriptions {
            let (location, projection) = materialize(&request, access);
            let delivered = DeliveredEvent {
                envelope: EventEnvelope {
                    schema: INSTRUMENTATION_EVENT_SCHEMA.into(),
                    protocol: INSTRUMENTATION_PROTOCOL.into(),
                    instrument_id: instrument.instrument_id().into(),
                    runtime: descriptor.backend.clone(),
                    session_id: descriptor.session_id.clone(),
                    target_id: descriptor.target_id.clone(),
                    target_kind: descriptor.kind,
                    generation: target.generation(),
                    sequence,
                    phase: event.phase,
                    event: event.event,
                    location,
                    data: event.data.clone(),
                },
                projection,
                dropped_before: 0,
            };
            match delivery {
                EventDelivery::Callback => report.callbacks.push(delivered),
                EventDelivery::Queue { capacity } => {
                    let queue = self
                        .delivery
                        .queues
                        .entry(instrument)
                        .or_insert_with(|| InstrumentQueue::new(capacity));
                    debug_assert_eq!(queue.capacity, capacity);
                    if queue.push(delivered) {
                        report.dropped = report.dropped.saturating_add(1);
                    }
                    report.queued = report.queued.saturating_add(1);
                }
            }
        }
        Ok(report)
    }

    pub fn drain_events(
        &mut self,
        instrument: &InstrumentHandle,
    ) -> Result<EventBatch, InstrumentationError> {
        let registration = self.resolve_instrument(instrument)?.registration.clone();
        let EventDelivery::Queue { capacity } = registration.delivery else {
            return Ok(EventBatch::default());
        };
        Ok(self
            .delivery
            .queues
            .entry(instrument.clone())
            .or_insert_with(|| InstrumentQueue::new(capacity))
            .drain())
    }

    pub fn queued_event_count(
        &self,
        instrument: &InstrumentHandle,
    ) -> Result<usize, InstrumentationError> {
        self.resolve_instrument(instrument)?;
        Ok(self
            .delivery
            .queues
            .get(instrument)
            .map_or(0, |queue| queue.events.len()))
    }

    pub fn authorize_control(
        &self,
        lease: &ControlLease,
        capability: Capability,
    ) -> Result<(), InstrumentationError> {
        let instrument = self.resolve_instrument(&lease.instrument)?;
        let target = self.resolve_target(&lease.target)?;
        match self.control_leases.get(&lease.target) {
            Some(holder) if holder == &lease.instrument => {}
            _ => {
                return Err(InstrumentationError::InvalidControlLease {
                    target_id: lease.target.target_id().into(),
                    instrument_id: lease.instrument.instrument_id().into(),
                });
            }
        }
        if !capability.is_control() || !instrument.registration.capabilities.contains(&capability) {
            return Err(InstrumentationError::UnsupportedCapabilities {
                target_id: target.descriptor.target_id.clone(),
                backend: target.descriptor.backend.clone(),
                missing: BTreeSet::from([capability]),
            });
        }
        Ok(())
    }

    /// Stores a controller directive for application at the next real
    /// evaluator or machine safepoint. `Continue` clears a pending directive.
    pub fn request_directive(
        &mut self,
        lease: &ControlLease,
        directive: InstrumentDirective,
    ) -> Result<(), InstrumentationError> {
        if let Some(capability) = directive.required_capability() {
            self.authorize_control(lease, capability)?;
            self.delivery
                .directives
                .insert(lease.target.clone(), directive);
        } else {
            self.authorize_lease(lease)?;
            self.delivery.directives.remove(&lease.target);
        }
        Ok(())
    }

    pub fn take_directive(
        &mut self,
        target: &TargetHandle,
    ) -> Result<InstrumentDirective, InstrumentationError> {
        self.resolve_target(target)?;
        Ok(self
            .delivery
            .directives
            .remove(target)
            .unwrap_or(InstrumentDirective::Continue))
    }

    fn authorize_lease(&self, lease: &ControlLease) -> Result<(), InstrumentationError> {
        self.resolve_instrument(&lease.instrument)?;
        self.resolve_target(&lease.target)?;
        match self.control_leases.get(&lease.target) {
            Some(holder) if holder == &lease.instrument => Ok(()),
            _ => Err(InstrumentationError::InvalidControlLease {
                target_id: lease.target.target_id().into(),
                instrument_id: lease.instrument.instrument_id().into(),
            }),
        }
    }
}

fn merge_limits(
    left: Option<ProjectionLimits>,
    right: Option<ProjectionLimits>,
) -> Option<ProjectionLimits> {
    match (left, right) {
        (None, value) | (value, None) => value,
        (Some(left), Some(right)) => Some(ProjectionLimits {
            max_items: left.max_items.max(right.max_items),
            max_depth: left.max_depth.max(right.max_depth),
            max_bytes: left.max_bytes.max(right.max_bytes),
        }),
    }
}

fn materialize<A: EventAccess>(
    request: &ProjectionRequest,
    access: &mut A,
) -> (Option<EventLocation>, EventProjection) {
    let location = request
        .source_location
        .then(|| access.source_location())
        .flatten();
    let projection = EventProjection {
        current_frame: request
            .current_frame
            .and_then(|limits| access.current_frame(limits)),
        frames: request.frames.and_then(|limits| access.frames(limits)),
        locals: request.locals.and_then(|limits| access.locals(limits)),
        stack: request.stack.and_then(|limits| access.stack(limits)),
        value_preview: request
            .value_preview
            .and_then(|limits| access.value_preview(limits)),
        machine_snapshot: request
            .machine_snapshot
            .and_then(|limits| access.machine_snapshot(limits)),
    };
    (location, projection)
}
