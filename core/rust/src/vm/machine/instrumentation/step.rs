use std::collections::BTreeMap;

use super::super::{Dispatch, Machine, VmSlot};
use super::{
    InstructionEvent, Opcode, TerminalEvent, TerminalKind, TransitionEvent, TransitionKind, VmProbe,
};
use crate::core::{Promise, PromiseState, Value};
use crate::instrumentation::{EventLocation, PortableProjection, ProjectionLimits, SourceSpan};
use crate::vm::error::VmError;
use crate::vm::opcode::Instruction;

/// Runtime outcome retained separately from portable instrumentation data.
pub enum VmBoundaryOutcome {
    Continue,
    Suspended(Promise),
    Yielded(Value),
    Returned(Value),
    Failed(VmError),
}

/// One actual HBC dispatch boundary with only scalar event metadata.
pub struct VmBoundary {
    pub instruction: Option<InstructionEvent>,
    pub transition: Option<TransitionEvent>,
    pub terminal: Option<TerminalEvent>,
    pub outcome: VmBoundaryOutcome,
}

impl Machine {
    /// Executes exactly one production dispatch boundary without projecting VM
    /// state. The caller may lazily inspect the machine after event/filter
    /// matching through the shared instrumentation hub.
    pub fn step_instrumented_boundary<P: VmProbe>(&mut self, probe: &mut P) -> VmBoundary {
        self.clear_step_jit_state();
        let program = self.program.clone();
        let Some(function) = program.functions.get(self.function) else {
            let error = VmError::new("function index out of range", 0, None);
            let terminal = self.boundary_terminal_event(TerminalKind::Fail);
            probe.on_terminal(terminal);
            return VmBoundary {
                instruction: None,
                transition: None,
                terminal: Some(terminal),
                outcome: VmBoundaryOutcome::Failed(error),
            };
        };
        let Some(instruction) = function.code.get(self.ip).cloned() else {
            let error = self.error(function, "instruction pointer out of range");
            let terminal = self.boundary_terminal_event(TerminalKind::Fail);
            probe.on_terminal(terminal);
            return VmBoundary {
                instruction: None,
                transition: None,
                terminal: Some(terminal),
                outcome: VmBoundaryOutcome::Failed(error),
            };
        };
        let instruction_event = self.boundary_instruction_event(&instruction);
        probe.on_instruction(instruction_event);
        let from_function = self.function;
        let from_ip = self.ip;

        match self.dispatch(&program, function, &instruction) {
            Dispatch::Next(ip) => {
                self.ip = ip;
                VmBoundary {
                    instruction: Some(instruction_event),
                    transition: None,
                    terminal: None,
                    outcome: VmBoundaryOutcome::Continue,
                }
            }
            Dispatch::Unwound(ip) => {
                self.ip = ip;
                self.transition_boundary(
                    probe,
                    instruction_event,
                    TransitionKind::ExceptionUnwind,
                    from_function,
                    from_ip,
                    VmBoundaryOutcome::Continue,
                )
            }
            Dispatch::Call { callee, args } => {
                if let Err(message) = self.enter_callable(&program, callee, args) {
                    match self.raise(function, message) {
                        Ok(target) => {
                            self.ip = target;
                            self.transition_boundary(
                                probe,
                                instruction_event,
                                TransitionKind::ExceptionUnwind,
                                from_function,
                                from_ip,
                                VmBoundaryOutcome::Continue,
                            )
                        }
                        Err(error) => self.failed_boundary(probe, Some(instruction_event), error),
                    }
                } else {
                    self.transition_boundary(
                        probe,
                        instruction_event,
                        TransitionKind::CallEnter,
                        from_function,
                        from_ip,
                        VmBoundaryOutcome::Continue,
                    )
                }
            }
            Dispatch::CallStatic {
                prototype,
                args,
                captures,
            } => {
                self.enter_or_spawn(&program, prototype, args, captures);
                self.transition_boundary(
                    probe,
                    instruction_event,
                    TransitionKind::CallEnter,
                    from_function,
                    from_ip,
                    VmBoundaryOutcome::Continue,
                )
            }
            Dispatch::CallStaticDirect { prototype, argc } => {
                self.enter_static_direct(&program, prototype, argc);
                self.transition_boundary(
                    probe,
                    instruction_event,
                    TransitionKind::CallEnter,
                    from_function,
                    from_ip,
                    VmBoundaryOutcome::Continue,
                )
            }
            Dispatch::Returned(value) => {
                self.stack.truncate(self.frame.base());
                if let Some(caller) = self.calls.pop() {
                    self.function = caller.function;
                    let completed = std::mem::replace(&mut self.frame, caller.frame);
                    self.free_locals.push(completed.into_locals());
                    self.ip = caller.call_ip + 1;
                    self.stack.push(value);
                    self.transition_boundary(
                        probe,
                        instruction_event,
                        TransitionKind::CallReturn,
                        from_function,
                        from_ip,
                        VmBoundaryOutcome::Continue,
                    )
                } else {
                    let terminal = self.boundary_terminal_event(TerminalKind::Return);
                    probe.on_terminal(terminal);
                    VmBoundary {
                        instruction: Some(instruction_event),
                        transition: None,
                        terminal: Some(terminal),
                        outcome: VmBoundaryOutcome::Returned(Self::into_value(
                            program.clone(),
                            value,
                        )),
                    }
                }
            }
            Dispatch::Suspended(promise) => self.transition_boundary(
                probe,
                instruction_event,
                TransitionKind::MachineSuspend,
                from_function,
                from_ip,
                VmBoundaryOutcome::Suspended(promise),
            ),
            Dispatch::Yielded(value) => self.transition_boundary(
                probe,
                instruction_event,
                TransitionKind::MachineSuspend,
                from_function,
                from_ip,
                VmBoundaryOutcome::Yielded(value),
            ),
            Dispatch::Failed(error) => self.failed_boundary(probe, Some(instruction_event), error),
        }
    }

    /// Applies exactly one settlement at the actual retained `Await` boundary.
    /// Subsequent instructions are left for later `step_instrumented_boundary`
    /// calls.
    pub fn resume_instrumented_boundary<P: VmProbe>(
        &mut self,
        state: PromiseState,
        probe: &mut P,
    ) -> VmBoundary {
        self.clear_step_jit_state();
        let from_function = self.function;
        let from_ip = self.ip;
        let Some(function) = self.program.functions.get(self.function).cloned() else {
            let error = VmError::new("function index out of range", 0, None);
            return self.failed_boundary(probe, None, error);
        };
        if !matches!(function.code.get(self.ip), Some(Instruction::Await)) {
            let error = self.error(&function, "VM is not suspended at await");
            return self.failed_boundary(probe, None, error);
        }
        match state {
            PromiseState::Pending => {
                let promise = match self.stack.last().and_then(VmSlot::runtime_value) {
                    Some(Value::Promise(promise)) => promise,
                    _ => {
                        let error = self.error(&function, "await expects a promise");
                        return self.failed_boundary(probe, None, error);
                    }
                };
                self.resume_transition_boundary(
                    probe,
                    TransitionKind::MachineSuspend,
                    from_function,
                    from_ip,
                    VmBoundaryOutcome::Suspended(promise),
                )
            }
            PromiseState::Fulfilled(value) => {
                self.stack.pop();
                self.stack.push(value.into());
                self.ip += 1;
                self.resume_transition_boundary(
                    probe,
                    TransitionKind::MachineResume,
                    from_function,
                    from_ip,
                    VmBoundaryOutcome::Continue,
                )
            }
            PromiseState::Rejected(error) => {
                self.stack.pop();
                match self.raise(&function, crate::core::promise_rejection_error(error)) {
                    Ok(target) => {
                        self.ip = target;
                        self.resume_transition_boundary(
                            probe,
                            TransitionKind::ExceptionUnwind,
                            from_function,
                            from_ip,
                            VmBoundaryOutcome::Continue,
                        )
                    }
                    Err(error) => self.failed_boundary(probe, None, error),
                }
            }
        }
    }

    pub(crate) fn instrumentation_location_at(
        &self,
        function: usize,
        ip: usize,
        source_id: &str,
    ) -> EventLocation {
        let prototype = self.program.functions.get(function);
        let position = prototype.and_then(|prototype| prototype.source_map.position(ip));
        EventLocation {
            source_id: Some(source_id.into()),
            form_path: None,
            span: position.map(|position| SourceSpan {
                start: position.offset,
                end: position.offset,
            }),
            function: prototype.and_then(|prototype| prototype.name.clone()),
            instruction_pointer: Some(ip),
        }
    }

    pub(crate) fn instrumentation_current_frame(
        &self,
        limits: ProjectionLimits,
    ) -> PortableProjection {
        let mut projection = PortableProjection::new("hbc/current-frame")
            .with_field("function", self.function.to_string())
            .with_field("ip", self.ip.to_string())
            .with_field("stack-base", self.frame.base().to_string());
        if let Some(name) = self
            .program
            .functions
            .get(self.function)
            .and_then(|function| function.name.as_ref())
        {
            projection
                .fields
                .insert("function/name".into(), name.clone());
        }
        append_slots(
            &mut projection.fields,
            "local",
            self.frame.locals(),
            limits,
            false,
        );
        projection
    }

    pub(crate) fn instrumentation_frames(&self, limits: ProjectionLimits) -> PortableProjection {
        let mut projection = PortableProjection::new("hbc/frames")
            .with_field("count", (self.calls.len() + 1).to_string());
        let start = self.calls.len().saturating_sub(limits.max_items);
        for (index, frame) in self.calls[start..].iter().enumerate() {
            projection.fields.insert(
                format!("frame/{index}/function"),
                frame.function.to_string(),
            );
            projection
                .fields
                .insert(format!("frame/{index}/call-ip"), frame.call_ip.to_string());
            if let Some(name) = self
                .program
                .functions
                .get(frame.function)
                .and_then(|function| function.name.as_ref())
            {
                projection
                    .fields
                    .insert(format!("frame/{index}/name"), name.clone());
            }
        }
        projection
            .fields
            .insert("omitted".into(), start.to_string());
        projection
    }

    pub(crate) fn instrumentation_locals(&self, limits: ProjectionLimits) -> PortableProjection {
        let mut projection = PortableProjection::new("hbc/locals");
        append_slots(
            &mut projection.fields,
            "local",
            self.frame.locals(),
            limits,
            false,
        );
        projection
    }

    pub(crate) fn instrumentation_stack(&self, limits: ProjectionLimits) -> PortableProjection {
        let mut projection = PortableProjection::new("hbc/stack");
        append_slots(&mut projection.fields, "stack", &self.stack, limits, true);
        projection
    }

    pub(crate) fn instrumentation_value_preview(
        &self,
        limits: ProjectionLimits,
    ) -> Option<PortableProjection> {
        let value = self.stack.last()?;
        Some(
            PortableProjection::new("hbc/value-preview")
                .with_field("display", slot_display(value, limits.max_bytes.min(16_384))),
        )
    }

    pub(crate) fn instrumentation_snapshot(&self, limits: ProjectionLimits) -> PortableProjection {
        let mut projection = PortableProjection::new("hbc/snapshot")
            .with_field("program/entry", self.program.entry.to_string())
            .with_field(
                "program/functions",
                self.program.functions.len().to_string(),
            )
            .with_field(
                "program/constants",
                self.program.constants.len().to_string(),
            )
            .with_field("function", self.function.to_string())
            .with_field("ip", self.ip.to_string())
            .with_field("calls", self.calls.len().to_string())
            .with_field("stack/depth", self.stack.len().to_string())
            .with_field("locals/count", self.frame.locals().len().to_string());
        append_slots(&mut projection.fields, "stack", &self.stack, limits, true);
        projection
    }

    fn transition_boundary<P: VmProbe>(
        &self,
        probe: &mut P,
        instruction: InstructionEvent,
        kind: TransitionKind,
        from_function: usize,
        from_ip: usize,
        outcome: VmBoundaryOutcome,
    ) -> VmBoundary {
        let transition = self.boundary_transition_event(kind, from_function, from_ip);
        probe.on_transition(transition);
        VmBoundary {
            instruction: Some(instruction),
            transition: Some(transition),
            terminal: None,
            outcome,
        }
    }

    fn resume_transition_boundary<P: VmProbe>(
        &self,
        probe: &mut P,
        kind: TransitionKind,
        from_function: usize,
        from_ip: usize,
        outcome: VmBoundaryOutcome,
    ) -> VmBoundary {
        let transition = self.boundary_transition_event(kind, from_function, from_ip);
        probe.on_transition(transition);
        VmBoundary {
            instruction: None,
            transition: Some(transition),
            terminal: None,
            outcome,
        }
    }

    fn failed_boundary<P: VmProbe>(
        &self,
        probe: &mut P,
        instruction: Option<InstructionEvent>,
        error: VmError,
    ) -> VmBoundary {
        let terminal = self.boundary_terminal_event(TerminalKind::Fail);
        probe.on_terminal(terminal);
        VmBoundary {
            instruction,
            transition: None,
            terminal: Some(terminal),
            outcome: VmBoundaryOutcome::Failed(error),
        }
    }

    #[inline(always)]
    fn boundary_instruction_event(&self, instruction: &Instruction) -> InstructionEvent {
        InstructionEvent {
            function: saturating_u16(self.function),
            ip: saturating_u32(self.ip),
            opcode: Opcode::from_instruction(instruction),
            stack_depth: saturating_u32(self.stack.len()),
            call_depth: saturating_u16(self.calls.len()),
        }
    }

    #[inline(always)]
    fn boundary_transition_event(
        &self,
        kind: TransitionKind,
        from_function: usize,
        from_ip: usize,
    ) -> TransitionEvent {
        TransitionEvent {
            kind,
            from_function: saturating_u16(from_function),
            from_ip: saturating_u32(from_ip),
            to_function: saturating_u16(self.function),
            to_ip: saturating_u32(self.ip),
            stack_depth: saturating_u32(self.stack.len()),
            call_depth: saturating_u16(self.calls.len()),
        }
    }

    #[inline(always)]
    fn boundary_terminal_event(&self, kind: TerminalKind) -> TerminalEvent {
        TerminalEvent {
            kind,
            function: saturating_u16(self.function),
            ip: saturating_u32(self.ip),
            stack_depth: saturating_u32(self.stack.len()),
            call_depth: saturating_u16(self.calls.len()),
        }
    }

    #[cfg(feature = "tracing-jit")]
    fn clear_step_jit_state(&mut self) {
        self.jit_path.clear();
        self.jit_loop_entries.clear();
        self.jit_suppressed_range = None;
    }

    #[cfg(not(feature = "tracing-jit"))]
    fn clear_step_jit_state(&mut self) {}
}

fn append_slots(
    fields: &mut BTreeMap<String, String>,
    prefix: &str,
    slots: &[VmSlot],
    limits: ProjectionLimits,
    tail: bool,
) {
    let retained = slots.len().min(limits.max_items);
    let start = if tail {
        slots.len().saturating_sub(retained)
    } else {
        0
    };
    for (output_index, value) in slots[start..start + retained].iter().enumerate() {
        let source_index = start + output_index;
        fields.insert(
            format!("{prefix}/{source_index}"),
            slot_display(value, limits.max_bytes.min(16_384)),
        );
    }
    fields.insert(format!("{prefix}/count"), slots.len().to_string());
    fields.insert(
        format!("{prefix}/omitted"),
        slots.len().saturating_sub(retained).to_string(),
    );
}

fn slot_display(slot: &VmSlot, limit: usize) -> String {
    let display = match slot {
        VmSlot::Number(value) => value.to_string(),
        VmSlot::Bool(value) => value.to_string(),
        VmSlot::Nil => "nil".into(),
        VmSlot::Value(value) => value.display(),
        VmSlot::InlineClosure {
            prototype,
            identity,
        } => {
            format!("#<hbc-closure {prototype}@{identity}>")
        }
        VmSlot::Closure(closure) => format!("#<hbc-closure {}>", closure.prototype),
        VmSlot::MultiArity(dispatch) => format!("#<hbc-multi-arity {}>", dispatch.name),
    };
    bounded_text(&display, limit)
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.into();
    }
    let mut output = value.chars().take(limit).collect::<String>();
    output.push('…');
    output
}

#[inline(always)]
fn saturating_u16(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

#[inline(always)]
fn saturating_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}
