//! Production-generated conformance evidence for the portable `code.vm` stack.
//!
//! Every fixture runs through the real Evaluation Journal collector, HALC
//! encoder/decoder and bytecode observation session. The report contains only
//! bounded JSON-safe data so the same library surface can be compiled for the
//! browser and consumed by Hodos without terminal coupling.

#[path = "conformance/corpus.rs"]
mod corpus;
#[path = "conformance/document.rs"]
mod document;
#[path = "conformance/inspect.rs"]
mod inspect;

pub use corpus::{
    parse_corpus, validate_upstream, Corpus, CorpusCase, ExpectedOutcome, CORPUS_SCHEMA,
};
pub use inspect::normalize_error_category;

use inspect::{
    compile_error_category, evidence_bool, evidence_string, expected_text,
    normalize_compile_marker, outcome_text, teaching_annotations, trace_evidence_string,
    trace_sequences_contiguous, trace_steps, trace_string, trace_usize,
};

use crate::core::Value;
use crate::journal::{Journal, JournalStatus};
use crate::kernel::halc_bytecode_trace::trace_halc_source_to_bytecode;
use crate::kernel::halc_trace::{HalcArtifactTrace, HalcTraceValue};
use crate::Runtime;

use super::{
    compile_source_with, BytecodeObservationSession, BytecodeSessionStatus, SessionRetentionLimits,
};

pub const REPORT_SCHEMA: &str = "hal.code-vm-conformance-runtime/0-alpha";
pub const EMBEDDED_CORPUS: &str = include_str!("../../assets/code-vm-conformance.edn");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageOutcome {
    pub status: String,
    pub display: Option<String>,
    pub category: Option<String>,
    pub message: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    pub pass: bool,
    pub expected: String,
    pub actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HalcSummary {
    pub status: String,
    pub decode_parity: Option<bool>,
    pub handoff_status: Option<String>,
    pub handoff_category: Option<String>,
    pub fallback: Option<bool>,
    pub namespace: Option<String>,
    pub resource: Option<String>,
    pub source_hash: Option<String>,
    pub event_count: usize,
    pub sequences_contiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeSummary {
    pub outcome: StageOutcome,
    pub source_id: Option<String>,
    pub step_count: usize,
    pub dropped: usize,
    pub sequences_contiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeachingAnnotation {
    pub concept: String,
    pub stage: String,
    pub sequence: usize,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaseObservation {
    pub case: CorpusCase,
    pub interpreter: StageOutcome,
    pub interpreter_required: bool,
    pub journal: Journal,
    pub halc: HalcSummary,
    pub halc_trace: HalcArtifactTrace,
    pub bytecode: BytecodeSummary,
    pub bytecode_trace: Value,
    pub checks: Vec<Check>,
    pub teaching: Vec<TeachingAnnotation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionReport {
    pub corpus: Corpus,
    pub cases: Vec<CaseObservation>,
}

impl ProductionReport {
    pub fn passed(&self) -> bool {
        self.cases
            .iter()
            .flat_map(|case| case.checks.iter())
            .all(|check| check.pass)
    }

    pub fn failed_checks(&self) -> usize {
        self.cases
            .iter()
            .flat_map(|case| case.checks.iter())
            .filter(|check| !check.pass)
            .count()
    }

    pub fn to_value(&self) -> Value {
        document::report_value(self, false)
    }

    pub fn browser_value(&self) -> Value {
        document::report_value(self, true)
    }

    pub fn to_json(&self, pretty: bool) -> Result<String, String> {
        let value = self.to_value();
        if pretty {
            crate::json::write_pretty(&value)
        } else {
            crate::json::write(&value)
        }
    }

    pub fn browser_json(&self, pretty: bool) -> Result<String, String> {
        let value = self.browser_value();
        if pretty {
            crate::json::write_pretty(&value)
        } else {
            crate::json::write(&value)
        }
    }
}

pub fn run_embedded() -> Result<ProductionReport, String> {
    run_corpus(EMBEDDED_CORPUS)
}

pub fn run_corpus(source: &str) -> Result<ProductionReport, String> {
    let corpus = corpus::parse_corpus(source)?;
    let cases = corpus
        .cases
        .iter()
        .map(observe_case)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProductionReport { corpus, cases })
}

fn observe_case(case: &CorpusCase) -> Result<CaseObservation, String> {
    let (journal, interpreter) = observe_interpreter(case);
    let halc_trace = trace_halc_source_to_bytecode(
        format!("code-vm/{}", case.id),
        &case.namespace,
        &case.resource,
        &case.source,
    );
    let halc = summarize_halc(&halc_trace);
    let (bytecode, bytecode_trace) = observe_bytecode(case)?;
    let checks = checks(case, &interpreter, &journal, &halc, &bytecode);
    let teaching = teaching_annotations(&journal, &halc_trace, &bytecode_trace);
    Ok(CaseObservation {
        case: case.clone(),
        interpreter,
        interpreter_required: case.interpreter_required,
        journal,
        halc,
        halc_trace,
        bytecode,
        bytecode_trace,
        checks,
        teaching,
    })
}

fn observe_interpreter(case: &CorpusCase) -> (Journal, StageOutcome) {
    let mut runtime = Runtime::new();
    let journal = runtime.eval_native_journal(&case.source);
    let truncated = journal.status == JournalStatus::Truncated;
    let outcome = if let Some(result) = journal.result.as_ref() {
        StageOutcome {
            status: "returned".into(),
            display: Some(result.display.clone()),
            category: None,
            message: None,
            truncated,
        }
    } else {
        let message = journal.error.clone().unwrap_or_default();
        StageOutcome {
            status: "failed".into(),
            display: None,
            category: Some(normalize_error_category(&message).into()),
            message: Some(message),
            truncated,
        }
    };
    (journal, outcome)
}

fn observe_bytecode(case: &CorpusCase) -> Result<(BytecodeSummary, Value), String> {
    let registry = crate::embedding_namespace_registry();
    let program = match compile_source_with(&case.source, &registry) {
        Ok(program) => program,
        Err(error) => {
            let outcome = StageOutcome {
                status: "compile-error".into(),
                display: None,
                category: Some(compile_error_category(error.kind()).into()),
                message: Some(error.to_string()),
                truncated: false,
            };
            return Ok((
                BytecodeSummary {
                    outcome,
                    source_id: None,
                    step_count: 0,
                    dropped: 0,
                    sequences_contiguous: true,
                },
                Value::Nil,
            ));
        }
    };
    let mut session = BytecodeObservationSession::from_program(
        format!("code-vm/{}", case.id),
        &case.source_id,
        program,
    )
    .map_err(|error| error.to_string())?;
    session.set_retention_limits(SessionRetentionLimits {
        events: case.trace_limit,
        trace: case.trace_limit,
    });
    session.run(case.steps).map_err(|error| error.to_string())?;
    let outcome = match session.status() {
        BytecodeSessionStatus::Returned => StageOutcome {
            status: "returned".into(),
            display: session.result().map(|value| value.display()),
            category: None,
            message: None,
            truncated: false,
        },
        BytecodeSessionStatus::Failed => {
            let message = session
                .error()
                .map(|error| error.message.clone())
                .unwrap_or_default();
            StageOutcome {
                status: "failed".into(),
                display: None,
                category: Some(normalize_error_category(&message).into()),
                message: Some(message),
                truncated: false,
            }
        }
        status => StageOutcome {
            status: status.as_keyword().into(),
            display: None,
            category: None,
            message: None,
            truncated: false,
        },
    };
    let trace = session.trace();
    let step_count = trace_steps(&trace).len();
    let dropped = trace_usize(&trace, "dropped").unwrap_or(0);
    let source_id = trace_string(&trace, "sourceId");
    let sequences_contiguous = trace_sequences_contiguous(&trace);
    Ok((
        BytecodeSummary {
            outcome,
            source_id,
            step_count,
            dropped,
            sequences_contiguous,
        },
        trace,
    ))
}

fn summarize_halc(trace: &HalcArtifactTrace) -> HalcSummary {
    let result = trace.result.as_ref();
    let handoff = trace
        .events
        .iter()
        .find(|event| event.stage == "handoff/bytecode");
    let failure = trace.events.iter().rev().find_map(|event| {
        event.evidence.get("diagnostic/category").and_then(|value| {
            if let HalcTraceValue::String(value) = value {
                Some(value.clone())
            } else {
                None
            }
        })
    });
    HalcSummary {
        status: trace.status.as_keyword().into(),
        decode_parity: result.and_then(|values| evidence_bool(values, "decode/parity")),
        handoff_status: handoff
            .and_then(|event| evidence_string(&event.evidence, "handoff/status")),
        handoff_category: failure,
        fallback: handoff.and_then(|event| evidence_bool(&event.evidence, "handoff/fallback")),
        namespace: trace_evidence_string(trace, "module/namespace"),
        resource: trace_evidence_string(trace, "module/resource"),
        source_hash: trace_evidence_string(trace, "source/hash"),
        event_count: trace.events.len(),
        sequences_contiguous: trace
            .events
            .iter()
            .enumerate()
            .all(|(index, event)| event.sequence == index as u64 + 1),
    }
}

fn checks(
    case: &CorpusCase,
    interpreter: &StageOutcome,
    journal: &Journal,
    halc: &HalcSummary,
    bytecode: &BytecodeSummary,
) -> Vec<Check> {
    let mut checks = Vec::new();
    checks.push(outcome_check(
        "bytecode/expected",
        &case.expected,
        &bytecode.outcome,
    ));
    checks.push(if case.interpreter_required {
        outcome_check("interpreter/expected", &case.expected, interpreter)
    } else {
        Check {
            id: "interpreter/expected".into(),
            pass: true,
            expected: "not-required-for-compile-only-case".into(),
            actual: interpreter.status.clone(),
        }
    });
    checks.push(runtime_parity_check(case, interpreter, &bytecode.outcome));
    checks.push(halc_check(case, halc));
    let halc_identity = match (halc.namespace.as_deref(), halc.resource.as_deref()) {
        (Some(namespace), Some(resource)) => {
            namespace == case.namespace.as_str() && resource == case.resource.as_str()
        }
        (None, None) => halc.handoff_status.is_none(),
        _ => false,
    };
    checks.push(Check {
        id: "source/identity".into(),
        pass: halc_identity
            && bytecode
                .source_id
                .as_deref()
                .map_or(true, |source_id| source_id == case.source_id.as_str()),
        expected: format!("{}|{}|{}", case.namespace, case.resource, case.source_id),
        actual: format!(
            "{}|{}|{}",
            halc.namespace.as_deref().unwrap_or("none"),
            halc.resource.as_deref().unwrap_or("none"),
            bytecode.source_id.as_deref().unwrap_or("none")
        ),
    });
    let journal_sequence = journal
        .events
        .iter()
        .enumerate()
        .all(|(index, event)| event.sequence == index as u64 + 1);
    checks.push(Check {
        id: "trace/sequences".into(),
        pass: journal_sequence && halc.sequences_contiguous && bytecode.sequences_contiguous,
        expected: "contiguous".into(),
        actual: format!(
            "interpreter={journal_sequence},halc={},bytecode={}",
            halc.sequences_contiguous, bytecode.sequences_contiguous
        ),
    });
    checks.push(Check {
        id: "trace/bounded".into(),
        pass: bytecode.step_count <= case.trace_limit
            && (!case.expect_dropped || bytecode.dropped > 0),
        expected: format!(
            "steps<={},dropped{}0",
            case.trace_limit,
            if case.expect_dropped { ">" } else { ">=" }
        ),
        actual: format!("steps={},dropped={}", bytecode.step_count, bytecode.dropped),
    });
    checks.push(Check {
        id: "fallback/forbidden".into(),
        pass: halc.fallback.map_or(true, |fallback| !fallback),
        expected: "false-or-not-applicable".into(),
        actual: halc
            .fallback
            .map(|value| value.to_string())
            .unwrap_or_else(|| "not-applicable".into()),
    });
    checks
}

fn outcome_check(id: &str, expected: &ExpectedOutcome, actual: &StageOutcome) -> Check {
    let (pass, expected_text) = match expected {
        ExpectedOutcome::Display(display) => (
            actual.status == "returned" && actual.display.as_deref() == Some(display.as_str()),
            format!("returned:{display}"),
        ),
        ExpectedOutcome::ErrorCategory(category) => (
            actual.category.as_deref() == Some(category),
            format!("error-category:{category}"),
        ),
        ExpectedOutcome::CompileError(marker) => (
            actual.status == "compile-error"
                && actual
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains(marker)),
            format!("compile-error:{marker}"),
        ),
    };
    Check {
        id: id.into(),
        pass,
        expected: expected_text,
        actual: outcome_text(actual),
    }
}

fn runtime_parity_check(
    case: &CorpusCase,
    interpreter: &StageOutcome,
    bytecode: &StageOutcome,
) -> Check {
    if !case.interpreter_required || bytecode.status == "compile-error" {
        return Check {
            id: "runtime/parity".into(),
            pass: true,
            expected: "not-required".into(),
            actual: "not-compared".into(),
        };
    }
    let pass = match &case.expected {
        ExpectedOutcome::Display(_) => interpreter.display == bytecode.display,
        ExpectedOutcome::ErrorCategory(_) => interpreter.category == bytecode.category,
        ExpectedOutcome::CompileError(_) => true,
    };
    Check {
        id: "runtime/parity".into(),
        pass,
        expected: outcome_text(interpreter),
        actual: outcome_text(bytecode),
    }
}

fn halc_check(case: &CorpusCase, halc: &HalcSummary) -> Check {
    let pass = match &case.expected {
        ExpectedOutcome::Display(_) => {
            halc.status == "ok"
                && halc.decode_parity == Some(true)
                && halc.handoff_status.as_deref() == Some("ready")
        }
        ExpectedOutcome::ErrorCategory(category) if category != "reader" => {
            halc.status == "ok"
                && halc.decode_parity == Some(true)
                && halc.handoff_status.as_deref() == Some("ready")
        }
        ExpectedOutcome::ErrorCategory(category) if category == "reader" => {
            halc.status == "error" && halc.handoff_status.is_none()
        }
        ExpectedOutcome::CompileError(marker) => {
            halc.status == "error"
                && halc.handoff_category.as_deref().is_some_and(|category| {
                    category.contains(normalize_compile_marker(marker))
                        || marker.contains(normalize_compile_marker(category))
                })
        }
        ExpectedOutcome::ErrorCategory(_) => false,
    };
    Check {
        id: "halc/production-handoff".into(),
        pass,
        expected: expected_text(&case.expected),
        actual: format!(
            "status={},decode={:?},handoff={:?},category={:?}",
            halc.status, halc.decode_parity, halc.handoff_status, halc.handoff_category
        ),
    }
}

#[cfg(test)]
#[path = "conformance/tests.rs"]
mod tests;
