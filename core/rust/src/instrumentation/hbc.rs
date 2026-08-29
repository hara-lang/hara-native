use std::collections::BTreeSet;

use crate::core::{Promise, PromiseState, Value};
use crate::vm::machine::instrumentation::{
    NoProbe, TerminalEvent, TransitionEvent, TransitionKind, VmBoundary, VmBoundaryOutcome,
};
use crate::vm::Machine;

use super::{
    Capability, ControlLease, DispatchReport, EventAccess, EventKind, EventLocation,
    InstrumentDirective, InstrumentationError, InstrumentationHub, PortableProjection,
    ProducerEvent, ProjectionLimits, TargetHandle, TargetKind,
};

/// Exact Rust HBC capabilities backed by the production Machine dispatch path.
pub fn hbc_capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([
        Capability::EventInstruction,
        Capability::EventCall,
        Capability::EventException,
        Capability::EventSuspension,
        Capability::EventLifecycle,
        Capability::InspectSourceLocation,
        Capability::InspectCurrentFrame,
        Capability::InspectFrames,
        Capability::InspectLocals,
        Capability::InspectStack,
        Capability::InspectValuePreview,
        Capability::InspectSnapshot,
        Capability::ControlPause,
        Capability::ControlSingleStep,
        Capability::ControlResume,
        Capability::ControlSettle,
        Capability::ControlTerminate,
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HbcBoundary {
    pub status: &'static str,
    pub paused: bool,
    pub reports: Vec<DispatchReport>,
}

enum HbcState {
    Running,
    Suspended(Promise),
    Yielded(Value),
    Returned(Value),
    Failed(String),
    Cancelled,
}

/// One instrumentation target around the actual validated HBC machine.
pub struct HbcTarget {
    target: TargetHandle,
    source_id: String,
    machine: Machine,
    state: HbcState,
    paused: bool,
    terminal_emitted: bool,
}

impl HbcTarget {
    pub fn new(
        hub: &InstrumentationHub,
        target: TargetHandle,
        source_id: impl Into<String>,
        machine: Machine,
    ) -> Result<Self, InstrumentationError> {
        let descriptor = hub.target_descriptor(&target)?;
        if descriptor.kind != TargetKind::Hbc {
            return Err(InstrumentationError::InvalidTarget(
                "HBC probe requires an HBC target".into(),
            ));
        }
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(InstrumentationError::InvalidTarget(
                "HBC source id must be non-empty".into(),
            ));
        }
        Ok(Self {
            target,
            source_id,
            machine,
            state: HbcState::Running,
            paused: false,
            terminal_emitted: false,
        })
    }

    pub fn target(&self) -> &TargetHandle {
        &self.target
    }

    pub fn status(&self) -> &'static str {
        state_keyword(&self.state)
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pending(&self) -> Option<Promise> {
        match &self.state {
            HbcState::Suspended(promise) => Some(promise.clone()),
            _ => None,
        }
    }

    pub fn result(&self) -> Option<Value> {
        match &self.state {
            HbcState::Returned(value) | HbcState::Yielded(value) => Some(value.clone()),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match &self.state {
            HbcState::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// Executes one real instruction or documented transition/terminal
    /// boundary through `Machine::dispatch` without eager snapshots.
    pub fn step(
        &mut self,
        hub: &mut InstrumentationHub,
    ) -> Result<HbcBoundary, InstrumentationError> {
        if self.finished() {
            return Ok(self.boundary(Vec::new()));
        }
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
                self.state = HbcState::Cancelled;
                self.paused = true;
                let mut reports = Vec::new();
                self.emit_terminal(hub, "cancelled", None, &mut reports)?;
                return Ok(self.boundary(reports));
            }
            InstrumentDirective::Continue | InstrumentDirective::StepNext => {
                self.paused = false;
            }
        }

        let mut probe = NoProbe;
        let boundary = self.machine.step_instrumented_boundary(&mut probe);
        let mut reports = Vec::new();
        self.emit_boundary(hub, &boundary, &mut reports)?;
        self.apply_outcome(boundary.outcome);
        if pause_after {
            self.paused = true;
        }
        Ok(self.boundary(reports))
    }

    pub fn run(
        &mut self,
        hub: &mut InstrumentationHub,
        boundary_limit: usize,
    ) -> Result<Vec<HbcBoundary>, InstrumentationError> {
        let mut boundaries = Vec::new();
        for _ in 0..boundary_limit {
            if self.paused || self.finished() || matches!(&self.state, HbcState::Suspended(_)) {
                break;
            }
            let boundary = self.step(hub)?;
            boundaries.push(boundary);
            if self.paused || self.finished() || matches!(&self.state, HbcState::Suspended(_)) {
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
        self.check_lease_target(lease)?;
        self.paused = false;
        Ok(())
    }

    /// Settles the exact Promise retained at the machine's current `Await`.
    pub fn settle(
        &mut self,
        hub: &mut InstrumentationHub,
        lease: &ControlLease,
        state: PromiseState,
    ) -> Result<HbcBoundary, InstrumentationError> {
        hub.authorize_control(lease, Capability::ControlSettle)?;
        self.check_lease_target(lease)?;
        if !matches!(&self.state, HbcState::Suspended(_)) {
            return Err(InstrumentationError::Execution(
                "HBC target is not suspended".into(),
            ));
        }
        if matches!(state, PromiseState::Pending) {
            return Err(InstrumentationError::Execution(
                "HBC settlement cannot remain pending".into(),
            ));
        }
        self.paused = false;
        let mut probe = NoProbe;
        let boundary = self.machine.resume_instrumented_boundary(state, &mut probe);
        let mut reports = Vec::new();
        self.emit_boundary(hub, &boundary, &mut reports)?;
        self.apply_outcome(boundary.outcome);
        Ok(self.boundary(reports))
    }

    fn emit_boundary(
        &mut self,
        hub: &mut InstrumentationHub,
        boundary: &VmBoundary,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        if let Some(instruction) = boundary.instruction {
            let location = self.machine.instrumentation_location_at(
                usize::from(instruction.function),
                instruction.ip as usize,
                &self.source_id,
            );
            self.emit(
                hub,
                ProducerEvent::live(EventKind::InstructionExecute)
                    .with_data("opcode", instruction.opcode.as_keyword())
                    .with_data("stack/depth", instruction.stack_depth.to_string())
                    .with_data("call/depth", instruction.call_depth.to_string()),
                Some(location),
                reports,
            )?;
        }
        if let Some(transition) = boundary.transition {
            self.emit_transition(hub, transition, reports)?;
        }
        if let Some(terminal) = boundary.terminal {
            let status = match terminal.kind {
                crate::vm::machine::instrumentation::TerminalKind::Return => "returned",
                crate::vm::machine::instrumentation::TerminalKind::Fail => "failed",
            };
            self.emit_terminal(hub, status, Some(terminal), reports)?;
        }
        Ok(())
    }

    fn emit_transition(
        &self,
        hub: &mut InstrumentationHub,
        transition: TransitionEvent,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        let event = match transition.kind {
            TransitionKind::CallEnter => EventKind::CallEnter,
            TransitionKind::CallReturn => EventKind::CallReturn,
            TransitionKind::ExceptionUnwind => EventKind::ExceptionUnwind,
            TransitionKind::MachineSuspend => EventKind::MachineSuspend,
            TransitionKind::MachineResume => EventKind::MachineResume,
        };
        let location = self.machine.instrumentation_location_at(
            usize::from(transition.from_function),
            transition.from_ip as usize,
            &self.source_id,
        );
        self.emit(
            hub,
            ProducerEvent::live(event)
                .with_data("from/function", transition.from_function.to_string())
                .with_data("from/ip", transition.from_ip.to_string())
                .with_data("to/function", transition.to_function.to_string())
                .with_data("to/ip", transition.to_ip.to_string()),
            Some(location),
            reports,
        )
    }

    fn emit_terminal(
        &mut self,
        hub: &mut InstrumentationHub,
        status: &str,
        terminal: Option<TerminalEvent>,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        if self.terminal_emitted {
            return Ok(());
        }
        let location = terminal.map(|terminal| {
            self.machine.instrumentation_location_at(
                usize::from(terminal.function),
                terminal.ip as usize,
                &self.source_id,
            )
        });
        let mut event =
            ProducerEvent::live(EventKind::ExecutionTerminal).with_data("status", status);
        if let Some(terminal) = terminal {
            event = event
                .with_data("stack/depth", terminal.stack_depth.to_string())
                .with_data("call/depth", terminal.call_depth.to_string());
        }
        self.terminal_emitted = true;
        self.emit(hub, event, location, reports)
    }

    fn emit(
        &self,
        hub: &mut InstrumentationHub,
        event: ProducerEvent,
        location: Option<EventLocation>,
        reports: &mut Vec<DispatchReport>,
    ) -> Result<(), InstrumentationError> {
        let mut access = HbcAccess {
            machine: &self.machine,
            location,
        };
        reports.push(hub.emit(&self.target, event, &mut access)?);
        Ok(())
    }

    fn apply_outcome(&mut self, outcome: VmBoundaryOutcome) {
        self.state = match outcome {
            VmBoundaryOutcome::Continue => HbcState::Running,
            VmBoundaryOutcome::Suspended(promise) => HbcState::Suspended(promise),
            VmBoundaryOutcome::Yielded(value) => HbcState::Yielded(value),
            VmBoundaryOutcome::Returned(value) => HbcState::Returned(value),
            VmBoundaryOutcome::Failed(error) => HbcState::Failed(error.to_string()),
        };
    }

    fn check_lease_target(&self, lease: &ControlLease) -> Result<(), InstrumentationError> {
        if lease.target() == &self.target {
            Ok(())
        } else {
            Err(InstrumentationError::InvalidControlLease {
                target_id: self.target.target_id().into(),
                instrument_id: lease.instrument().instrument_id().into(),
            })
        }
    }

    fn finished(&self) -> bool {
        matches!(
            &self.state,
            HbcState::Yielded(_)
                | HbcState::Returned(_)
                | HbcState::Failed(_)
                | HbcState::Cancelled
        )
    }

    fn boundary(&self, reports: Vec<DispatchReport>) -> HbcBoundary {
        HbcBoundary {
            status: state_keyword(&self.state),
            paused: self.paused,
            reports,
        }
    }
}

struct HbcAccess<'a> {
    machine: &'a Machine,
    location: Option<EventLocation>,
}

impl EventAccess for HbcAccess<'_> {
    fn source_location(&mut self) -> Option<EventLocation> {
        self.location.clone()
    }

    fn current_frame(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        Some(self.machine.instrumentation_current_frame(limits))
    }

    fn frames(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        Some(self.machine.instrumentation_frames(limits))
    }

    fn locals(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        Some(self.machine.instrumentation_locals(limits))
    }

    fn stack(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        Some(self.machine.instrumentation_stack(limits))
    }

    fn value_preview(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.machine.instrumentation_value_preview(limits)
    }

    fn machine_snapshot(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        Some(self.machine.instrumentation_snapshot(limits))
    }
}

fn state_keyword(state: &HbcState) -> &'static str {
    match state {
        HbcState::Running => "running",
        HbcState::Suspended(_) => "suspended",
        HbcState::Yielded(_) => "yielded",
        HbcState::Returned(_) => "returned",
        HbcState::Failed(_) => "failed",
        HbcState::Cancelled => "cancelled",
    }
}

#[cfg(test)]
#[path = "hbc/tests.rs"]
mod tests;
