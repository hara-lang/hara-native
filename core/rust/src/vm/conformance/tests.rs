use super::*;
use crate::journal::SCHEMA as JOURNAL_SCHEMA;
use crate::kernel::halc_trace::HALC_TRACE_SCHEMA;
use crate::vm::session::BYTECODE_TRACE_SCHEMA;
use sha2::{Digest, Sha256};

const HCC_MAGIC: &[u8; 4] = b"HCC0";
const HCC_ARTIFACT: &[u8] = include_bytes!("../../../assets/bytecode-conformance.hcc");

struct HccCase<'a> {
    id: &'a str,
    expected_display: &'a str,
    artifact: &'a [u8],
}

struct HccReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> HccReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn u32(&mut self) -> Result<usize, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")) as usize)
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = self.u32()?;
        self.take(length)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "HCC0 field length overflows the artifact".to_owned())?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "HCC0 field exceeds the artifact".to_owned())?;
        self.offset = end;
        Ok(value)
    }

    fn finish(&self) -> Result<(), String> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err("HCC0 artifact has trailing bytes".into())
        }
    }
}

fn parse_hcc(bytes: &[u8]) -> Result<Vec<HccCase<'_>>, String> {
    if !bytes.starts_with(HCC_MAGIC) {
        return Err("HCC0 artifact has invalid magic".into());
    }
    let body = bytes
        .get(4..)
        .ok_or_else(|| "HCC0 artifact is truncated before its payload".to_owned())?;
    if body.len() < 32 {
        return Err("HCC0 artifact is truncated before its payload".into());
    }
    let (digest, payload) = body.split_at(32);
    let expected: [u8; 32] = digest.try_into().expect("HCC0 digest has 32 bytes");
    let actual: [u8; 32] = Sha256::digest(payload).into();
    if actual != expected {
        return Err("HCC0 artifact checksum mismatch".into());
    }
    let mut reader = HccReader::new(payload);
    let count = reader.u32()?;
    if count > reader.bytes.len() / 12 {
        return Err("HCC0 case count exceeds its payload".into());
    }
    let mut cases = Vec::with_capacity(count);
    for _ in 0..count {
        let id = std::str::from_utf8(reader.bytes()?)
            .map_err(|_| "HCC0 case id is not UTF-8")?;
        let expected_display = std::str::from_utf8(reader.bytes()?)
            .map_err(|_| "HCC0 expected display is not UTF-8")?;
        let artifact = reader.bytes()?;
        cases.push(HccCase {
            id,
            expected_display,
            artifact,
        });
    }
    reader.finish()?;
    Ok(cases)
}

fn requires_mounted_foundation_package(program: &crate::vm::Program) -> bool {
    program.constants.iter().any(|value| {
        value
            .display()
            .strip_prefix("std.foundation/")
            .is_some_and(|method| !method.is_empty() && !method.starts_with('/'))
    })
}

fn case<'a>(report: &'a ProductionReport, id: &str) -> &'a CaseObservation {
    report
        .cases
        .iter()
        .find(|case| case.case.id == id)
        .unwrap_or_else(|| panic!("missing case {id}"))
}

#[test]
fn embedded_production_corpus_passes_real_runtime_checks() {
    let report = run_embedded().expect("production corpus runs");
    assert!(report.passed(), "{} failed checks", report.failed_checks());
    assert_eq!(report.cases.len(), 14);
    assert!(report
        .cases
        .iter()
        .all(|case| case.journal.schema == JOURNAL_SCHEMA));
    assert!(report
        .cases
        .iter()
        .all(|case| case.halc_trace.schema == HALC_TRACE_SCHEMA));
}

#[test]
fn arithmetic_fixture_carries_all_three_production_traces() {
    let report = run_embedded().expect("production corpus runs");
    let arithmetic = case(&report, "arith/nested");
    assert_eq!(arithmetic.interpreter.display.as_deref(), Some("7"));
    assert_eq!(arithmetic.halc.handoff_status.as_deref(), Some("ready"));
    assert_eq!(arithmetic.bytecode.outcome.display.as_deref(), Some("7"));
    assert!(arithmetic
        .halc_trace
        .events
        .iter()
        .any(|event| { event.stage == "handoff/bytecode" }));
    let trace_json = crate::json::write(&arithmetic.bytecode_trace).unwrap();
    assert!(trace_json.contains(BYTECODE_TRACE_SCHEMA));
    assert!(trace_json.contains("code.vm/arith/nested"));
    assert!(arithmetic
        .teaching
        .iter()
        .any(|annotation| annotation.stage == "bytecode"));
}

#[test]
fn deep_fixture_is_bounded_without_changing_its_result() {
    let report = run_embedded().expect("production corpus runs");
    let looping = case(&report, "loop/many-iterations");
    assert_eq!(looping.bytecode.outcome.display.as_deref(), Some("1024"));
    assert!(looping.bytecode.step_count <= looping.case.trace_limit);
    assert!(looping.bytecode.dropped > 0);
    assert!(looping
        .checks
        .iter()
        .find(|check| check.id == "trace/bounded")
        .is_some_and(|check| check.pass));
}

#[test]
fn multi_arity_function_is_compiled_dispatched_and_never_falls_back() {
    let report = run_embedded().expect("production corpus runs");
    let multi_arity = case(&report, "compile/fn-multi-arity");
    assert_eq!(multi_arity.bytecode.outcome.status, "returned");
    assert_eq!(
        multi_arity.bytecode.outcome.display.as_deref(),
        Some("42")
    );
    assert_eq!(multi_arity.halc.status, "ok");
    assert_eq!(multi_arity.halc.fallback, Some(false));
    assert!(multi_arity
        .checks
        .iter()
        .find(|check| check.id == "fallback/forbidden")
        .is_some_and(|check| check.pass));
}

#[test]
fn reports_are_deterministic_and_browser_view_is_terminal_neutral() {
    let first = run_embedded().expect("first run");
    let second = run_embedded().expect("second run");
    assert_eq!(
        first.to_json(false).unwrap(),
        second.to_json(false).unwrap()
    );

    let browser = first.browser_json(false).unwrap();
    assert!(browser.contains("\"view\":\"browser\""));
    assert!(browser.contains("\"terminalNeutral\":true"));
    assert!(browser.contains("\"browserSafe\":true"));
    assert!(browser.contains("\"supported\":false"));
    assert!(!browser.contains("loop/many-iterations"));
}

#[test]
fn core_runtime_executes_every_source_free_hcc_success_case_serially() {
    let cases = parse_hcc(HCC_ARTIFACT).expect("embedded HCC0 corpus is valid");
    assert!(cases.len() >= 80, "HCC0 corpus has only {} cases", cases.len());

    let mut runtime = crate::Runtime::core();
    let mut executed = 0;
    let mut failure_ownership_required = 0;
    for case in &cases {
        if case.id.starts_with("error/") {
            failure_ownership_required += 1;
            continue;
        }
        let program = crate::vm::decode_program(case.artifact)
            .unwrap_or_else(|error| panic!("{}: invalid HBC0 artifact: {error}", case.id));
        assert!(
            !requires_mounted_foundation_package(&program),
            "{} must not require a Foundation package",
            case.id
        );
        let actual = runtime
            .eval_bytecode_artifact(case.artifact)
            .unwrap_or_else(|error| panic!("{}: bytecode execution failed: {error}", case.id));
        assert_eq!(actual, case.expected_display, "{} display", case.id);
        executed += 1;
    }
    assert_eq!(
        executed,
        cases.len() - failure_ownership_required,
        "only failure-ownership vectors may be held outside the source-free success lane"
    );
    assert!(
        failure_ownership_required > 0,
        "the corpus must retain failure-ownership HBC0 vectors"
    );
}
