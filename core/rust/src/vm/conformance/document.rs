use crate::core::Value;
use crate::journal::{Journal, JournalEvent, JournalEventKind, JournalStatus, ValuePreview};
use crate::kernel::halc_trace::{
    HalcArtifactTrace, HalcTraceEvent, HalcTraceStatus, HalcTraceValue,
};
use crate::lang::data::{OrderedMap, Vector};

use super::{
    BytecodeSummary, CaseObservation, Check, ExpectedOutcome, HalcSummary, ProductionReport,
    StageOutcome, TeachingAnnotation, REPORT_SCHEMA,
};

pub(super) fn report_value(report: &ProductionReport, browser_only: bool) -> Value {
    let selected = report
        .cases
        .iter()
        .filter(|case| !browser_only || case.case.browser_safe)
        .collect::<Vec<_>>();
    let checks = selected
        .iter()
        .flat_map(|case| case.checks.iter())
        .collect::<Vec<_>>();
    let passed_checks = checks.iter().filter(|check| check.pass).count();
    let failed_checks = checks.len().saturating_sub(passed_checks);
    let status = if failed_checks == 0 {
        "passed"
    } else {
        "failed"
    };

    object([
        ("schema", string(REPORT_SCHEMA)),
        (
            "view",
            string(if browser_only { "browser" } else { "complete" }),
        ),
        ("status", string(status)),
        ("terminalNeutral", Value::Bool(true)),
        (
            "corpus",
            object([
                ("schema", string(super::CORPUS_SCHEMA)),
                ("id", string(&report.corpus.id)),
                ("upstream", string(&report.corpus.upstream)),
            ]),
        ),
        (
            "summary",
            object([
                ("cases", integer(selected.len())),
                ("checks", integer(checks.len())),
                ("passed", integer(passed_checks)),
                ("failed", integer(failed_checks)),
            ]),
        ),
        ("runtimeMatrix", runtime_matrix()),
        ("cases", vector(selected.into_iter().map(case_value))),
    ])
}

fn runtime_matrix() -> Value {
    object([
        (
            "rust",
            object([
                ("supported", Value::Bool(true)),
                ("interpreter", string("production-evaluation-journal")),
                ("halc", string("production-encoder-decoder")),
                ("bytecode", string("production-observation-session")),
            ]),
        ),
        (
            "wasm",
            object([
                ("supported", Value::Bool(true)),
                ("contract", string("same-library-report")),
                ("validation", string("wasm32-ci-compile")),
            ]),
        ),
        (
            "truffle",
            object([
                ("supported", Value::Bool(false)),
                ("status", string("unsupported")),
                ("reason", string("production-corpus-runner-pending-406")),
            ]),
        ),
    ])
}

fn case_value(observation: &CaseObservation) -> Value {
    let passed = observation.checks.iter().all(|check| check.pass);
    object([
        ("id", string(&observation.case.id)),
        ("upstreamId", string(&observation.case.upstream_id)),
        ("sourceId", string(&observation.case.source_id)),
        ("namespace", string(&observation.case.namespace)),
        ("resource", string(&observation.case.resource)),
        ("source", string(&observation.case.source)),
        ("browserSafe", Value::Bool(observation.case.browser_safe)),
        ("expected", expected_value(&observation.case.expected)),
        ("passed", Value::Bool(passed)),
        (
            "stages",
            object([
                (
                    "interpreter",
                    object([
                        ("required", Value::Bool(observation.interpreter_required)),
                        ("outcome", outcome_value(&observation.interpreter)),
                        (
                            "trace",
                            journal_value(&observation.journal, &observation.case.source_id),
                        ),
                    ]),
                ),
                (
                    "halc",
                    object([
                        ("summary", halc_summary_value(&observation.halc)),
                        ("trace", halc_trace_value(&observation.halc_trace)),
                    ]),
                ),
                (
                    "bytecode",
                    object([
                        ("summary", bytecode_summary_value(&observation.bytecode)),
                        ("trace", observation.bytecode_trace.clone()),
                    ]),
                ),
            ]),
        ),
        ("checks", vector(observation.checks.iter().map(check_value))),
        (
            "teaching",
            vector(observation.teaching.iter().map(annotation_value)),
        ),
    ])
}

fn expected_value(expected: &ExpectedOutcome) -> Value {
    match expected {
        ExpectedOutcome::Display(value) => {
            object([("status", string("returned")), ("display", string(value))])
        }
        ExpectedOutcome::ErrorCategory(value) => {
            object([("status", string("error")), ("category", string(value))])
        }
        ExpectedOutcome::CompileError(value) => object([
            ("status", string("compile-error")),
            ("marker", string(value)),
        ]),
    }
}

fn outcome_value(outcome: &StageOutcome) -> Value {
    object([
        ("status", string(&outcome.status)),
        ("display", optional_string(outcome.display.as_deref())),
        ("category", optional_string(outcome.category.as_deref())),
        ("message", optional_string(outcome.message.as_deref())),
        ("truncated", Value::Bool(outcome.truncated)),
    ])
}

fn halc_summary_value(summary: &HalcSummary) -> Value {
    object([
        ("status", string(&summary.status)),
        (
            "decodeParity",
            summary.decode_parity.map(Value::Bool).unwrap_or(Value::Nil),
        ),
        (
            "handoffStatus",
            optional_string(summary.handoff_status.as_deref()),
        ),
        (
            "handoffCategory",
            optional_string(summary.handoff_category.as_deref()),
        ),
        (
            "fallback",
            summary.fallback.map(Value::Bool).unwrap_or(Value::Nil),
        ),
        ("namespace", optional_string(summary.namespace.as_deref())),
        ("resource", optional_string(summary.resource.as_deref())),
        (
            "sourceHash",
            optional_string(summary.source_hash.as_deref()),
        ),
        ("events", integer(summary.event_count)),
        (
            "sequencesContiguous",
            Value::Bool(summary.sequences_contiguous),
        ),
    ])
}

fn bytecode_summary_value(summary: &BytecodeSummary) -> Value {
    object([
        ("outcome", outcome_value(&summary.outcome)),
        ("sourceId", optional_string(summary.source_id.as_deref())),
        ("steps", integer(summary.step_count)),
        ("dropped", integer(summary.dropped)),
        (
            "sequencesContiguous",
            Value::Bool(summary.sequences_contiguous),
        ),
    ])
}

fn check_value(check: &Check) -> Value {
    object([
        ("id", string(&check.id)),
        ("pass", Value::Bool(check.pass)),
        ("expected", string(&check.expected)),
        ("actual", string(&check.actual)),
    ])
}

fn annotation_value(annotation: &TeachingAnnotation) -> Value {
    object([
        ("concept", string(&annotation.concept)),
        ("stage", string(&annotation.stage)),
        ("sequence", integer(annotation.sequence)),
        ("detail", string(&annotation.detail)),
    ])
}

fn journal_value(journal: &Journal, source_id: &str) -> Value {
    object([
        ("schema", string(journal.schema)),
        ("id", string(journal.journal_id.to_string())),
        ("sourceId", string(source_id)),
        ("status", string(journal_status(&journal.status))),
        (
            "events",
            vector(journal.events.iter().map(journal_event_value)),
        ),
        (
            "result",
            journal
                .result
                .as_ref()
                .map(preview_value)
                .unwrap_or(Value::Nil),
        ),
        ("error", optional_string(journal.error.as_deref())),
    ])
}

fn journal_event_value(event: &JournalEvent) -> Value {
    object([
        ("id", integer_u64(event.id.0)),
        ("sequence", integer_u64(event.sequence)),
        ("kind", string(journal_kind(&event.kind))),
        (
            "operation",
            event
                .operation
                .map(|value| integer_u64(value.0))
                .unwrap_or(Value::Nil),
        ),
        (
            "parentOperation",
            event
                .parent_operation
                .map(|value| integer_u64(value.0))
                .unwrap_or(Value::Nil),
        ),
        ("depth", integer(event.depth)),
        ("function", optional_string(event.function.as_deref())),
        ("values", vector(event.values.iter().map(preview_value))),
        ("message", optional_string(event.message.as_deref())),
    ])
}

fn preview_value(preview: &ValuePreview) -> Value {
    object([
        ("type", string(&preview.type_name)),
        ("display", string(&preview.display)),
        ("truncated", Value::Bool(preview.truncated)),
    ])
}

fn halc_trace_value(trace: &HalcArtifactTrace) -> Value {
    object([
        ("schema", string(trace.schema)),
        ("id", string(&trace.id)),
        ("status", string(halc_status(trace.status))),
        ("events", vector(trace.events.iter().map(halc_event_value))),
        (
            "result",
            trace
                .result
                .as_ref()
                .map(evidence_value)
                .unwrap_or(Value::Nil),
        ),
        ("error", optional_string(trace.error.as_deref())),
    ])
}

fn halc_event_value(event: &HalcTraceEvent) -> Value {
    object([
        ("id", integer_u64(event.id)),
        ("sequence", integer_u64(event.sequence)),
        ("stage", string(event.stage)),
        ("status", string(halc_status(event.status))),
        ("evidence", evidence_value(&event.evidence)),
        ("error", optional_string(event.error.as_deref())),
    ])
}

fn evidence_value(evidence: &std::collections::BTreeMap<String, HalcTraceValue>) -> Value {
    object(evidence.iter().map(|(key, value)| {
        let value = match value {
            HalcTraceValue::String(value) => string(value),
            HalcTraceValue::Integer(value) => integer_u64(*value),
            HalcTraceValue::Boolean(value) => Value::Bool(*value),
            HalcTraceValue::Strings(values) => {
                vector(values.iter().map(|value| string(value.as_str())))
            }
        };
        (key.clone(), value)
    }))
}

fn journal_status(status: &JournalStatus) -> &'static str {
    match status {
        JournalStatus::Ok => "ok",
        JournalStatus::Error => "error",
        JournalStatus::Truncated => "truncated",
    }
}

fn journal_kind(kind: &JournalEventKind) -> &'static str {
    match kind {
        JournalEventKind::EvaluationStart => "evaluation/start",
        JournalEventKind::MacroExpand => "macro/expand",
        JournalEventKind::OperationEnter => "operation/enter",
        JournalEventKind::OperationReturn => "operation/return",
        JournalEventKind::Error => "evaluation/error",
        JournalEventKind::JournalTruncated => "journal/truncated",
    }
}

fn halc_status(status: HalcTraceStatus) -> &'static str {
    match status {
        HalcTraceStatus::Ok => "ok",
        HalcTraceStatus::Error => "error",
    }
}

fn object<I, K>(fields: I) -> Value
where
    I: IntoIterator<Item = (K, Value)>,
    K: Into<String>,
{
    Value::OrderedMap(Box::new(OrderedMap::from_iter(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.into()), value)),
    )))
}

fn vector<I>(values: I) -> Value
where
    I: IntoIterator<Item = Value>,
{
    Value::Vector(Vector::from_iter(values))
}

fn string(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

fn optional_string(value: Option<&str>) -> Value {
    value.map(string).unwrap_or(Value::Nil)
}

fn integer(value: usize) -> Value {
    Value::Number(i64::try_from(value).unwrap_or(i64::MAX))
}

fn integer_u64(value: u64) -> Value {
    Value::Number(i64::try_from(value).unwrap_or(i64::MAX))
}
