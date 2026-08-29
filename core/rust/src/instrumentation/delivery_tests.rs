use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn target(capabilities: impl IntoIterator<Item = Capability>) -> TargetDescriptor {
    TargetDescriptor {
        target_id: "execution".into(),
        session_id: "session".into(),
        kind: TargetKind::Interpreter,
        backend: RuntimeBackend::new("rust").expect("valid backend"),
        capabilities: set(capabilities),
    }
}

fn passive(
    id: &str,
    delivery: EventDelivery,
    projection: ProjectionRequest,
) -> InstrumentRegistration {
    let mut capabilities = set([Capability::EventCall]);
    capabilities.extend(projection.required_capabilities());
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: "session".into(),
        mode: InstrumentMode::Passive,
        capabilities,
        events: set([EventKind::CallEnter]),
        filter: InstrumentFilter::default(),
        projection,
        delivery,
    }
}

#[derive(Default)]
struct CountingAccess {
    locations: usize,
    locals: usize,
}

impl EventAccess for CountingAccess {
    fn source_location(&mut self) -> Option<EventLocation> {
        self.locations += 1;
        Some(EventLocation {
            source_id: Some("editor/main".into()),
            form_path: Some(vec![0, 1]),
            ..EventLocation::default()
        })
    }

    fn locals(&mut self, limits: ProjectionLimits) -> Option<PortableProjection> {
        self.locals += 1;
        Some(
            PortableProjection::new("interpreter/locals")
                .with_field("max-items", limits.max_items.to_string()),
        )
    }
}

#[test]
fn unmatched_events_do_not_enter_lazy_access() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(target([Capability::EventCall]))
        .expect("target");
    let mut access = CountingAccess::default();

    let report = hub
        .emit(
            &target,
            ProducerEvent::live(EventKind::CallEnter),
            &mut access,
        )
        .expect("empty delivery is valid");

    assert_eq!(report, DispatchReport::default());
    assert_eq!(access.locations, 0);
    assert_eq!(access.locals, 0);
}

#[test]
fn matching_projection_is_materialized_only_after_attachment() {
    let mut hub = InstrumentationHub::new();
    let projection = ProjectionRequest {
        source_location: true,
        locals: Some(ProjectionLimits {
            max_items: 7,
            ..ProjectionLimits::default()
        }),
        ..ProjectionRequest::default()
    };
    let instrument = hub
        .register(passive("trace", EventDelivery::Callback, projection))
        .expect("registration");
    let target = hub
        .register_target(target([
            Capability::EventCall,
            Capability::InspectSourceLocation,
            Capability::InspectLocals,
        ]))
        .expect("target");
    hub.attach(&instrument, &target).expect("attachment");
    let mut access = CountingAccess::default();

    let report = hub
        .emit(
            &target,
            ProducerEvent::live(EventKind::CallEnter).with_data("function", "calculate"),
            &mut access,
        )
        .expect("delivery");

    assert_eq!(access.locations, 1);
    assert_eq!(access.locals, 1);
    assert_eq!(report.callbacks.len(), 1);
    let event = &report.callbacks[0];
    assert_eq!(event.envelope.sequence, 1);
    assert_eq!(
        event
            .envelope
            .location
            .as_ref()
            .and_then(|location| location.source_id.as_deref()),
        Some("editor/main")
    );
    assert_eq!(
        event
            .projection
            .locals
            .as_ref()
            .and_then(|projection| projection.fields.get("max-items"))
            .map(String::as_str),
        Some("7")
    );
}

#[test]
fn bounded_queues_keep_latest_events_and_report_drops() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(passive(
            "trace",
            EventDelivery::Queue { capacity: 2 },
            ProjectionRequest::default(),
        ))
        .expect("registration");
    let target = hub
        .register_target(target([Capability::EventCall]))
        .expect("target");
    hub.attach(&instrument, &target).expect("attachment");
    let mut access = CountingAccess::default();

    for function in ["one", "two", "three"] {
        hub.emit(
            &target,
            ProducerEvent::live(EventKind::CallEnter).with_data("function", function),
            &mut access,
        )
        .expect("delivery");
    }

    assert_eq!(hub.queued_event_count(&instrument), Ok(2));
    let batch = hub.drain_events(&instrument).expect("queue batch");
    assert_eq!(batch.dropped, 1);
    assert_eq!(batch.events.len(), 2);
    assert_eq!(
        batch
            .events
            .iter()
            .map(|event| event.envelope.sequence)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(batch.events[1].dropped_before, 1);
    assert_eq!(
        batch.events[0].envelope.data,
        BTreeMap::from([("function".into(), "two".into())])
    );
}

#[test]
fn control_directives_are_capability_checked_and_consumed_once() {
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
            delivery: EventDelivery::Queue { capacity: 4 },
        })
        .expect("controller");
    let target = hub
        .register_target(target([
            Capability::EventLifecycle,
            Capability::ControlSingleStep,
        ]))
        .expect("target");
    hub.attach(&controller, &target).expect("attachment");
    let lease = hub
        .acquire_control(&controller, &target)
        .expect("control lease");

    hub.request_directive(&lease, InstrumentDirective::StepNext)
        .expect("single-step is granted");
    assert_eq!(
        hub.take_directive(&target),
        Ok(InstrumentDirective::StepNext)
    );
    assert_eq!(
        hub.take_directive(&target),
        Ok(InstrumentDirective::Continue)
    );
    assert!(matches!(
        hub.request_directive(&lease, InstrumentDirective::Terminate),
        Err(InstrumentationError::UnsupportedCapabilities { missing, .. })
            if missing == set([Capability::ControlTerminate])
    ));
}
