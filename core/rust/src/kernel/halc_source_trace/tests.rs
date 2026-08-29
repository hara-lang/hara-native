use super::*;
use crate::kernel::halc_trace::{
    HalcArtifactTrace, HalcTraceEvent, HalcTraceStatus, HalcTraceValue,
};

const SOURCE: &str = "(ns demo.schema) \
                     (def Customer [:map [:id :int]]) \
                     (defn ^{:schema #'-/Customer} customer-id [customer] customer)";

fn stage<'a>(trace: &'a HalcArtifactTrace, name: &str) -> &'a HalcTraceEvent {
    trace
        .events
        .iter()
        .find(|event| event.stage == name)
        .unwrap_or_else(|| panic!("missing stage {name}"))
}

fn evidence_strings<'a>(event: &'a HalcTraceEvent, key: &str) -> &'a [String] {
    match event.evidence.get(key) {
        Some(HalcTraceValue::Strings(values)) => values,
        value => panic!("expected string vector at {key}, got {value:?}"),
    }
}

#[test]
fn traces_source_through_the_production_encoder_and_decoder() {
    let trace = trace_halc_source("source-trace-1", "demo.schema", "demo/schema.hal", SOURCE);
    assert_eq!(trace.schema, HALC_TRACE_SCHEMA);
    assert_eq!(trace.status, HalcTraceStatus::Ok);
    assert_eq!(
        trace
            .events
            .iter()
            .map(|event| event.stage)
            .collect::<Vec<_>>(),
        vec![
            "source/read",
            "module/identity",
            "forms/canonicalize",
            "schema/index",
            "payload/encode",
            "envelope/build",
            "artifact/validate",
            "artifact/decode",
        ]
    );
    assert_eq!(
        trace.result.as_ref().unwrap().get("decode/parity"),
        Some(&HalcTraceValue::Boolean(true))
    );
    assert_eq!(
        trace.result.as_ref().unwrap().get("module/namespace"),
        Some(&HalcTraceValue::String("demo.schema".to_owned()))
    );
}

#[test]
fn exposes_canonical_forms_and_resolved_schema_types_as_bounded_data() {
    let trace = trace_halc_source("schema-trace", "demo.schema", "demo/schema.hal", SOURCE);
    let forms = evidence_strings(stage(&trace, "forms/canonicalize"), "forms/structural");
    assert!(forms
        .iter()
        .any(|form| form.contains("demo.schema/Customer")));

    let schemas = stage(&trace, "schema/index");
    assert_eq!(
        schemas.evidence.get("schema/definitions"),
        Some(&HalcTraceValue::Strings(vec![
            "demo.schema/Customer".to_owned()
        ]))
    );
    assert_eq!(
        schemas.evidence.get("schema/functions"),
        Some(&HalcTraceValue::Strings(vec![
            "demo.schema/customer-id".to_owned()
        ]))
    );
    assert!(evidence_strings(schemas, "schema/resolved-functions")
        .iter()
        .any(|schema| schema.starts_with("demo.schema/customer-id=")));
    assert_eq!(
        schemas.evidence.get("schema/truncated"),
        Some(&HalcTraceValue::Boolean(false))
    );
}

#[test]
fn bounds_forms_schema_entries_and_artifact_previews() {
    let trace = trace_halc_source_with_limits(
        "bounded",
        "demo.schema",
        "demo/schema.hal",
        SOURCE,
        HalcSourceTraceLimits {
            max_forms: 1,
            max_schema_entries: 1,
            max_text_bytes: 16,
            max_artifact_preview_bytes: 4,
        },
    );
    assert_eq!(trace.status, HalcTraceStatus::Ok);
    assert_eq!(
        stage(&trace, "forms/canonicalize")
            .evidence
            .get("forms/truncated"),
        Some(&HalcTraceValue::Boolean(true))
    );
    assert_eq!(
        stage(&trace, "envelope/build")
            .evidence
            .get("artifact/preview-bytes"),
        Some(&HalcTraceValue::Integer(4))
    );
    assert_eq!(
        stage(&trace, "envelope/build")
            .evidence
            .get("artifact/preview-truncated"),
        Some(&HalcTraceValue::Boolean(true))
    );
}

#[test]
fn source_parse_failures_stop_at_a_typed_source_stage() {
    let trace = trace_halc_source("bad-source", "demo.bad", "demo/bad.hal", "(");
    assert_eq!(trace.status, HalcTraceStatus::Error);
    assert_eq!(trace.events.len(), 1);
    assert_eq!(trace.events[0].stage, "source/read");
    assert_eq!(trace.events[0].status, HalcTraceStatus::Error);
    assert_eq!(
        trace.events[0].evidence.get("diagnostic/category"),
        Some(&HalcTraceValue::String("source/parse".to_owned()))
    );
    assert!(trace.error.is_some());
}

#[test]
fn source_tracing_is_deterministic_and_never_evaluates_forms() {
    let source = "(ns demo.noeval) (throw \"must not execute\")";
    let first = trace_halc_source("no-evaluation", "demo.noeval", "demo/noeval.hal", source);
    let second = trace_halc_source("no-evaluation", "demo.noeval", "demo/noeval.hal", source);
    assert_eq!(first, second);
    assert_eq!(first.status, HalcTraceStatus::Ok);
    assert_eq!(
        first.result.as_ref().unwrap().get("decode/parity"),
        Some(&HalcTraceValue::Boolean(true))
    );
}
