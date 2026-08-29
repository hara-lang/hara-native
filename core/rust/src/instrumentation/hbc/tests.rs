use std::collections::BTreeSet;
use std::rc::Rc;

use super::*;
use crate::instrumentation::{
    EventDelivery, InstrumentFilter, InstrumentMode, InstrumentRegistration, ProjectionRequest,
    RuntimeBackend, TargetDescriptor,
};
use crate::vm::{compile_source, Machine};

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn register_target(hub: &mut InstrumentationHub) -> TargetHandle {
    hub.register_target(TargetDescriptor {
        target_id: "hbc-1".into(),
        session_id: "session".into(),
        kind: TargetKind::Hbc,
        backend: RuntimeBackend::new("rust").expect("valid backend"),
        capabilities: hbc_capabilities(),
    })
    .expect("HBC target")
}

fn machine(source: &str) -> Machine {
    Machine::entry(Rc::new(compile_source(source).expect("compiled program")))
}

#[test]
fn no_instruments_execute_the_actual_machine_without_retained_events() {
    let mut hub = InstrumentationHub::new();
    let handle = register_target(&mut hub);
    let mut target =
        HbcTarget::new(&hub, handle, "editor/main", machine("(+ 19 23)")).expect("target");

    target.run(&mut hub, 128).expect("run");

    assert_eq!(target.status(), "returned");
    assert_eq!(target.result(), Some(Value::Number(42)));
    assert!(hub.enabled_events().is_empty());
}

#[test]
fn instruction_and_call_events_are_emitted_from_real_dispatch_boundaries() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(InstrumentRegistration {
            instrument_id: "trace".into(),
            session_id: "session".into(),
            mode: InstrumentMode::Passive,
            capabilities: set([
                Capability::EventInstruction,
                Capability::EventCall,
                Capability::InspectSourceLocation,
            ]),
            events: set([
                EventKind::InstructionExecute,
                EventKind::CallEnter,
                EventKind::CallReturn,
            ]),
            filter: InstrumentFilter::default(),
            projection: ProjectionRequest {
                source_location: true,
                ..ProjectionRequest::default()
            },
            // Instruction events are deliberately retained alongside calls in
            // this test. Keep the queue above the bounded 256-step run so the
            // early call-enter boundary cannot be discarded before read-back.
            delivery: EventDelivery::Queue { capacity: 512 },
        })
        .expect("instrument");
    let handle = register_target(&mut hub);
    hub.attach(&instrument, &handle).expect("attachment");
    // Immediate anonymous calls are intentionally inlined by the compiler and
    // therefore have no machine call frame. A named function exercises the
    // authoritative Dispatch::Call/Returned frame boundaries instead.
    let mut target = HbcTarget::new(
        &hub,
        handle,
        "editor/main",
        machine("(do (defn f [x] (+ x 1)) (f 41))"),
    )
    .expect("target");

    let registry = crate::embedding_namespace_registry();
    crate::core::with_namespace_registry(&registry, || target.run(&mut hub, 256)).expect("run");
    let batch = hub.drain_events(&instrument).expect("events");
    assert_eq!(batch.dropped, 0, "focused call trace must not overflow");
    let events = batch.events;

    assert_eq!(target.result(), Some(Value::Number(42)));
    assert!(events
        .iter()
        .any(|event| event.envelope.event == EventKind::InstructionExecute));
    assert!(events
        .iter()
        .any(|event| event.envelope.event == EventKind::CallEnter));
    assert!(events
        .iter()
        .any(|event| event.envelope.event == EventKind::CallReturn));
    assert!(events.iter().all(|event| {
        event
            .envelope
            .location
            .as_ref()
            .is_none_or(|location| location.form_path.is_none())
    }));
}

#[test]
fn stack_projection_is_absent_until_requested() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(InstrumentRegistration {
            instrument_id: "trace".into(),
            session_id: "session".into(),
            mode: InstrumentMode::Passive,
            capabilities: set([Capability::EventInstruction, Capability::InspectStack]),
            events: set([EventKind::InstructionExecute]),
            filter: InstrumentFilter::default(),
            projection: ProjectionRequest {
                stack: Some(ProjectionLimits::default()),
                ..ProjectionRequest::default()
            },
            delivery: EventDelivery::Queue { capacity: 64 },
        })
        .expect("instrument");
    let handle = register_target(&mut hub);
    hub.attach(&instrument, &handle).expect("attachment");
    let mut target =
        HbcTarget::new(&hub, handle, "editor/main", machine("(+ 19 23)")).expect("target");

    target.run(&mut hub, 128).expect("run");
    let events = hub.drain_events(&instrument).expect("events").events;

    assert!(events.iter().any(|event| event.projection.stack.is_some()));
    assert!(events
        .iter()
        .all(|event| event.projection.current_frame.is_none()));
}

#[test]
fn single_step_directive_advances_one_machine_boundary_then_pauses() {
    let mut hub = InstrumentationHub::new();
    let controller = hub
        .register(InstrumentRegistration {
            instrument_id: "debugger".into(),
            session_id: "session".into(),
            mode: InstrumentMode::Control,
            capabilities: set([Capability::EventLifecycle, Capability::ControlSingleStep]),
            events: set([EventKind::ExecutionTerminal]),
            filter: InstrumentFilter::default(),
            projection: ProjectionRequest::default(),
            delivery: EventDelivery::Queue { capacity: 8 },
        })
        .expect("controller");
    let handle = register_target(&mut hub);
    hub.attach(&controller, &handle).expect("attachment");
    let lease = hub
        .acquire_control(&controller, &handle)
        .expect("control lease");
    let mut target =
        HbcTarget::new(&hub, handle, "editor/main", machine("(+ 19 23)")).expect("target");

    hub.request_directive(&lease, InstrumentDirective::StepNext)
        .expect("step request");
    let boundary = target.step(&mut hub).expect("one boundary");

    assert!(boundary.paused);
    let retained = target.step(&mut hub).expect("paused direct step");
    assert!(retained.paused);
    assert_eq!(target.status(), "running");
    assert_eq!(target.run(&mut hub, 32).expect("paused run").len(), 0);
}
