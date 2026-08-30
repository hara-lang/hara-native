use sha2::{Digest, Sha256};

const HNC_MAGIC: &[u8; 4] = b"HNC1";
const HNC_ARTIFACT: &[u8] = include_bytes!("../../../assets/native-protocol-conformance.hnc");
const ERROR_EXPECTATION_PREFIX: &str = "!error:";

struct HncCase<'a> {
    id: &'a str,
    expected: HncExpectation<'a>,
    artifact: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HncExpectation<'a> {
    Display(&'a str),
    Error(&'a str),
}

impl<'a> HncExpectation<'a> {
    fn parse(value: &'a str) -> Self {
        value
            .strip_prefix(ERROR_EXPECTATION_PREFIX)
            .map(Self::Error)
            .unwrap_or(Self::Display(value))
    }
}

struct HncSuite<'a> {
    id: &'a str,
    setup: &'a [u8],
    cases: Vec<HncCase<'a>>,
}

struct HncReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> HncReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn u32(&mut self) -> Result<usize, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")) as usize)
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = self.u32()?;
        self.take(length)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "HNC1 field length overflows the artifact".to_owned())?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "HNC1 field exceeds the artifact".to_owned())?;
        self.offset = end;
        Ok(value)
    }

    fn finish(&self) -> Result<(), String> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err("HNC1 artifact has trailing bytes".into())
        }
    }
}

fn parse_hnc(bytes: &[u8]) -> Result<Vec<HncSuite<'_>>, String> {
    if !bytes.starts_with(HNC_MAGIC) || bytes.len() < 36 {
        return Err("HNC1 artifact has an invalid or truncated header".into());
    }
    let (digest, payload) = bytes[4..].split_at(32);
    let expected: [u8; 32] = digest.try_into().expect("HNC1 digest has 32 bytes");
    let actual: [u8; 32] = Sha256::digest(payload).into();
    if actual != expected {
        return Err("HNC1 artifact checksum mismatch".into());
    }
    let mut reader = HncReader::new(payload);
    let count = reader.u32()?;
    if count != 2 {
        return Err(format!(
            "HNC1 must contain native and protocol suites, found {count}"
        ));
    }
    let mut suites = Vec::with_capacity(count);
    for _ in 0..count {
        let id = std::str::from_utf8(reader.bytes()?).map_err(|_| "HNC1 suite id is not UTF-8")?;
        let setup = reader.bytes()?;
        let case_count = reader.u32()?;
        if case_count == 0 || case_count > reader.bytes.len() / 12 {
            return Err(format!("HNC1 suite {id} has an invalid case count"));
        }
        let mut cases = Vec::with_capacity(case_count);
        for _ in 0..case_count {
            let case_id =
                std::str::from_utf8(reader.bytes()?).map_err(|_| "HNC1 case id is not UTF-8")?;
            let expected = std::str::from_utf8(reader.bytes()?)
                .map_err(|_| "HNC1 expected display is not UTF-8")?;
            cases.push(HncCase {
                id: case_id,
                expected: HncExpectation::parse(expected),
                artifact: reader.bytes()?,
            });
        }
        suites.push(HncSuite { id, setup, cases });
    }
    reader.finish()?;
    if suites[0].id != "native" || suites[1].id != "protocol" {
        return Err("HNC1 suites must be ordered native then protocol".into());
    }
    Ok(suites)
}

fn requires_foundation(program: &crate::vm::Program) -> bool {
    program.constants.iter().any(|value| {
        let text = value.display();
        text.starts_with("std.foundation/") || text.starts_with("std.foundation.")
    })
}

fn normalized_error_category(error: &str) -> Option<&'static str> {
    let error = error.trim_start_matches("error: ");
    if error.starts_with("protocol/arity:") {
        Some("protocol/arity")
    } else if error.starts_with("protocol/unsupported-receiver:") {
        Some("protocol/unsupported-receiver")
    } else if error.contains("expects") {
        let arity = [
            "expects one ",
            "expects two ",
            "expects three ",
            "expects four ",
            "expects at least ",
            "expects no ",
        ]
        .iter()
        .any(|marker| error.contains(marker));
        if arity {
            Some("native/arity")
        } else if error.contains("number")
            || error.contains("numeric")
            || error.contains("integer")
            || error.contains("string")
        {
            Some("native/type")
        } else {
            Some("native/arity")
        }
    } else {
        None
    }
}

fn assert_outcome(case: &HncCase<'_>, actual: Result<String, String>) -> Result<(), String> {
    match (case.expected, actual) {
        (HncExpectation::Display(expected), Ok(actual)) if actual == expected => Ok(()),
        (HncExpectation::Display(expected), Ok(actual)) => Err(format!(
            "{} expected display {expected:?}, observed {actual:?}",
            case.id
        )),
        (HncExpectation::Display(expected), Err(error)) => Err(format!(
            "{} expected display {expected:?}, raised {error}",
            case.id
        )),
        (HncExpectation::Error(expected), Err(error))
            if normalized_error_category(&error) == Some(expected) =>
        {
            Ok(())
        }
        (HncExpectation::Error(expected), Err(error)) => Err(format!(
            "{} expected error {expected:?}, observed {error:?} ({:?})",
            case.id,
            normalized_error_category(&error)
        )),
        (HncExpectation::Error(expected), Ok(actual)) => Err(format!(
            "{} expected error {expected:?}, returned {actual:?}",
            case.id
        )),
    }
}

#[test]
fn core_runtime_executes_the_native_protocol_artifact_serially() {
    let suites = parse_hnc(HNC_ARTIFACT).expect("embedded HNC1 corpus is valid");
    let expected = suites.iter().map(|suite| suite.cases.len()).sum::<usize>();
    let mut runtime = crate::Runtime::core();
    let mut executed = 0;
    for suite in &suites {
        let setup = crate::vm::decode_program(suite.setup)
            .unwrap_or_else(|error| panic!("{} setup has invalid HBC0: {error}", suite.id));
        assert!(
            !requires_foundation(&setup),
            "{} setup must not require Foundation",
            suite.id
        );
        runtime
            .eval_bytecode_artifact(suite.setup)
            .unwrap_or_else(|error| panic!("{} setup failed: {error}", suite.id));
        for case in &suite.cases {
            let program = crate::vm::decode_program(case.artifact)
                .unwrap_or_else(|error| panic!("{} has invalid HBC0: {error}", case.id));
            assert!(
                !requires_foundation(&program),
                "{} must not require Foundation",
                case.id
            );
            let actual = runtime.eval_bytecode_artifact(case.artifact);
            assert_outcome(case, actual).unwrap_or_else(|error| panic!("{error}"));
            executed += 1;
        }
    }
    assert_eq!(executed, expected, "all declared native/protocol cases ran");
}

#[test]
fn hnc_expected_outcomes_reject_wrong_values_and_error_categories() {
    let value_case = HncCase {
        id: "fixture/value",
        expected: HncExpectation::Display("42"),
        artifact: &[],
    };
    assert!(assert_outcome(&value_case, Ok("41".into())).is_err());
    let error_case = HncCase {
        id: "fixture/error",
        expected: HncExpectation::Error("protocol/arity"),
        artifact: &[],
    };
    assert!(assert_outcome(
        &error_case,
        Err("protocol/unsupported-receiver: missing".into())
    )
    .is_err());
    assert_eq!(
        normalized_error_category("abs expects one numeric value"),
        Some("native/arity")
    );
    assert_eq!(
        normalized_error_category("abs expects a numeric value"),
        Some("native/type")
    );
}
