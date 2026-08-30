use hara_native::{
    core::{native_declarations, NativeAvailability},
    kernel::Form,
    lang::protocol::{protocol_declarations, ProtocolArity, ProtocolAvailability},
    Runtime,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process,
};

const MAGIC: &[u8; 4] = b"HNC1";
const SPEC_PATH: &str = "specs/native-protocol-v1.edn";
const ASSET_PATH: &str = "assets/native-protocol-conformance.hnc";
const EXPANDED_EDN_PATH: &str = "assets/native-protocol-conformance.edn";
const EXPANDED_EDN_HEADER: &str = ";; Generated expanded corpus. Edit specs/native-protocol-v1.edn instead.\n;; The adjacent .hnc is the checksummed bytecode form of these same cases.\n\n";
const MIRROR_HEADER: &str = ";; Generated mirror. Edit hara-native/core/rust/specs/native-protocol-v1.edn instead.\n;; This registry copy is checked against the native-owned source.\n\n";
const ERROR_EXPECTATION_PREFIX: &str = "!error:";

#[derive(Debug, Clone)]
struct ExpandedCase {
    id: String,
    source: String,
    expectation: Expectation,
}

#[derive(Debug, Clone)]
struct ExpandedSuite {
    id: String,
    setup_source: String,
    cases: Vec<ExpandedCase>,
}

#[derive(Debug)]
struct CompiledCorpus {
    binary: Vec<u8>,
    expanded_edn: String,
}

/// HNC1 keeps its existing text expectation field. Error expectations reserve
/// a prefix that cannot be produced by a Hara value display, so every existing
/// reader can distinguish a value assertion from a normalized runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expectation {
    Display(String),
    Error(String),
}

impl Expectation {
    fn encoded(&self) -> String {
        match self {
            Self::Display(display) => display.clone(),
            Self::Error(category) => format!("{ERROR_EXPECTATION_PREFIX}{category}"),
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-native-conformance-artifact: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "check".into());
    let mirror_path = arguments.next().map(PathBuf::from);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let spec_path = root.join(SPEC_PATH);
    let asset_path = root.join(ASSET_PATH);
    let expanded_edn_path = root.join(EXPANDED_EDN_PATH);
    let source = fs::read_to_string(&spec_path).map_err(|error| error.to_string())?;
    let compiled = compile_specification(&source)?;
    match command.as_str() {
        "generate" => {
            fs::create_dir_all(asset_path.parent().expect("asset has parent"))
                .map_err(|error| error.to_string())?;
            fs::write(&asset_path, &compiled.binary).map_err(|error| error.to_string())?;
            fs::write(&expanded_edn_path, &compiled.expanded_edn)
                .map_err(|error| error.to_string())?;
            println!(
                "wrote {} bytes to {} and {} bytes to {}",
                compiled.binary.len(),
                asset_path.display()
                ,
                compiled.expanded_edn.len(),
                expanded_edn_path.display(),
            );
            Ok(())
        }
        "check" => {
            let tracked = fs::read(&asset_path).map_err(|error| error.to_string())?;
            if tracked != compiled.binary {
                return Err(format!(
                    "{} is stale; run with generate",
                    asset_path.display()
                ));
            }
            let expanded_edn =
                fs::read_to_string(&expanded_edn_path).map_err(|error| error.to_string())?;
            if expanded_edn != compiled.expanded_edn {
                return Err(format!(
                    "{} is stale; run with generate",
                    expanded_edn_path.display()
                ));
            }
            println!(
                "{} and {} are current ({} binary bytes, {} EDN bytes)",
                asset_path.display(),
                expanded_edn_path.display(),
                tracked.len(),
                expanded_edn.len(),
            );
            Ok(())
        }
        "mirror" => {
            let mirror_path = required_mirror_path(mirror_path)?;
            let mirror = mirror_source(&source);
            write_mirror(&mirror_path, &mirror)?;
            println!("wrote {} bytes to {}", mirror.len(), mirror_path.display());
            Ok(())
        }
        "check-mirror" => {
            let mirror_path = required_mirror_path(mirror_path)?;
            let mirror = fs::read_to_string(&mirror_path).map_err(|error| error.to_string())?;
            if mirror != mirror_source(&source) {
                return Err(format!(
                    "{} is stale; run with mirror {}",
                    mirror_path.display(),
                    mirror_path.display()
                ));
            }
            println!("{} is current", mirror_path.display());
            Ok(())
        }
        _ => Err("usage: hara-native-conformance-artifact [generate|check|mirror <path>|check-mirror <path>]".into()),
    }
}

fn required_mirror_path(path: Option<PathBuf>) -> Result<PathBuf, String> {
    path.ok_or_else(|| "mirror commands require an explicit destination path".into())
}

fn mirror_source(source: &str) -> String {
    format!("{MIRROR_HEADER}{source}")
}

fn write_mirror(path: &Path, source: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, source).map_err(|error| error.to_string())
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

fn form_source(form: &Form, context: &str, surface: &str) -> Result<String, String> {
    let source = form.to_string();
    if source.contains("std.foundation") {
        return Err(format!("{context} must not reference std.foundation"));
    }
    let required_prefix = match surface {
        "native" => "std.native.",
        "protocol" => "std.protocol.",
        _ => return Err(format!("unknown conformance suite :{surface}")),
    };
    if !source.contains(required_prefix) {
        return Err(format!("{context} must invoke {required_prefix}* directly"));
    }
    Ok(source)
}

fn contains_direct_call(form: &Form, symbol: &str) -> bool {
    match form {
        Form::List(values) => {
            matches!(values.first(), Some(Form::Symbol(head)) if head == symbol)
                || values
                    .iter()
                    .any(|value| contains_direct_call(value, symbol))
        }
        Form::Map(entries) => entries.iter().any(|(key, value)| {
            contains_direct_call(key, symbol) || contains_direct_call(value, symbol)
        }),
        Form::Set(values) | Form::Vector(values) => values
            .iter()
            .any(|value| contains_direct_call(value, symbol)),
        Form::Tagged(_, value) | Form::Metadata(_, value) => contains_direct_call(value, symbol),
        _ => false,
    }
}

fn validate_native_behavioral_coverage(
    manifest: &[(Form, Form)],
    suites: &[Form],
) -> Result<(), String> {
    let Form::Map(coverage) = required(manifest, "coverage", "specification")? else {
        return Err("specification :coverage must be a map".into());
    };
    let Form::Vector(groups) = required(coverage, "native/portable", ":coverage")? else {
        return Err("specification :coverage :native/portable must be a vector".into());
    };
    let native_suite = suites
        .iter()
        .find_map(|suite| match suite {
            Form::Map(entries)
                if matches!(entry(entries, "id"), Some(Form::Keyword(id)) if id == "native") =>
            {
                Some(entries)
            }
            _ => None,
        })
        .ok_or_else(|| "specification is missing the :native suite".to_owned())?;
    let Form::Vector(cases) = required(native_suite, "cases", ":native")? else {
        return Err(":native :cases must be a vector".into());
    };
    let programs = cases
        .iter()
        .map(|case| {
            let Form::Map(case) = case else {
                return Err(":native cases must be maps".into());
            };
            let Form::Keyword(id) = required(case, "id", ":native case")? else {
                return Err(":native case :id must be a keyword".into());
            };
            Ok(required(case, "program", &format!(":{id}"))?)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut covered = HashSet::new();
    let mut unexercised = Vec::new();
    for group in groups {
        let Form::Map(group) = group else {
            return Err(":coverage :native/portable entries must be maps".into());
        };
        let Form::String(type_name) = required(group, "type", ":coverage :native/portable")? else {
            return Err(":coverage :native/portable :type must be a string".into());
        };
        let declaration = native_declarations()
            .iter()
            .find(|declaration| declaration.qualified_name() == *type_name)
            .ok_or_else(|| format!(":coverage names unknown native type {type_name}"))?;
        if declaration.availability != NativeAvailability::Portable {
            return Err(format!(
                ":coverage native type {type_name} is not portable; put it in a capability profile"
            ));
        }
        let Form::Vector(methods) = required(group, "methods", type_name)? else {
            return Err(format!(":coverage {type_name} :methods must be a vector"));
        };
        for method in methods {
            let Form::String(method) = method else {
                return Err(format!(":coverage {type_name} methods must be strings"));
            };
            if !declaration.method(method) {
                return Err(format!(
                    ":coverage names unknown native method {type_name}/{method}"
                ));
            }
            let symbol = format!("{type_name}/{method}");
            if !covered.insert(symbol.clone()) {
                return Err(format!(":coverage names {symbol} more than once"));
            }
            if !programs
                .iter()
                .any(|program| contains_direct_call(program, &symbol))
            {
                unexercised.push(symbol);
            }
        }
    }
    let portable = native_declarations()
        .iter()
        .filter(|declaration| declaration.availability == NativeAvailability::Portable)
        .flat_map(|declaration| {
            declaration
                .methods
                .iter()
                .map(move |method| format!("{}/{}", declaration.qualified_name(), method))
        })
        .collect::<HashSet<_>>();
    let missing = portable.difference(&covered).cloned().collect::<Vec<_>>();
    let extra = covered.difference(&portable).cloned().collect::<Vec<_>>();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(format!(
            ":coverage must own every portable native method; missing [{}], extra [{}]",
            missing.join(", "),
            extra.join(", "),
        ));
    }
    if !unexercised.is_empty() {
        return Err(format!(
            ":coverage requires exact direct native calls for [{}]",
            unexercised.join(", ")
        ));
    }
    Ok(())
}

fn expectation(case: &[(Form, Form)], context: &str) -> Result<Expectation, String> {
    let Form::Map(expect) = required(case, "expect", context)? else {
        return Err(format!("{context} :expect must be a map"));
    };
    match (entry(expect, "display"), entry(expect, "error")) {
        (Some(Form::String(display)), None) => Ok(Expectation::Display(display.clone())),
        (None, Some(Form::Keyword(category))) if category.contains('/') => {
            Ok(Expectation::Error(category.clone()))
        }
        (Some(_), Some(_)) => Err(format!(
            "{context} :expect cannot contain both :display and :error"
        )),
        _ => Err(format!(
            "{context} :expect must contain :display or a namespaced :error category"
        )),
    }
}

#[derive(Debug, Clone)]
struct GeneratedCase {
    id: String,
    source: String,
    expectation: Expectation,
}

fn protocol_arguments(arity: ProtocolArity, variadic: bool) -> String {
    let (minimum, _) = arity.range();
    let count = if variadic {
        minimum
    } else {
        minimum.saturating_sub(1)
    };
    std::iter::repeat_n("nil", count)
        .collect::<Vec<_>>()
        .join(" ")
}

fn protocol_dispatch_source(
    protocol_name: &str,
    method_name: &str,
    arity: ProtocolArity,
    ordinal: usize,
    variadic: bool,
) -> (String, String) {
    let fixture = format!("Fixture{ordinal}");
    let arguments = protocol_arguments(arity, variadic);
    let invocation = if arguments.is_empty() {
        "receiver".to_owned()
    } else {
        format!("receiver {arguments}")
    };
    // A variadic witness has the receiver plus its required trailing value.
    let expected_arity = arity.range().0 + usize::from(variadic);
    (
        format!(
            "(let [target (std.native.Base/namespace 'hnc.protocol.functional.{ordinal}) \
                  fixture (std.native.Base/struct target '{fixture} (std.native.Base/vector)) \
                  protocol (std.protocol.ideref.IDeref/deref (std.native.Base/resolve '{protocol_name})) \
                  _ (std.native.Base/extend target fixture protocol \
                      {{'{method_name} (fn [& values] \
                          (std.protocol.icount.ICount/count values))}}) \
                  constructor (std.protocol.ideref.IDeref/deref \
                               (std.native.Base/resolve target '->{fixture})) \
                  receiver (constructor)] \
              ({protocol_name}/{method_name} {invocation}))"
        ),
        expected_arity.to_string(),
    )
}

fn protocol_functional_cases() -> Vec<GeneratedCase> {
    let mut cases = Vec::new();
    let mut ordinal = 0;
    for declaration in protocol_declarations()
        .iter()
        .filter(|declaration| declaration.availability == ProtocolAvailability::Portable)
    {
        let protocol_name = declaration.runtime_name();
        for method in declaration.methods {
            let (source, expected) =
                protocol_dispatch_source(&protocol_name, method.name, method.arity, ordinal, false);
            cases.push(GeneratedCase {
                id: format!(
                    "protocol-functional-{}-{}",
                    declaration.name.to_ascii_lowercase(),
                    method.name
                ),
                source,
                expectation: Expectation::Display(expected),
            });
            cases.push(GeneratedCase {
                id: format!(
                    "protocol-arity-{}-{}",
                    declaration.name.to_ascii_lowercase(),
                    method.name
                ),
                source: format!("({protocol_name}/{})", method.name),
                expectation: Expectation::Error("protocol/arity".into()),
            });
            if declaration.name != "IEncodable" {
                let arguments = protocol_arguments(method.arity, false);
                let invocation = if arguments.is_empty() {
                    "receiver".to_owned()
                } else {
                    format!("receiver {arguments}")
                };
                cases.push(GeneratedCase {
                    id: format!(
                        "protocol-unsupported-{}-{}",
                        declaration.name.to_ascii_lowercase(),
                        method.name
                    ),
                    source: format!(
                        "(let [receiver (std.protocol.ideref.IDeref/deref \
                                         (std.native.Base/resolve '{protocol_name}))] \
                           ({protocol_name}/{} {invocation}))",
                        method.name
                    ),
                    expectation: Expectation::Error("protocol/unsupported-receiver".into()),
                });
            }
            if matches!(method.arity, ProtocolArity::Variadic { .. }) {
                let (source, expected) = protocol_dispatch_source(
                    &protocol_name,
                    method.name,
                    method.arity,
                    ordinal,
                    true,
                );
                cases.push(GeneratedCase {
                    id: format!(
                        "protocol-variadic-{}-{}",
                        declaration.name.to_ascii_lowercase(),
                        method.name
                    ),
                    source,
                    expectation: Expectation::Display(expected),
                });
            }
            ordinal += 1;
        }
    }
    cases
}

fn compile_specification(source: &str) -> Result<CompiledCorpus, String> {
    let forms = hara_native::kernel::parse_forms(source)?;
    let Some(Form::Map(manifest)) = forms.first() else {
        return Err("native/protocol conformance specification must be a map".into());
    };
    match entry(manifest, "format") {
        Some(Form::Keyword(value)) if value == "hara-native/native-protocol-conformance" => {}
        _ => {
            return Err(
                "specification :format must be :hara-native/native-protocol-conformance".into(),
            )
        }
    }
    match entry(manifest, "version") {
        Some(Form::Number(1)) => {}
        _ => return Err("specification :version must be 1".into()),
    }
    let Form::Vector(suites) = required(manifest, "suites", "specification")? else {
        return Err("specification :suites must be a vector".into());
    };
    if suites.len() != 2 {
        return Err("specification must contain exactly native and protocol suites".into());
    }
    validate_native_behavioral_coverage(manifest, suites)?;

    let runtime = Runtime::new();
    let generated_cases = protocol_functional_cases();
    let mut payload = Vec::new();
    let mut expanded_suites = Vec::with_capacity(suites.len());
    put_u32(&mut payload, suites.len())?;
    for suite in suites {
        let Form::Map(suite) = suite else {
            return Err("every suite must be a map".into());
        };
        let Form::Keyword(id) = required(suite, "id", "suite")? else {
            return Err("suite :id must be a keyword".into());
        };
        let surface = id.as_str();
        let setup = required(suite, "setup", &format!(":{id}"))?;
        let setup_source = if matches!(setup, Form::Nil) {
            "nil".to_owned()
        } else {
            form_source(setup, &format!(":{id} :setup"), surface)?
        };
        let setup_artifact = runtime
            .compile_bytecode_artifact(&setup_source)
            .map_err(|error| format!(":{id} setup failed to compile: {error}"))?;
        let Form::Vector(cases) = required(suite, "cases", &format!(":{id}"))? else {
            return Err(format!(":{id} :cases must be a vector"));
        };
        if cases.is_empty() {
            return Err(format!(":{id} must contain at least one case"));
        }
        put_bytes(&mut payload, id.as_bytes())?;
        put_bytes(&mut payload, &setup_artifact)?;
        let generated = if surface == "protocol" {
            generated_cases.iter().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        put_u32(&mut payload, cases.len() + generated.len())?;
        let mut seen_ids = HashSet::new();
        let mut expanded_cases = Vec::with_capacity(cases.len() + generated.len());
        for case in cases {
            let Form::Map(case) = case else {
                return Err(format!(":{id} cases must be maps"));
            };
            let Form::Keyword(case_id) = required(case, "id", &format!(":{id} case"))? else {
                return Err(format!(":{id} case :id must be a keyword"));
            };
            if !seen_ids.insert(case_id.clone()) {
                return Err(format!(":{id} has a duplicate case id :{case_id}"));
            }
            let program = required(case, "program", &format!(":{case_id}"))?;
            let program_source = form_source(program, &format!(":{case_id}"), surface)?;
            let expected = expectation(case, &format!(":{case_id}"))?;
            let artifact = runtime
                .compile_bytecode_artifact(&program_source)
                .map_err(|error| format!(":{case_id} failed to compile: {error}"))?;
            put_bytes(&mut payload, case_id.as_bytes())?;
            put_bytes(&mut payload, expected.encoded().as_bytes())?;
            put_bytes(&mut payload, &artifact)?;
            expanded_cases.push(ExpandedCase {
                id: case_id.clone(),
                source: program_source,
                expectation: expected,
            });
        }
        for case in generated {
            if !seen_ids.insert(case.id.clone()) {
                return Err(format!(
                    ":{id} has a duplicate generated case id :{}",
                    case.id
                ));
            }
            let source = form_source(
                &hara_native::kernel::parse_forms(&case.source)
                    .map_err(|error| format!(":{} source failed to parse: {error}", case.id))?
                    .into_iter()
                    .next()
                    .ok_or_else(|| format!(":{} source is empty", case.id))?,
                &format!(":{}", case.id),
                surface,
            )?;
            let artifact = runtime
                .compile_bytecode_artifact(&source)
                .map_err(|error| format!(":{} failed to compile: {error}", case.id))?;
            put_bytes(&mut payload, case.id.as_bytes())?;
            put_bytes(&mut payload, case.expectation.encoded().as_bytes())?;
            put_bytes(&mut payload, &artifact)?;
            expanded_cases.push(ExpandedCase {
                id: case.id.clone(),
                source,
                expectation: case.expectation.clone(),
            });
        }
        expanded_suites.push(ExpandedSuite {
            id: id.clone(),
            setup_source,
            cases: expanded_cases,
        });
    }
    let mut output = MAGIC.to_vec();
    output.extend_from_slice(&Sha256::digest(&payload));
    output.extend_from_slice(&payload);
    Ok(CompiledCorpus {
        expanded_edn: render_expanded_edn(source, &output, &expanded_suites),
        binary: output,
    })
}

fn render_expanded_edn(source: &str, binary: &[u8], suites: &[ExpandedSuite]) -> String {
    let case_count = suites.iter().map(|suite| suite.cases.len()).sum::<usize>();
    let mut rendered = format!(
        "{EXPANDED_EDN_HEADER}{{:format :hara-native/native-protocol-conformance-expanded\n \
         :version 1\n \
         :source-sha256 \"{}\"\n \
         :binary-sha256 \"{}\"\n \
         :case-count {case_count}\n \
         :suites\n [",
        sha256_hex(source.as_bytes()),
        sha256_hex(binary),
    );
    for (suite_index, suite) in suites.iter().enumerate() {
        if suite_index > 0 {
            rendered.push('\n');
            rendered.push_str("  ");
        }
        let _ = write!(
            rendered,
            "{{:id :{}\n  :setup {}\n  :cases\n  [",
            suite.id, suite.setup_source
        );
        for (case_index, case) in suite.cases.iter().enumerate() {
            if case_index > 0 {
                rendered.push('\n');
                rendered.push_str("   ");
            }
            let expected = match &case.expectation {
                Expectation::Display(display) => format!(":display {}", edn_string(display)),
                Expectation::Error(category) => format!(":error :{category}"),
            };
            let _ = write!(
                rendered,
                "{{:id :{}\n    :program {}\n    :expect {{{expected}}}}}",
                case.id, case.source,
            );
        }
        rendered.push_str("]}");
    }
    rendered.push_str("]}\n");
    rendered
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn edn_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '\\' => encoded.push_str("\\\\"),
            '"' => encoded.push_str("\\\""),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(encoded, "\\u{:04x}", character as u32);
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn put_u32(output: &mut Vec<u8>, value: usize) -> Result<(), String> {
    output.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| "conformance artifact exceeds u32 limits")?
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
    fn hnc1_expectations_preserve_value_and_error_outcomes() {
        assert_eq!(
            Expectation::Display("42".into()).encoded(),
            "42",
            "value expectations retain their HNC1 representation"
        );
        assert_eq!(
            Expectation::Error("protocol/arity".into()).encoded(),
            "!error:protocol/arity",
            "error expectations use the reserved HNC1 representation"
        );
    }

    #[test]
    fn generated_protocol_cases_invoke_dispatch_and_cover_normalized_failures() {
        let cases = protocol_functional_cases();
        assert!(cases.iter().any(|case| {
            case.id == "protocol-functional-iassoc-assoc"
                && matches!(case.expectation, Expectation::Display(ref value) if value == "3")
                && case.source.contains("std.protocol.iassoc.IAssoc/assoc")
        }));
        assert!(cases.iter().any(|case| {
            case.id == "protocol-arity-iassoc-assoc"
                && case.expectation == Expectation::Error("protocol/arity".into())
        }));
        assert!(cases.iter().any(|case| {
            case.id == "protocol-variadic-icontext-call"
                && case.expectation == Expectation::Display("2".into())
        }));
        assert!(cases
            .iter()
            .all(|case| !case.source.contains("std.foundation")));
    }

    #[test]
    fn native_behavioral_coverage_rejects_a_missing_direct_call() {
        let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(SPEC_PATH))
            .expect("native conformance spec is readable");
        compile_specification(&source).expect("checked-in coverage is complete");
        let incomplete = source.replacen("(std.native.Iter/iter-constantly 7)", "(fn [] 7)", 1);
        let error = compile_specification(&incomplete)
            .expect_err("coverage must fail when a listed direct call disappears");
        assert!(error.contains("std.native.Iter/iter-constantly"));
    }

    #[test]
    fn expanded_edn_is_a_parseable_inventory_of_every_binary_case() {
        let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(SPEC_PATH))
            .expect("native conformance spec is readable");
        let compiled = compile_specification(&source).expect("checked-in coverage is complete");
        let forms = hara_native::kernel::parse_forms(&compiled.expanded_edn)
            .expect("expanded EDN is parseable");
        let Some(Form::Map(manifest)) = forms.first() else {
            panic!("expanded corpus is a map");
        };
        assert_eq!(
            entry(manifest, "format"),
            Some(&Form::Keyword(
                "hara-native/native-protocol-conformance-expanded".into()
            ))
        );
        assert_eq!(
            entry(manifest, "source-sha256"),
            Some(&Form::String(sha256_hex(source.as_bytes())))
        );
        assert_eq!(
            entry(manifest, "binary-sha256"),
            Some(&Form::String(sha256_hex(&compiled.binary)))
        );
        let Form::Number(case_count) = required(manifest, "case-count", "expanded corpus")
            .expect("expanded corpus has a case count")
        else {
            panic!("expanded case count is numeric");
        };
        let Form::Vector(suites) =
            required(manifest, "suites", "expanded corpus").expect("expanded corpus has suites")
        else {
            panic!("expanded suites are a vector");
        };
        let expanded_case_count = suites
            .iter()
            .map(|suite| {
                let Form::Map(suite) = suite else {
                    panic!("expanded suite is a map");
                };
                let Form::Vector(cases) =
                    required(suite, "cases", "expanded suite").expect("expanded suite has cases")
                else {
                    panic!("expanded cases are a vector");
                };
                cases.len()
            })
            .sum::<usize>();
        assert_eq!(*case_count as usize, expanded_case_count);
        let source_forms =
            hara_native::kernel::parse_forms(&source).expect("source specification is parseable");
        let Some(Form::Map(source_manifest)) = source_forms.first() else {
            panic!("source specification is a map");
        };
        let Form::Vector(source_suites) = required(source_manifest, "suites", "specification")
            .expect("source specification has suites")
        else {
            panic!("source suites are a vector");
        };
        let declared_case_count = source_suites
            .iter()
            .map(|suite| {
                let Form::Map(suite) = suite else {
                    panic!("source suite is a map");
                };
                let Form::Vector(cases) =
                    required(suite, "cases", "source suite").expect("source suite has cases")
                else {
                    panic!("source cases are a vector");
                };
                cases.len()
            })
            .sum::<usize>();
        assert_eq!(
            expanded_case_count,
            declared_case_count + protocol_functional_cases().len(),
            "expanded EDN includes declared behavior and functional protocol dispatch, never resolver-only cases"
        );
    }
}
