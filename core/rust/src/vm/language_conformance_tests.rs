use sha2::{Digest, Sha256};

const HLC_MAGIC: &[u8; 4] = b"HLC1";
const HLC_ARTIFACT: &[u8] = include_bytes!("../../assets/language-conformance.hlc");
const ERROR_EXPECTATION_PREFIX: &str = "!error:";

#[derive(Debug)]
struct HlcCase<'a> {
    id: &'a str,
    layer: &'a str,
    expected: HlcExpectation<'a>,
    browser_safe: bool,
    source: &'a str,
    artifact: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
enum HlcExpectation<'a> {
    Display(&'a str),
    Error(&'a str),
}

impl<'a> HlcExpectation<'a> {
    fn parse(value: &'a str) -> Self {
        match value.strip_prefix(ERROR_EXPECTATION_PREFIX) {
            Some(category) => Self::Error(category),
            None => Self::Display(value),
        }
    }
}

struct HlcReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> HlcReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn u32(&mut self) -> Result<usize, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")) as usize)
    }

    fn boolean(&mut self) -> Result<bool, String> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("HLC1 boolean field must be zero or one".into()),
        }
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = self.u32()?;
        self.take(length)
    }

    fn text(&mut self, field: &str) -> Result<&'a str, String> {
        std::str::from_utf8(self.bytes()?).map_err(|_| format!("HLC1 {field} is not UTF-8"))
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "HLC1 field length overflows the artifact".to_owned())?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "HLC1 field exceeds the artifact".to_owned())?;
        self.offset = end;
        Ok(value)
    }

    fn finish(&self) -> Result<(), String> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err("HLC1 artifact has trailing bytes".into())
        }
    }
}

fn parse_hlc(bytes: &[u8]) -> Result<Vec<HlcCase<'_>>, String> {
    if !bytes.starts_with(HLC_MAGIC) || bytes.len() < 36 {
        return Err("HLC1 artifact has an invalid or truncated header".into());
    }
    let (digest, payload) = bytes[4..].split_at(32);
    let expected: [u8; 32] = digest.try_into().expect("HLC1 digest has 32 bytes");
    let actual: [u8; 32] = Sha256::digest(payload).into();
    if actual != expected {
        return Err("HLC1 artifact checksum mismatch".into());
    }
    let mut reader = HlcReader::new(payload);
    let count = reader.u32()?;
    if count == 0 || count > reader.bytes.len() / 24 {
        return Err("HLC1 has an invalid case count".into());
    }
    let mut cases = Vec::with_capacity(count);
    for _ in 0..count {
        let id = reader.text("case id")?;
        let layer = reader.text("case layer")?;
        if !matches!(layer, "parser" | "evaluator" | "native-abi") {
            return Err(format!("HLC1 {id} has an invalid layer {layer:?}"));
        }
        let expected = HlcExpectation::parse(reader.text("case expectation")?);
        let browser_safe = reader.boolean()?;
        let source = reader.text("case source")?;
        let artifact = reader.bytes()?;
        cases.push(HlcCase {
            id,
            layer,
            expected,
            browser_safe,
            source,
            artifact,
        });
    }
    reader.finish()?;
    Ok(cases)
}

fn requires_source_library(program: &crate::vm::Program) -> bool {
    program.constants.iter().any(|value| {
        let text = value.display();
        text.starts_with("std.foundation/")
            || text.starts_with("std.foundation.")
            || text.starts_with("std.lib/")
            || text.starts_with("std.lib.")
    })
}

fn normalized_error_category(error: &str) -> Option<&'static str> {
    let error = error.trim_start_matches("error: ");
    if error.contains("division by zero") {
        Some("division by zero")
    } else if error.contains("expects numbers") {
        Some("expects numbers")
    } else {
        None
    }
}

fn assert_outcome(case: &HlcCase<'_>, actual: Result<String, String>) -> Result<(), String> {
    match (case.expected, actual) {
        (HlcExpectation::Display(expected), Ok(actual)) if actual == expected => Ok(()),
        (HlcExpectation::Display(expected), Ok(actual)) => Err(format!(
            "{} ({}) expected display {expected:?}, observed {actual:?}",
            case.id, case.layer
        )),
        (HlcExpectation::Display(expected), Err(error)) => Err(format!(
            "{} ({}) expected display {expected:?}, raised {error}",
            case.id, case.layer
        )),
        (HlcExpectation::Error(expected), Err(error))
            if normalized_error_category(&error) == Some(expected) =>
        {
            Ok(())
        }
        (HlcExpectation::Error(expected), Err(error)) => Err(format!(
            "{} ({}) expected error {expected:?}, observed {error:?} ({:?})",
            case.id,
            case.layer,
            normalized_error_category(&error)
        )),
        (HlcExpectation::Error(expected), Ok(actual)) => Err(format!(
            "{} ({}) expected error {expected:?}, returned {actual:?}",
            case.id, case.layer
        )),
    }
}

#[test]
fn core_runtime_executes_every_hlc1_case_from_rust_produced_hbc0() {
    let cases = parse_hlc(HLC_ARTIFACT).expect("embedded HLC1 corpus is valid");
    assert!(
        cases.len() >= 20,
        "HLC1 corpus must cover functional behavior"
    );
    assert!(cases.iter().any(|case| case.layer == "parser"));
    assert!(cases.iter().any(|case| case.layer == "evaluator"));
    assert!(cases.iter().any(|case| case.layer == "native-abi"));
    assert!(cases.iter().any(|case| case.browser_safe));

    for case in &cases {
        let program = crate::vm::decode_program(case.artifact)
            .unwrap_or_else(|error| panic!("{} has invalid HBC0: {error}", case.id));
        assert_eq!(
            crate::vm::encode_program(&program).unwrap(),
            case.artifact,
            "{} HBC0 must be canonical",
            case.id
        );
        assert!(
            !requires_source_library(&program),
            "{} must not require a source package",
            case.id
        );
        let source_actual = crate::Runtime::core().eval_native(case.source);
        assert_outcome(case, source_actual)
            .unwrap_or_else(|error| panic!("source conformance failure: {error}"));
        let artifact_actual = crate::Runtime::core().eval_bytecode_artifact(case.artifact);
        assert_outcome(case, artifact_actual)
            .unwrap_or_else(|error| panic!("artifact conformance failure: {error}"));
    }
}

#[test]
fn hlc1_rejects_corruption_and_wrong_normalized_errors() {
    let mut corrupted = HLC_ARTIFACT.to_vec();
    corrupted[36] ^= 1;
    assert!(parse_hlc(&corrupted).unwrap_err().contains("checksum"));
    let wrong = HlcCase {
        id: "fixture/error",
        layer: "evaluator",
        expected: HlcExpectation::Error("division by zero"),
        browser_safe: false,
        source: "(/ 1 0)",
        artifact: &[],
    };
    assert!(assert_outcome(&wrong, Err("expects numbers".into())).is_err());
}
