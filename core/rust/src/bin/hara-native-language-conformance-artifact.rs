use hara_native::{kernel::Form, Runtime};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process,
};

const MAGIC: &[u8; 4] = b"HLC1";
const SPEC_PATH: &str = "specs/language-v1.edn";
const LOWERING_PATH: &str = "specs/lowering-v1.edn";
const ASSET_PATH: &str = "assets/language-conformance.hlc";
const ERROR_EXPECTATION_PREFIX: &str = "!error:";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expectation {
    Display(String),
    Error(String),
}

impl Expectation {
    fn encoded(&self) -> String {
        match self {
            Self::Display(value) => value.clone(),
            Self::Error(value) => format!("{ERROR_EXPECTATION_PREFIX}{value}"),
        }
    }
}

#[derive(Debug, Clone)]
struct LanguageCase {
    id: String,
    layer: String,
    source: String,
    expectation: Expectation,
    browser_safe: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-native-language-conformance-artifact: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = env::args().nth(1).unwrap_or_else(|| "check".into());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let specification =
        fs::read_to_string(root.join(SPEC_PATH)).map_err(|error| error.to_string())?;
    let compiled = compile_specification(&root, &specification)?;
    let asset = root.join(ASSET_PATH);
    match command.as_str() {
        "generate" => {
            fs::create_dir_all(asset.parent().expect("artifact has a parent"))
                .map_err(|error| error.to_string())?;
            fs::write(&asset, &compiled).map_err(|error| error.to_string())?;
            println!("wrote {} bytes to {}", compiled.len(), asset.display());
            Ok(())
        }
        "check" => {
            let tracked = fs::read(&asset).map_err(|error| error.to_string())?;
            if tracked != compiled {
                return Err(format!("{} is stale; run with generate", asset.display()));
            }
            println!("{} is current ({} bytes)", asset.display(), tracked.len());
            Ok(())
        }
        _ => Err("usage: hara-native-language-conformance-artifact [generate|check]".into()),
    }
}

fn compile_specification(root: &Path, source: &str) -> Result<Vec<u8>, String> {
    let forms = hara_native::kernel::parse_forms(source)?;
    let [Form::Map(manifest)] = forms.as_slice() else {
        return Err("language conformance specification must contain one map".into());
    };
    expect_keyword(
        manifest,
        "format",
        "specification",
        "hara-native/language-conformance",
    )?;
    match entry(manifest, "version") {
        Some(Form::Number(1)) => {}
        _ => return Err("specification :version must be 1".into()),
    }
    let Form::Vector(cases) = required(manifest, "cases", "specification")? else {
        return Err("specification :cases must be a vector".into());
    };
    if cases.is_empty() {
        return Err("specification :cases must not be empty".into());
    }

    let registry_source = fs::read_to_string(root.join("specs/language/registry/bytecode-vm.edn"))
        .map_err(|error| error.to_string())?;
    let registry = single_map(&registry_source, "imported bytecode VM registry fixture")?;
    let lowering_source =
        fs::read_to_string(root.join(LOWERING_PATH)).map_err(|error| error.to_string())?;
    let lowering = single_map(&lowering_source, "lowering ledger")?;
    let runtime = Runtime::new();
    let mut payload = Vec::new();
    let mut identifiers = HashSet::new();
    put_u32(&mut payload, cases.len())?;
    for form in cases {
        let language_case = parse_case(form, &registry, &lowering)?;
        if !identifiers.insert(language_case.id.clone()) {
            return Err(format!(
                "specification has a duplicate case id :{}",
                language_case.id
            ));
        }
        let artifact = runtime
            .compile_bytecode_artifact(&language_case.source)
            .map_err(|error| format!(":{} failed to compile: {error}", language_case.id))?;
        put_bytes(&mut payload, language_case.id.as_bytes())?;
        put_bytes(&mut payload, language_case.layer.as_bytes())?;
        put_bytes(&mut payload, language_case.expectation.encoded().as_bytes())?;
        put_u32(&mut payload, usize::from(language_case.browser_safe))?;
        put_bytes(&mut payload, language_case.source.as_bytes())?;
        put_bytes(&mut payload, &artifact)?;
    }
    let mut output = MAGIC.to_vec();
    output.extend_from_slice(&Sha256::digest(&payload));
    output.extend_from_slice(&payload);
    Ok(output)
}

fn parse_case(
    form: &Form,
    registry: &[(Form, Form)],
    lowering: &[(Form, Form)],
) -> Result<LanguageCase, String> {
    let Form::Map(case) = form else {
        return Err("every language conformance case must be a map".into());
    };
    let id = required_keyword(case, "id", "case")?;
    let layer = required_keyword(case, "layer", &id)?;
    if !matches!(layer.as_str(), "parser" | "evaluator" | "native-abi") {
        return Err(format!(":{id} has unsupported layer :{layer}"));
    }
    let program = required(case, "program", &id)?;
    let source = program.to_string();
    if source.contains("std.foundation") || source.contains("std.lib.") {
        return Err(format!(":{id} must not reference a source library"));
    }
    if layer == "native-abi" && !source.contains("std.native.") && !source.contains("std.protocol.")
    {
        return Err(format!(
            ":{id} native ABI case must call std.native or std.protocol"
        ));
    }
    let expectation = expectation(case, &id)?;
    let browser_safe = optional_bool(case, "browser-safe", false)?;
    validate_origin(case, &id, program, &expectation, registry, lowering)?;
    Ok(LanguageCase {
        id,
        layer,
        source,
        expectation,
        browser_safe,
    })
}

fn validate_origin(
    case: &[(Form, Form)],
    id: &str,
    program: &Form,
    case_expectation: &Expectation,
    registry: &[(Form, Form)],
    lowering: &[(Form, Form)],
) -> Result<(), String> {
    let Form::Map(origin) = required(case, "origin", id)? else {
        return Err(format!(":{id} :origin must be a map"));
    };
    let kind = required_keyword(origin, "kind", &format!(":{id} :origin"))?;
    let source_case = required_keyword(origin, "case", &format!(":{id} :origin"))?;
    let document = match kind.as_str() {
        "registry" => {
            let expected_document = "language/registry/bytecode-vm.edn";
            if required_string(origin, "document", &format!(":{id} :origin"))? != expected_document
            {
                return Err(format!(
                    ":{id} registry origin must reference {expected_document}"
                ));
            }
            registry
        }
        "lowering" => lowering,
        _ => return Err(format!(":{id} has unsupported origin kind :{kind}")),
    };
    let Form::Vector(cases) = required(document, "cases", "origin document")? else {
        return Err("origin document :cases must be a vector".into());
    };
    let matching = cases
        .iter()
        .filter_map(|candidate| match candidate {
            Form::Map(entries)
                if matches!(entry(entries, "id"), Some(Form::Keyword(candidate_id)) if candidate_id == &source_case) =>
            {
                Some(entries)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [source] = matching.as_slice() else {
        return Err(format!(
            ":{id} origin :{source_case} must exist exactly once"
        ));
    };
    let source_program = match kind.as_str() {
        "registry" => {
            let Form::String(source) = required(source, "source", &source_case)? else {
                return Err(format!(":{source_case} :source must be a string"));
            };
            let forms = hara_native::kernel::parse_forms(source)
                .map_err(|error| format!(":{source_case} source failed to parse: {error}"))?;
            let [form] = forms.as_slice() else {
                return Err(format!(":{source_case} must contain one source form"));
            };
            form.clone()
        }
        "lowering" => required(source, "program", &source_case)?.clone(),
        _ => unreachable!("origin kind was checked above"),
    };
    if program != &source_program {
        return Err(format!(
            ":{id} program differs from its :{kind} origin :{source_case}"
        ));
    }
    let expected = expectation(source, &source_case)?;
    if case_expectation != &expected {
        return Err(format!(
            ":{id} expectation differs from its :{kind} origin :{source_case}"
        ));
    }
    Ok(())
}

fn expectation(case: &[(Form, Form)], context: &str) -> Result<Expectation, String> {
    let Form::Map(expect) = required(case, "expect", context)? else {
        return Err(format!(":{context} :expect must be a map"));
    };
    match (
        entry(expect, "display"),
        entry(expect, "error"),
        entry(expect, "error-category"),
    ) {
        (Some(Form::String(value)), None, None) => Ok(Expectation::Display(value.clone())),
        (None, Some(Form::Keyword(value)), None) => Ok(Expectation::Error(value.replace('-', " "))),
        (None, Some(Form::String(value)), None) => Ok(Expectation::Error(value.clone())),
        (None, None, Some(Form::String(value))) => Ok(Expectation::Error(value.clone())),
        _ => Err(format!(
            ":{context} :expect must contain exactly one display or error expectation"
        )),
    }
}

fn single_map(source: &str, context: &str) -> Result<Vec<(Form, Form)>, String> {
    let forms = hara_native::kernel::parse_forms(source)?;
    let [Form::Map(document)] = forms.as_slice() else {
        return Err(format!("{context} must contain one map"));
    };
    Ok(document.clone())
}

fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Form::Keyword(name) if name == key => Some(value),
            _ => None,
        })
}

fn required<'a>(entries: &'a [(Form, Form)], key: &str, context: &str) -> Result<&'a Form, String> {
    entry(entries, key).ok_or_else(|| format!("{context} is missing :{key}"))
}

fn required_keyword(entries: &[(Form, Form)], key: &str, context: &str) -> Result<String, String> {
    match required(entries, key, context)? {
        Form::Keyword(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!("{context} :{key} must be a keyword")),
    }
}

fn expect_keyword(
    entries: &[(Form, Form)],
    key: &str,
    context: &str,
    expected: &str,
) -> Result<(), String> {
    match required(entries, key, context)? {
        Form::Keyword(value) if value == expected => Ok(()),
        _ => Err(format!("{context} :{key} must be :{expected}")),
    }
}

fn required_string(entries: &[(Form, Form)], key: &str, context: &str) -> Result<String, String> {
    match required(entries, key, context)? {
        Form::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!("{context} :{key} must be a non-empty string")),
    }
}

fn optional_bool(entries: &[(Form, Form)], key: &str, default: bool) -> Result<bool, String> {
    match entry(entries, key) {
        None => Ok(default),
        Some(Form::Bool(value)) => Ok(*value),
        _ => Err(format!(":{key} must be a boolean")),
    }
}

fn put_u32(output: &mut Vec<u8>, value: usize) -> Result<(), String> {
    output.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| "language conformance artifact exceeds u32 limits")?
            .to_le_bytes(),
    );
    Ok(())
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
    put_u32(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_specification_compiles_to_a_checksummed_hlc1_artifact() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let source = fs::read_to_string(root.join(SPEC_PATH)).unwrap();
        let compiled = compile_specification(&root, &source).unwrap();
        assert!(compiled.starts_with(MAGIC));
        assert!(compiled.len() > 36);
        let digest: [u8; 32] = Sha256::digest(&compiled[36..]).into();
        assert_eq!(&compiled[4..36], &digest);
    }

    #[test]
    fn provenance_rejects_a_program_that_drifted_from_the_lowering_ledger() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let source = fs::read_to_string(root.join(SPEC_PATH)).unwrap();
        let drifted = source.replacen(
            "(std.protocol.icount.ICount/count (std.native.Base/vector 1 2 3))",
            "(std.protocol.icount.ICount/count (std.native.Base/vector 1 2))",
            1,
        );
        assert!(compile_specification(&root, &drifted)
            .unwrap_err()
            .contains("differs from its :lowering origin"));
    }
}
