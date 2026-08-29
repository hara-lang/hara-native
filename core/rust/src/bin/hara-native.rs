//! Standalone HAL-free native host command.
//!
//! `hara-native` executes core forms directly and mounts source only from a
//! verified HARP archive.  The user-facing `hara` command is intentionally a
//! source-package wrapper and is not built into this binary.

use hara_native::{package, package_manifest::PackageManifest, project, Runtime};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
enum Command {
    Eval(String),
    Run(PathBuf),
    Repl,
    Test {
        suite: PathBuf,
        groups: Vec<String>,
    },
    BundleVerify(PathBuf),
    BundleInstall(PathBuf),
    BundleRun {
        archive: PathBuf,
        entry: Option<String>,
    },
    Help,
    Version,
}

fn usage() {
    println!(
        "hara-native <command>\n\n\
         Commands:\n\
           eval FORM                     evaluate one core-language form\n\
           run FILE                      evaluate a source file without libraries\n\
           repl                          start a core-language REPL\n\
           test SUITE.json [GROUP...]    run selected host tests serially in one runtime\n\
           bundle verify ARCHIVE.harp    verify archive paths, digests, and metadata\n\
           bundle install ARCHIVE.harp   verify and install a content-addressed package\n\
           bundle run ARCHIVE.harp [--entry NAMESPACE/SYMBOL]\n\
                                       mount an installed package and evaluate its main\n\n\
         Hara language libraries and the full `hara` CLI are source packages;\n\
         install them through a verified .harp archive."
    );
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "--help" | "-h" | "help" => Ok(Command::Help),
        "--version" | "-V" | "version" => Ok(Command::Version),
        "eval" => {
            Ok(Command::Eval(arguments.next().ok_or_else(|| {
                "hara-native eval requires FORM".to_owned()
            })?))
        }
        "run" => Ok(Command::Run(required_path(&mut arguments, "run")?)),
        "repl" => {
            if let Some(argument) = arguments.next() {
                return Err(format!("hara-native repl does not accept {argument}"));
            }
            Ok(Command::Repl)
        }
        "test" => Ok(Command::Test {
            suite: required_path(&mut arguments, "test")?,
            groups: arguments.collect(),
        }),
        "bundle" => parse_bundle(arguments),
        other => Err(format!("unknown hara-native command: {other}")),
    }
}

fn required_path(
    arguments: &mut impl Iterator<Item = String>,
    command: &str,
) -> Result<PathBuf, String> {
    arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("hara-native {command} requires a path"))
}

fn parse_bundle(mut arguments: impl Iterator<Item = String>) -> Result<Command, String> {
    let operation = arguments
        .next()
        .ok_or_else(|| "hara-native bundle requires verify, install, or run".to_owned())?;
    let archive = required_path(&mut arguments, "bundle")?;
    match operation.as_str() {
        "verify" => no_extra(arguments, Command::BundleVerify(archive)),
        "install" => no_extra(arguments, Command::BundleInstall(archive)),
        "run" => {
            let entry = match arguments.next() {
                None => None,
                Some(option) if option == "--entry" => Some(arguments.next().ok_or_else(|| {
                    "hara-native bundle run --entry requires NAMESPACE/SYMBOL".to_owned()
                })?),
                Some(option) => {
                    return Err(format!(
                        "unknown hara-native bundle run option: {option}; expected --entry"
                    ));
                }
            };
            no_extra(arguments, Command::BundleRun { archive, entry })
        }
        other => Err(format!(
            "unknown hara-native bundle operation: {other}; expected verify, install, or run"
        )),
    }
}

fn no_extra(
    mut arguments: impl Iterator<Item = String>,
    command: Command,
) -> Result<Command, String> {
    match arguments.next() {
        None => Ok(command),
        Some(argument) => Err(format!("unexpected argument: {argument}")),
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Eval(source) => evaluate(&source),
        Command::Run(path) => evaluate_file(&path),
        Command::Repl => repl(),
        Command::Test { suite, groups } => run_test_suite(&suite, &groups),
        Command::BundleVerify(archive) => verify_bundle(&archive),
        Command::BundleInstall(archive) => install_bundle(&archive).map(|_| ()),
        Command::BundleRun { archive, entry } => run_bundle(&archive, entry.as_deref()),
        Command::Help => {
            usage();
            Ok(())
        }
        Command::Version => {
            println!("hara-native {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn evaluate(source: &str) -> Result<(), String> {
    let mut runtime = Runtime::core();
    let value = runtime.eval_native(source)?;
    println!("{value}");
    Ok(())
}

fn evaluate_file(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    evaluate(&source)
}

fn repl() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut runtime = Runtime::core();
    writeln!(stdout, "hara-native core REPL; Ctrl-D to exit").map_err(|error| error.to_string())?;
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        match runtime.eval_native(&line) {
            Ok(value) => writeln!(stdout, "{value}").map_err(|error| error.to_string())?,
            Err(error) => writeln!(stdout, "ERROR {error}").map_err(|error| error.to_string())?,
        }
        stdout.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn verify_bundle(archive: &Path) -> Result<(), String> {
    let manifest = PackageManifest::read_archive(archive).map_err(|error| error.to_string())?;
    println!("verified {} {}", manifest.identity, manifest.version);
    Ok(())
}

fn install_bundle(archive: &Path) -> Result<PathBuf, String> {
    verify_bundle(archive)?;
    let installed = package::install_path(archive)?;
    println!("installed {}", installed.display());
    Ok(installed)
}

fn run_bundle(archive: &Path, entry: Option<&str>) -> Result<(), String> {
    let root = install_bundle(archive)?;
    let project = project::read(&root)?;
    let main = project::main_file(&project)?;
    let source = fs::read_to_string(&main)
        .map_err(|error| format!("cannot read {}: {error}", main.display()))?;
    let catalog = project::source_catalog(&project)?;
    let mut runtime = Runtime::core();
    runtime.register_source_catalog(&catalog);
    runtime.eval_native(&source)?;
    let result = match entry {
        Some(symbol) => runtime.eval_native(&format!("({symbol})"))?,
        None => "nil".to_owned(),
    };
    println!("{result}");
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeTestCase {
    group: String,
    id: String,
    source: String,
    expected: ExpectedTestOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExpectedTestOutcome {
    Value(String),
    Error(String),
}

fn run_test_suite(path: &Path, selected_groups: &[String]) -> Result<(), String> {
    let suite = parse_test_suite(path)?;
    let cases = select_test_cases(&suite, selected_groups)?;
    let mut runtime = Runtime::core();
    let mut failures = Vec::new();

    for case in cases {
        let outcome = runtime.eval_native(&case.source);
        let passed = match (&case.expected, &outcome) {
            (ExpectedTestOutcome::Value(expected), Ok(actual)) => actual == expected,
            (ExpectedTestOutcome::Error(expected), Err(actual)) => actual.contains(expected),
            _ => false,
        };
        if passed {
            println!("PASS  {}/{}", case.group, case.id);
            continue;
        }
        let actual = match outcome {
            Ok(value) => format!("value {value}"),
            Err(error) => format!("error {error}"),
        };
        let expected = match case.expected {
            ExpectedTestOutcome::Value(value) => format!("value {value}"),
            ExpectedTestOutcome::Error(value) => format!("error containing {value}"),
        };
        println!("FAIL  {}/{}", case.group, case.id);
        println!("      expected: {expected}");
        println!("      actual:   {actual}");
        failures.push(format!("{}/{}", case.group, case.id));
    }

    let total = suite_case_count(&suite, selected_groups)?;
    let passed = total - failures.len();
    println!(
        "SUMMARY selected={total} passed={passed} failed={}",
        failures.len()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "native test suite failed: {} case(s)",
            failures.len()
        ))
    }
}

fn parse_test_suite(path: &Path) -> Result<BTreeMap<String, Vec<NativeTestCase>>, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read native test suite {}: {error}", path.display()))?;
    let document: JsonValue = serde_json::from_str(&source)
        .map_err(|error| format!("native test suite is not valid JSON: {error}"))?;
    let root = document
        .as_object()
        .ok_or_else(|| "native test suite must be a JSON object".to_owned())?;
    if root.get("format").and_then(JsonValue::as_str) != Some("hara-native/test-suite/1") {
        return Err("native test suite format must be hara-native/test-suite/1".into());
    }
    let groups = root
        .get("groups")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "native test suite groups must be an object".to_owned())?;
    if groups.is_empty() {
        return Err("native test suite must declare at least one group".into());
    }

    let mut parsed = BTreeMap::new();
    for (group, values) in groups {
        if group.trim().is_empty() {
            return Err("native test suite group names must be non-empty".into());
        }
        let values = values
            .as_array()
            .ok_or_else(|| format!("native test group {group} must be an array"))?;
        if values.is_empty() {
            return Err(format!("native test group {group} must not be empty"));
        }
        let mut cases = Vec::with_capacity(values.len());
        for value in values {
            let value = value
                .as_object()
                .ok_or_else(|| format!("native test group {group} contains a non-object case"))?;
            let id = required_test_string(value, "id", group)?;
            let source = required_test_string(value, "source", group)?;
            let expected_value = optional_test_string(value, "expect", group)?;
            let expected_error = optional_test_string(value, "error", group)?;
            let expected = match (expected_value, expected_error) {
                (Some(value), None) => ExpectedTestOutcome::Value(value),
                (None, Some(value)) => ExpectedTestOutcome::Error(value),
                _ => {
                    return Err(format!(
                        "native test {group}/{id} must declare exactly one of expect or error"
                    ));
                }
            };
            cases.push(NativeTestCase {
                group: group.clone(),
                id,
                source,
                expected,
            });
        }
        parsed.insert(group.clone(), cases);
    }
    Ok(parsed)
}

fn required_test_string(
    value: &serde_json::Map<String, JsonValue>,
    field: &str,
    group: &str,
) -> Result<String, String> {
    optional_test_string(value, field, group)?.ok_or_else(|| {
        format!("native test group {group} cases require a non-empty {field} string")
    })
}

fn optional_test_string(
    value: &serde_json::Map<String, JsonValue>,
    field: &str,
    group: &str,
) -> Result<Option<String>, String> {
    match value.get(field) {
        None => Ok(None),
        Some(JsonValue::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(format!(
            "native test group {group} cases require {field} to be a non-empty string"
        )),
    }
}

fn select_test_cases(
    suite: &BTreeMap<String, Vec<NativeTestCase>>,
    selected_groups: &[String],
) -> Result<Vec<NativeTestCase>, String> {
    let group_names = if selected_groups.is_empty() {
        suite.keys().cloned().collect()
    } else {
        selected_groups.to_vec()
    };
    let mut selected = Vec::new();
    for group in group_names {
        let cases = suite
            .get(&group)
            .ok_or_else(|| format!("native test group is unknown: {group}"))?;
        selected.extend(cases.iter().cloned());
    }
    if selected.is_empty() {
        return Err("native test selection is empty".into());
    }
    Ok(selected)
}

fn suite_case_count(
    suite: &BTreeMap<String, Vec<NativeTestCase>>,
    selected_groups: &[String],
) -> Result<usize, String> {
    Ok(select_test_cases(suite, selected_groups)?.len())
}

fn main() {
    let command = parse_arguments(env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("hara-native: {error}");
        std::process::exit(2);
    });
    if let Err(error) = run(command) {
        eprintln!("hara-native: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_arguments, parse_test_suite, run_test_suite, select_test_cases, Command};
    use hara_native::{package, package_manifest::PackageManifest};
    use std::fs;

    fn suite_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hara-native-test-suite-{}-{name}.json",
            std::process::id()
        ))
    }

    #[test]
    fn parses_source_and_bundle_commands() {
        assert!(matches!(
            parse_arguments(["eval".into(), "(+ 20 22)".into()]),
            Ok(Command::Eval(source)) if source == "(+ 20 22)"
        ));
        assert!(matches!(
            parse_arguments([
                "bundle".into(),
                "run".into(),
                "hara-cli.harp".into(),
                "--entry".into(),
                "tool.cli.main/run".into(),
            ]),
            Ok(Command::BundleRun { archive, entry: Some(entry) })
                if archive.to_string_lossy() == "hara-cli.harp" && entry == "tool.cli.main/run"
        ));
    }

    #[test]
    fn rejects_unknown_bundle_operations() {
        let error =
            parse_arguments(["bundle".into(), "publish".into(), "x.harp".into()]).unwrap_err();
        assert!(error.contains("unknown hara-native bundle operation"));
    }

    #[test]
    fn selects_groups_rejects_unknown_groups_and_requires_non_empty_cases() {
        let path = suite_path("selection");
        fs::write(
            &path,
            r#"{"format":"hara-native/test-suite/1","groups":{"one":[{"id":"one","source":"1","expect":"1"}],"two":[{"id":"two","source":"2","expect":"2"}]}}"#,
        )
        .unwrap();
        let suite = parse_test_suite(&path).unwrap();
        assert_eq!(select_test_cases(&suite, &["two".into()]).unwrap().len(), 1);
        assert!(select_test_cases(&suite, &["missing".into()])
            .unwrap_err()
            .contains("unknown"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn runs_selected_cases_serially_in_one_runtime_and_reports_evaluator_failures() {
        let path = suite_path("serial");
        fs::write(
            &path,
            r##"{"format":"hara-native/test-suite/1","groups":{"serial":[{"id":"define","source":"(def native-test-answer 42)","expect":"#'user/native-test-answer"},{"id":"read","source":"native-test-answer","expect":"42"}],"failure":[{"id":"missing","source":"native-suite-missing","error":"native-suite-missing"}]}}"##,
        )
        .unwrap();
        assert!(run_test_suite(&path, &["serial".into(), "failure".into()]).is_ok());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn verifies_and_installs_a_minimal_source_package_without_a_source_checkout() {
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        );
        let root = std::env::temp_dir().join(format!("hara-native-package-{nonce}"));
        let distribution = std::env::temp_dir().join(format!("hara-native-dist-{nonce}"));
        fs::create_dir_all(root.join("src/demo")).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/demo/main.hal"),
            "(ns demo.main)\n(defn start [] 42)\n",
        )
        .unwrap();

        let archive = package::build_path(&root, None).unwrap();
        let manifest = PackageManifest::read_archive(&archive).unwrap();
        assert_eq!(manifest.identity, "hara:demo/app");
        let installed = package::install_path_at(&archive, &distribution).unwrap();
        assert!(installed.join("project.edn").is_file());
        assert!(installed.join("src/demo/main.hal").is_file());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(distribution).unwrap();
    }
}
