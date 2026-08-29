use std::collections::{BTreeSet, HashMap};

use crate::core::{Promise, Value};

use super::*;
use crate::instrumentation::{
    EventDelivery, InstrumentFilter, InstrumentMode, InstrumentRegistration, ProjectionRequest,
    RuntimeBackend, TargetDescriptor,
};

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn register_target(hub: &mut InstrumentationHub) -> TargetHandle {
    hub.register_target(TargetDescriptor {
        target_id: "interpreter-1".into(),
        session_id: "session".into(),
        kind: TargetKind::Interpreter,
        backend: RuntimeBackend::new("rust").expect("valid backend"),
        capabilities: interpreter_capabilities(),
    })
    .expect("interpreter target")
}

fn passive_call_registration(projection: ProjectionRequest) -> InstrumentRegistration {
    let mut capabilities = set([Capability::EventCall]);
    capabilities.extend(projection.required_capabilities());
    InstrumentRegistration {
        instrument_id: "trace".into(),
        session_id: "session".into(),
        mode: InstrumentMode::Passive,
        capabilities,
        events: set([EventKind::CallEnter, EventKind::CallReturn]),
        filter: InstrumentFilter::default(),
        projection,
        delivery: EventDelivery::Queue { capacity: 32 },
    }
}

#[test]
fn no_instruments_execute_the_real_fiber_without_semantic_environment_clones() {
    let mut hub = InstrumentationHub::new();
    let handle = register_target(&mut hub);
    let mut target =
        InterpreterTarget::start(&hub, handle, "editor/main", "(+ 19 23)", HashMap::new())
            .expect("target start");

    target.run(&mut hub, 128).expect("target run");

    assert_eq!(target.state(), EvalFiberState::Completed(Value::Number(42)));
    assert_eq!(target.environment_clone_count(), 0);
    assert!(hub.enabled_events().is_empty());
}

#[test]
fn call_enter_and_return_events_come_from_the_real_cps_path_without_projection() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(passive_call_registration(ProjectionRequest::default()))
        .expect("instrument");
    let handle = register_target(&mut hub);
    hub.attach(&instrument, &handle).expect("attachment");
    let mut target = InterpreterTarget::start(
        &hub,
        handle,
        "editor/main",
        "((fn [x] (+ x 1)) 41)",
        HashMap::new(),
    )
    .expect("target start");

    target.run(&mut hub, 128).expect("target run");
    let events = hub.drain_events(&instrument).expect("events").events;

    assert_eq!(target.state(), EvalFiberState::Completed(Value::Number(42)));
    assert_eq!(target.environment_clone_count(), 0);
    let call_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.envelope.event,
                EventKind::CallEnter | EventKind::CallReturn
            )
        })
        .collect::<Vec<_>>();
    assert!(!call_events.is_empty());
    assert_eq!(
        call_events
            .iter()
            .filter(|event| event.envelope.event == EventKind::CallEnter)
            .count(),
        call_events
            .iter()
            .filter(|event| event.envelope.event == EventKind::CallReturn)
            .count()
    );
    assert_eq!(
        call_events.first().unwrap().envelope.event,
        EventKind::CallEnter
    );
    assert_eq!(
        call_events.last().unwrap().envelope.event,
        EventKind::CallReturn
    );
    assert!(events
        .iter()
        .all(|event| event.envelope.target_kind == TargetKind::Interpreter));
    assert!(events.iter().all(|event| {
        event
            .envelope
            .location
            .as_ref()
            .is_none_or(|location| location.instruction_pointer.is_none())
    }));
}

#[test]
fn frame_projection_enables_environment_capture_only_for_matching_events() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(passive_call_registration(ProjectionRequest {
            current_frame: Some(ProjectionLimits::default()),
            ..ProjectionRequest::default()
        }))
        .expect("instrument");
    let handle = register_target(&mut hub);
    hub.attach(&instrument, &handle).expect("attachment");
    let mut environment = HashMap::new();
    environment.insert("answer".into(), Value::Number(42));
    let mut target =
        InterpreterTarget::start(&hub, handle, "editor/main", "(+ answer 0)", environment)
            .expect("target start");

    target.run(&mut hub, 128).expect("target run");
    let events = hub.drain_events(&instrument).expect("events").events;

    assert!(target.environment_clone_count() > 0);
    assert!(events.iter().any(|event| {
        event
            .projection
            .current_frame
            .as_ref()
            .is_some_and(|frame| frame.fields.contains_key("binding/answer"))
    }));
}

#[test]
fn promise_settlement_resumes_the_exact_retained_promise_and_continuation() {
    let mut hub = InstrumentationHub::new();
    let controller = hub
        .register(InstrumentRegistration {
            instrument_id: "debugger".into(),
            session_id: "session".into(),
            mode: InstrumentMode::Control,
            capabilities: set([Capability::EventSuspension, Capability::ControlSettle]),
            events: set([EventKind::PromiseSuspend, EventKind::PromiseResume]),
            filter: InstrumentFilter::default(),
            projection: ProjectionRequest::default(),
            delivery: EventDelivery::Queue { capacity: 16 },
        })
        .expect("controller");
    let handle = register_target(&mut hub);
    hub.attach(&controller, &handle).expect("attachment");
    let lease = hub
        .acquire_control(&controller, &handle)
        .expect("control lease");
    let promise = Promise::new();
    let mut environment = HashMap::new();
    environment.insert("pending-value".into(), Value::Promise(promise.clone()));
    let mut target = InterpreterTarget::start(
        &hub,
        handle,
        "editor/main",
        "(Coroutine/await pending-value)",
        environment,
    )
    .expect("target start");

    target.run(&mut hub, 128).expect("run to suspension");
    assert_eq!(target.state(), EvalFiberState::Suspended);
    assert!(target
        .pending()
        .expect("retained promise")
        .same_identity(&promise));

    assert!(promise.resolve(Value::Number(42)));
    target
        .settle(&mut hub, &lease, promise.state())
        .expect("settlement");
    target.run(&mut hub, 128).expect("run to completion");

    assert_eq!(target.state(), EvalFiberState::Completed(Value::Number(42)));
    let events = hub.drain_events(&controller).expect("events").events;
    assert!(events
        .iter()
        .any(|event| event.envelope.event == EventKind::PromiseSuspend));
    assert!(events
        .iter()
        .any(|event| event.envelope.event == EventKind::PromiseResume));
}

#[test]
fn single_step_directive_pauses_after_one_authoritative_boundary() {
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
        InterpreterTarget::start(&hub, handle, "editor/main", "(do 1 2 3)", HashMap::new())
            .expect("target start");

    hub.request_directive(&lease, InstrumentDirective::StepNext)
        .expect("step request");
    let boundary = target.step(&mut hub).expect("one boundary");

    assert!(boundary.paused);
    let retained = target.step(&mut hub).expect("paused direct step");
    assert!(retained.paused);
    assert_eq!(target.run(&mut hub, 32).expect("paused run").len(), 0);
    assert!(matches!(target.state(), EvalFiberState::Running));
}
