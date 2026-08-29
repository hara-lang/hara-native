use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::instrumentation::{
    Capability, EventAccess, EventDelivery, EventKind, InstrumentDirective, InstrumentFilter,
    InstrumentMode, InstrumentRegistration, InstrumentationError, InstrumentationHub,
    ProducerEvent, ProjectionRequest, RuntimeBackend, TargetDescriptor, TargetHandle, TargetKind,
};
use crate::{SessionId, SessionKernel};

use super::*;

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn service(session_id: &str) -> (Rc<RefCell<InstrumentationHub>>, NativeInstrumentation) {
    let hub = Rc::new(RefCell::new(InstrumentationHub::new()));
    let service = NativeInstrumentation::new(session_id, hub.clone());
    (hub, service)
}

fn passive_registration(id: &str, session_id: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Passive,
        capabilities: set([Capability::EventCall]),
        events: set([EventKind::CallEnter]),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
}

fn control_registration(id: &str, session_id: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Control,
        capabilities: set([Capability::ControlSingleStep]),
        events: BTreeSet::new(),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
}

fn target(
    id: &str,
    session_id: &str,
    capabilities: impl IntoIterator<Item = Capability>,
) -> TargetDescriptor {
    TargetDescriptor {
        target_id: id.into(),
        session_id: session_id.into(),
        kind: TargetKind::Interpreter,
        backend: RuntimeBackend::new("rust").expect("test backend is valid"),
        capabilities: set(capabilities),
    }
}

#[derive(Default)]
struct NoAccess;

impl EventAccess for NoAccess {}

#[test]
fn trusted_native_passive_agent_attaches_drains_and_detaches() {
    let (hub, service) = service("session");
    let instrument = service
        .register(passive_registration("trace", "session"))
        .expect("trusted registration");
    let target = hub
        .borrow_mut()
        .register_target(target("execution", "session", [Capability::EventCall]))
        .expect("target registration");
    let target = service.bind_target(&target).expect("bind target");

    let attachment = service
        .attach(&instrument, &target)
        .expect("trusted attachment");
    assert_eq!(attachment.instrument().instrument_id(), "trace");
    assert_eq!(attachment.target().target_id(), "execution");
    assert_eq!(
        attachment.granted_capabilities(),
        &set([Capability::EventCall])
    );
    assert_eq!(
        service
            .granted_capabilities(&instrument, &target)
            .expect("granted capabilities"),
        set([Capability::EventCall])
    );

    let raw_target = target.handle.clone();
    let mut access = NoAccess;
    hub.borrow_mut()
        .emit(
            &raw_target,
            ProducerEvent::live(EventKind::CallEnter).with_data("function", "example/entry"),
            &mut access,
        )
        .expect("event delivery");
    assert_eq!(service.queued_event_count(&instrument).unwrap(), 1);
    let batch = service.drain_events(&instrument).unwrap();
    assert_eq!(batch.events.len(), 1);
    assert_eq!(
        batch.events[0].envelope.data,
        BTreeMap::from([("function".into(), "example/entry".into())])
    );

    service.detach(&instrument).expect("trusted detach");
    assert!(matches!(
        service.drain_events(&instrument),
        Err(NativeInstrumentationError::Hub(
            InstrumentationError::StaleInstrumentHandle { .. }
        ))
    ));
}

#[test]
fn trusted_native_controller_owns_one_lease_and_issues_safepoint_directives() {
    let (hub, service) = service("session");
    let first = service
        .register(control_registration("debugger-a", "session"))
        .expect("first controller");
    let second = service
        .register(control_registration("debugger-b", "session"))
        .expect("second controller");
    let raw_target = hub
        .borrow_mut()
        .register_target(target(
            "execution",
            "session",
            [Capability::ControlSingleStep],
        ))
        .expect("target registration");
    let target = service.bind_target(&raw_target).expect("bind target");
    service.attach(&first, &target).expect("first attachment");
    service.attach(&second, &target).expect("second attachment");

    let lease = service
        .acquire_control(&first, &target)
        .expect("first lease");
    assert_eq!(lease.instrument_id(), "debugger-a");
    assert_eq!(lease.target_id(), "execution");
    assert!(matches!(
        service.acquire_control(&second, &target),
        Err(NativeInstrumentationError::Hub(
            InstrumentationError::ControlLeaseHeld {
                target_id,
                holder,
            }
        )) if target_id == "execution" && holder == "debugger-a"
    ));

    service
        .request_directive(&lease, InstrumentDirective::StepNext)
        .expect("step directive");
    assert_eq!(
        hub.borrow_mut().take_directive(&raw_target).unwrap(),
        InstrumentDirective::StepNext
    );
    service.release_control(&lease).expect("lease release");
    service
        .acquire_control(&second, &target)
        .expect("released lease can be reacquired");
}

#[test]
fn trusted_native_unsupported_capabilities_include_exact_provider_evidence() {
    let (hub, service) = service("session");
    let instrument = service
        .register(passive_registration("trace", "session"))
        .expect("trusted registration");
    let target = hub
        .borrow_mut()
        .register_target(target("execution", "session", []))
        .expect("target registration");
    let target = service.bind_target(&target).expect("bind target");

    assert!(matches!(
        service.attach(&instrument, &target),
        Err(NativeInstrumentationError::UnsupportedCapabilities {
            target_id,
            backend,
            requested,
            potential,
            missing,
        }) if target_id == "execution"
            && backend == RuntimeBackend::new("rust").unwrap()
            && requested == set([Capability::EventCall])
            && potential.is_empty()
            && missing == set([Capability::EventCall])
    ));
}

#[test]
fn trusted_native_rejects_cross_runtime_and_cross_session_handles() {
    let (_hub_a, service_a) = service("session-a");
    let (_hub_b, service_b) = service("session-b");
    let instrument = service_a
        .register(passive_registration("trace", "session-a"))
        .expect("registration");
    assert!(matches!(
        service_b.detach(&instrument),
        Err(NativeInstrumentationError::CrossRuntimeHandle { kind: "instrument" })
    ));

    let shared = Rc::new(RefCell::new(InstrumentationHub::new()));
    let session_a = NativeInstrumentation::new("session-a", shared.clone());
    let session_b = NativeInstrumentation::new("session-b", shared.clone());
    let instrument = session_a
        .register(passive_registration("shared", "session-a"))
        .expect("registration");
    assert_eq!(
        session_b.detach(&instrument),
        Err(NativeInstrumentationError::CrossSessionHandle {
            kind: "instrument",
            expected: "session-b".into(),
            actual: "session-a".into(),
        })
    );
}

#[test]
fn trusted_native_stale_and_forged_handles_fail_closed() {
    let (_hub, service) = service("session");
    let original = service
        .register(passive_registration("trace", "session"))
        .expect("original registration");
    service.detach(&original).expect("detach original");
    let replacement = service
        .register(passive_registration("trace", "session"))
        .expect("replacement registration");
    assert_eq!(original.generation(), 0);
    assert_eq!(replacement.generation(), 1);
    assert!(matches!(
        service.detach(&original),
        Err(NativeInstrumentationError::Hub(
            InstrumentationError::StaleInstrumentHandle {
                instrument_id,
                generation: 0,
            }
        )) if instrument_id == "trace"
    ));

    let forged = NativeInstrumentHandle {
        session_id: "session".into(),
        hub: service.hub.clone(),
        handle: InstrumentHandle::new("forged".into(), 77),
    };
    assert_eq!(
        service.detach(&forged),
        Err(NativeInstrumentationError::Hub(
            InstrumentationError::UnknownInstrument("forged".into())
        ))
    );

    let forged_target = TargetHandle::new("forged-target".into(), 4);
    assert!(matches!(
        service.bind_target(&forged_target),
        Err(NativeInstrumentationError::Hub(
            InstrumentationError::UnknownTarget(target_id)
        )) if target_id == "forged-target"
    ));
}

#[test]
fn trusted_native_transform_and_unbound_callback_requests_fail_explicitly() {
    let (_hub, service) = service("session");
    let mut transform = passive_registration("transform", "session");
    transform.mode = InstrumentMode::Transform;
    transform.capabilities = BTreeSet::new();
    transform.events = BTreeSet::new();
    assert!(matches!(
        service.register(transform),
        Err(NativeInstrumentationError::UnsupportedMode(
            InstrumentMode::Transform
        ))
    ));

    let mut callback = passive_registration("callback", "session");
    callback.delivery = EventDelivery::Callback;
    assert!(matches!(
        service.register(callback),
        Err(NativeInstrumentationError::UnsupportedDelivery(
            "callback binding is not available in this tranche"
        ))
    ));
}

#[test]
fn trusted_native_service_is_invalidated_by_session_runtime_shutdown() {
    let mut kernel = SessionKernel::new();
    let session_id = SessionId::parse("trusted-native").unwrap();
    kernel.create_session(session_id.clone()).unwrap();
    let service = kernel
        .instrumentation(&session_id)
        .expect("trusted host service");
    service
        .register(passive_registration("trace", session_id.as_str()))
        .expect("registration");
    assert!(service.is_active());

    kernel.close_session(&session_id).expect("session close");
    assert!(!service.is_active());
    assert!(matches!(
        service.register(passive_registration("late", session_id.as_str())),
        Err(NativeInstrumentationError::RuntimeClosed { session_id: closed })
            if closed == session_id.as_str()
    ));
}

#[test]
fn trusted_native_service_never_enters_hara_or_sandbox_authority() {
    let mut kernel = SessionKernel::new();
    let session_id = SessionId::parse("ordinary").unwrap();
    kernel.create_session(session_id.clone()).unwrap();
    let service = kernel
        .instrumentation(&session_id)
        .expect("trusted embedding service");
    assert_eq!(service.session_id(), "ordinary");
    let runtime = kernel.session(&session_id).unwrap().runtime().unwrap();
    assert!(runtime
        .namespace_registry
        .find("std.native.Instrumentation")
        .is_none());

    let sandbox = crate::Runtime::sandbox();
    assert!(sandbox
        .namespace_registry
        .find("std.native.Instrumentation")
        .is_none());
}
