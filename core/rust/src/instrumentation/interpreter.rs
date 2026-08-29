use std::collections::{BTreeSet, HashMap};

use crate::core::{EvalFiber, EvalFiberState, PromiseState, Value};

use super::{
    Capability, ControlLease, DispatchReport, EventAccess, EventKind, EventLocation,
    InstrumentDirective, InstrumentationError, InstrumentationHub, PortableProjection,
    ProducerEvent, ProjectionLimits, TargetHandle, TargetKind,
};

const SEMANTIC_EVENTS: [EventKind; 6] = [
    EventKind::SemanticBoundary,
    EventKind::CallEnter,
    EventKind::CallReturn,
    EventKind::ExceptionRaise,
    EventKind::VarSet,
    EventKind::FieldSet,
];

/// Exact Rust interpreter capabilities backed by the production CPS fiber.
pub fn interpreter_capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([
        Capability::EventSemanticBoundary,
        Capability::EventCall,
        Capability::EventException,
        Capability::EventEffect,
        Capability::EventSuspension,
        Capability::EventLifecycle,
        Capability::InspectSourceLocation,
        Capability::InspectCurrentFrame,
        Capability::InspectFrames,
        Capability::InspectLocals,
        Capability::InspectValuePreview,
        Capability::InspectSnapshot,
        Capability::ControlPause,
        Capability::ControlSingleStep,
        Capability::ControlResume,
        Capability::ControlSettle,
        Capability::ControlTerminate,
    ])
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterpreterBoundary {
    pub state: EvalFiberState,
    pub paused: bool,
    pub reports: Vec<DispatchReport>,
}

/// One instrumentation target around the actual retained CPS continuation.
/// It neither replays source nor owns a second evaluator.
pub struct InterpreterTarget {
    target: TargetHandle,
    source_id: String,
    fiber: EvalFiber,
    semantic_sequence: usize,
    paused: bool,
    terminal_emitted: bool,
}

impl InterpreterTarget {
    pub fn start(
        hub: &InstrumentationHub,
        target: TargetHandle,
        source_id: impl Into<String>,
        source: &str,
        environment: HashMap<String, Value>,
    ) -> Result<Self, InstrumentationError> {
        let descriptor = hub.target_descriptor(&target)?;
        if descriptor.kind != TargetKind::Interpreter {
            return Err(InstrumentationError::InvalidTarget(
                "interpreter probe requires an interpreter target".into(),
            ));
        }
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(InstrumentationError::InvalidTarget(
                "interpreter source id must be non-empty".into(),
            ));
        }
        let fiber = EvalFiber::start_observed(source, environment)
            .map_err(InstrumentationError::Execution)?;
        fiber.configure_instrumentation_capture(false, false);
        Ok(Self {
            target,
            source_id,
            fiber,
            semantic_sequence: 0,
            paused: false,
            terminal_emitted: false,
        })
    }

    pub fn target(&self) -> &TargetHandle {
        &self.target
    }

    pub fn state(&self) -> EvalFiberState {
        self.fiber.state()
    }

    pub fn pending(&self) -> Option<crate::core::Promise> {
        self.fiber.pending()
    }

    pub fn environment(&self) -> HashMap<String, Value> {
        self.fiber.environment()
    }

    pub fn environment_clone_count(&self) -> u64 {
        self.fiber.instrumentation_environment_clone_count()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Executes at most one genuine evaluator or queued semantic boundary.
    pub fn step(
        &mut self,
        hub: &mut InstrumentationHub,
    ) -> Result<InterpreterBoundary, InstrumentationError> {
        let directive = hub.take_directive(&self.target)?;
        if self.paused && directive == InstrumentDirective::Continue {
            return Ok(self.boundary(Vec::new()));
        }
        let pause_after = directive == InstrumentDirective::StepNext;
        match directive {
            InstrumentDirective::Suspend => {
                self.paused = true;
                return Ok(self.boundary(Vec::new()));
            }
            InstrumentDirective::Terminate => {
                self.fiber.cancel();
                self.paused = true;
                let mut reports = Vec::new();
                self.emit_terminal(hub, &mut reports)?;
                return Ok(self.boundary(reports));
            }
            InstrumentDirective::Continue | InstrumentDirective::StepNext => {
                self.paused = false;
            }
        }

        self.refresh_capture(hub)?;
        let before = self.fiber.state();
        self.fiber.step_observed();
        let after = self.fiber.state();
        let mut reports = Vec::new();
        self.emit_current_semantic(hub, &mut reports)?;
        if !matches!(before, EvalFiberState::Suspended)
            && matches!(after, EvalFiberState::Suspended)
        {
            self.emit(
                hub,
                ProducerEvent::live(EventKind::PromiseSuspend),
                &mut reports,
            )?;
        }
        self.emit_terminal(hub, &mut reports)?;
        if pause_after {
            self.paused = true;
        }
        Ok(self.boundary(reports))
    }

    /// Runs genuine boundaries until completion, suspension, an explicit pause,
    /// or the caller's bound. This is a driver over the retained fiber, not a
    /// replay loop.
    pub fn run(
        &mut self,
        hub: &mut InstrumentationHub,
        boundary_limit: usize,
    ) -> Result<Vec<InterpreterBoundary>, InstrumentationError> {
        let mut boundaries = Vec::new();
        for _ in 0..boundary_limit {
            if self.paused || self.finished() {
                break;
            }
            let boundary = self.step(hub)?;
            let suspended = matches!(boundary.state, EvalFiberState::Suspended);
            boundaries.push(boundary);
            if self.paused || suspended || self.finished() {
                break;
            }
        }
        Ok(boundaries)
    }

    pub fn continue_execution(
        &mut self,
        hub: &InstrumentationHub,
        lease: &ControlLease,
    ) -> Result<(), InstrumentationError> {
        hub.authorize_control(lease, Capability::ControlResume)?;
        if lease.target() != &self.target {
            return Err(InstrumentationError::InvalidControlLease {
                target_id: self.target.target_id().into(),
                instrument_id: lease.instrument().instrument_id().into(),
            });
        }
        self.paused = false;
        Ok(())
    }

    /// Settles and resumes the exact Promise/continuation retained by EvalFiber.
    pub fn settle(
        &mut self,
        hub: &mut InstrumentationHub,
        lease: &ControlLease,
        state: PromiseState,
    ) -> Result<InterpreterBoundary, InstrumentationError> {
        hub.authorize_control(lease, Capability::ControlSettle)?;
        if lease.target() != &self.target {
            return Err(InstrumentationError::InvalidControlLease {
                target_id: self.target.target_id().into(),
                instrument_id: lease.instrument().instrument_id().into(),
            });
        }
        if !matches!(self.fiber.state(), EvalFiberState::Suspended) {
            return Err(InstrumentationError::Execution(
                "interpreter target is not suspended".into(),
            ));
        }
        if matches!(state, PromiseState::Pending) {
            return Err(InstrumentationError::Execution(
                "interpreter settlement cannot remain pending".into(),
            ));
        }
        self.refresh_capture(hub)?;
        self.paused = false;
        self.fiber.resume_observed(state);
        let mut reports = Vec::new();
        self.emit(
            hub,
            ProducerEvent::live(EventKind::PromiseResume),
            &mut reports,
        )?;
        self.emit_current_semantic(hub, &mut reports)?;
        self.emit_terminal(hub, &mut reports)?;
        Ok(self.boundary(reports))
    }

    fn refresh_capture(&self, hub: &InstrumentationHub) -> Result<(), InstrumentationError> {
        let mut capture_events = false;
        let mut capture_environment = false;
        for event in SEMANTIC_EVENTS {
            if hub.enabled_for_target(&self.target, event)? {
                capture_events = true;
                capture_environment |= hub
                    .requested_projection(&self.target, event)?
                    .needs_interpreter_environment();
            }
        }
        self.fiber
            .configure_instrumentation_capture(capture_events, capture_environment);
        Ok(())
    }

    fn emit_current_semantic(
        &mut self,
        hub: &mut InstrumentationHub,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        let Some((sequence, event)) = self.fiber.instrumentation_event() else {
            return Ok(());
        };
        if sequence <= self.semantic_sequence {
            return Ok(());
        }
        self.semantic_sequence = sequence;
        self.emit(hub, event, reports)
    }

    fn emit_terminal(
        &mut self,
        hub: &mut InstrumentationHub,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        if self.terminal_emitted || self.fiber.observed_pending_boundaries() > 0 {
            return Ok(());
        }
        let state = self.fiber.state();
        let status = match &state {
            EvalFiberState::Completed(_) => "returned",
            EvalFiberState::Failed(_) => "failed",
            EvalFiberState::Cancelled => "cancelled",
            EvalFiberState::Running | EvalFiberState::Suspended => return Ok(()),
        };
        let mut event =
            ProducerEvent::live(EventKind::ExecutionTerminal).with_data("status", status);
        match state {
            EvalFiberState::Completed(value) => {
                event = event.with_data("result/type", crate::core::portable_type_name(&value));
            }
            EvalFiberState::Failed(error) => {
                event = event.with_data("error", bounded_text(&error, 1_024));
            }
            EvalFiberState::Cancelled | EvalFiberState::Running | EvalFiberState::Suspended => {}
        }
        self.terminal_emitted = true;
        self.emit(hub, event, reports)
    }

    fn emit(
        &self,
        hub: &mut InstrumentationHub,
        event: ProducerEvent,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        let mut access = InterpreterAccess {
            fiber: &self.fiber,
            source_id: &self.source_id,
        };
        reports.push(hub.emit(&self.target, event, &mut access)?);
        Ok(())
    }

    fn finished(&self) -> bool {
        matches!(
            self.fiber.state(),
            EvalFiberState::Completed(_) | EvalFiberState::Failed(_) | EvalFiberState::Cancelled
        ) && self.fiber.observed_pending_boundaries() == 0
    }

    fn boundary(&self, reports: Vec<DispatchReport>) -> InterpreterBoundary {
        InterpreterBoundary {
            state: self.fiber.state(),
            paused: self.paused,
            reports,
        }
    }
}

struct InterpreterAccess<'a> {
    fiber: &'a EvalFiber,
    source_id: &'a str,
}

impl EventAccess for InterpreterAccess<'_> {
    fn source_location(&mut self) -> Option<EventLocation> {
        self.fiber.instrumentation_source_location(self.source_id)
    }

    fn current_frame(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.fiber.instrumentation_current_frame(limits)
    }

    fn frames(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.fiber.instrumentation_frames(limits)
    }

    fn locals(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.fiber.instrumentation_locals(limits)
    }

    fn value_preview(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.fiber.instrumentation_value_preview(limits)
    }

    fn machine_snapshot(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.fiber.instrumentation_snapshot(limits)
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.into();
    }
    let mut output = value.chars().take(limit).collect::<String>();
    output.push('…');
    output
}

#[cfg(test)]
#[path = "interpreter/tests.rs"]
mod tests;
