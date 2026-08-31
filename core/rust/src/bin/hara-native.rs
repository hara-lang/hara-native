//! Standalone HAL-free native host command.
//!
//! `hara-native` executes core forms directly and mounts source only from a
//! verified HARP archive.  The user-facing `hara` command is intentionally a
//! source-package wrapper and is not built into this binary.

use hara_native::kernel::{parse, read_forms, Form};
use hara_native::{
    command::{
        App as CommandApp, AppConfig, ArgumentSpec, CommandError, OptionKind, OptionSpec,
        ParsedValue, Request, Route, RouteSpec,
    },
    core::Value,
    distribution, identity_tool,
    native_cli::{install_native_kernel, RuntimeBroker},
    package,
    package_manifest::PackageManifest,
    project,
    resp::RespServer,
    Runtime,
};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

#[path = "hara-signer.rs"]
mod signer;

#[derive(Debug)]
enum Command {
    Eval(String),
    Run(PathBuf),
    Repl,
    Test {
        project: PathBuf,
        files: Vec<PathBuf>,
    },
    TestJson {
        suite: PathBuf,
        groups: Vec<String>,
    },
    BundleBuild {
        project: PathBuf,
        output: Option<PathBuf>,
    },
    BundleVerify(PathBuf),
    BundleInstall(PathBuf),
    BundleRun {
        archive: PathBuf,
        entry: Option<String>,
    },
    BundleExec {
        archive: PathBuf,
        entry: String,
        argv: Vec<String>,
    },
    DistributionBuild {
        project: PathBuf,
        output: PathBuf,
    },
    Signer(Vec<String>),
    Id(Vec<String>),
    Publish,
    Help,
    Version,
}

fn usage() {
    println!(
        r#"hara-native <command>

Commands:
  eval FORM                     evaluate one core-language form
  run FILE                      evaluate a source file without libraries
  repl                          start a core-language REPL
  test [--project PATH] [--file PATH]...
                                run project Test/* files in fresh runtimes
  test-json SUITE.json [GROUP...]
                                run selected JSON host tests in one runtime
  bundle build PROJECT [--output ARCHIVE.harp]
                                package a source project into a HARP archive
  bundle verify ARCHIVE.harp    verify archive paths, digests, and metadata
  bundle install ARCHIVE.harp   verify and install a content-addressed package
  bundle run ARCHIVE.harp [--entry NAMESPACE/SYMBOL]
                                mount an installed package and evaluate its main
  bundle exec ARCHIVE.harp --entry NAMESPACE/SYMBOL [-- ARG...]
                                mount a verified package and invoke its entry with argv
  distribution build PROJECT --output DIRECTORY
                                assemble a relocatable native host and HARP package
  signer generate --key-file PATH
                                create a local development Ed25519 key
  signer public-key --key-file PATH
                                print the key's lowercase public-key hex
  signer sign                   sign a canonical intent from stdin
  id <login|enroll|status|key|namespace|grant|policy> ...
                                manage a publisher identity with the integrated signer
  id policy grant --identity POLICY.edn --root-key-file PATH --key-id ID
                  --public-key HEX --github-subject ID --coordinate COORDINATE
                  --authorization-public-key HEX [--dry-run]
                                offline-root-sign one reviewed exact publisher grant
  publish [--tap TAP] [--dry-run] [--skip-signed-tag] [PROJECT]
                                sign and submit a source-package publication request

`publish --dry-run` performs every local identity, source-tag, recipe, and
registry-policy check without submitting a request. `publish` submits the
signed request; the protected registry deploys the final attested release.
`--skip-signed-tag` uses a clean checkout whose HEAD exactly matches origin's
default branch instead of a signed Git tag. The publisher's Ed25519 signature
binds that remote source commit to the request."#
    );
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let argv = arguments.into_iter().collect::<Vec<_>>();
    let app = cli_application()?;
    let request = app
        .parse(argv.clone())
        .map_err(|error| cli_parse_error(&argv, error))?;
    app.handler(&request).map_err(|error| error.to_string())?(&request)
}

type CliHandler = fn(&Request) -> Result<Command, String>;

fn cli_application() -> Result<CommandApp<CliHandler>, String> {
    let mut app = CommandApp::create(AppConfig {
        id: "hara-native".into(),
        desc: "HAL-free native Hara host".into(),
    })
    .map_err(|error| error.to_string())?;

    let mut help = cli_route("help", &[], "Show command help");
    help.spec.aliases = cli_paths(&[&["help"], &["--help"], &["-h"]]);
    cli_install(&mut app, help)?;
    let mut version = cli_route("version", &["version"], "Show the native host version");
    version.spec.aliases = cli_paths(&[&["--version"], &["-V"]]);
    cli_install(&mut app, version)?;

    let mut eval = cli_route("eval", &["eval"], "Evaluate one core-language form");
    eval.spec.arguments = vec![cli_argument("source", true, false)];
    cli_install(&mut app, eval)?;
    let mut run = cli_route("run", &["run"], "Evaluate a source file");
    run.spec.arguments = vec![cli_argument("file", true, false)];
    cli_install(&mut app, run)?;
    cli_install(
        &mut app,
        cli_route("repl", &["repl"], "Start a core-language REPL"),
    )?;

    let mut test = cli_route("test", &["test"], "Run project Test files");
    test.spec.options = vec![
        cli_string_option("project", "--project", false, Some(".")),
        cli_string_option("file", "--file", true, None),
    ];
    cli_install(&mut app, test)?;
    let mut test_json = cli_route("test-json", &["test-json"], "Run JSON host tests");
    test_json.spec.arguments = vec![
        cli_argument("suite", true, false),
        cli_argument("groups", false, true),
    ];
    cli_install(&mut app, test_json)?;

    let mut bundle = cli_route("bundle", &["bundle"], "Report an unknown bundle operation");
    bundle.spec.passthrough = true;
    bundle.spec.arguments = vec![
        cli_argument("operation", true, false),
        cli_argument("argv", false, true),
    ];
    cli_install(&mut app, bundle)?;
    let mut bundle_build = cli_route("bundle-build", &["bundle", "build"], "Build a HARP archive");
    bundle_build.spec.arguments = vec![cli_argument("project", true, false)];
    bundle_build.spec.options = vec![cli_string_option("output", "--output", false, None)];
    cli_install(&mut app, bundle_build)?;
    let mut bundle_verify = cli_route(
        "bundle-verify",
        &["bundle", "verify"],
        "Verify a HARP archive",
    );
    bundle_verify.spec.arguments = vec![cli_argument("archive", true, false)];
    cli_install(&mut app, bundle_verify)?;
    let mut bundle_install = cli_route(
        "bundle-install",
        &["bundle", "install"],
        "Install a HARP archive",
    );
    bundle_install.spec.arguments = vec![cli_argument("archive", true, false)];
    cli_install(&mut app, bundle_install)?;
    let mut bundle_run = cli_route("bundle-run", &["bundle", "run"], "Run a HARP archive");
    bundle_run.spec.arguments = vec![cli_argument("archive", true, false)];
    bundle_run.spec.options = vec![cli_string_option("entry", "--entry", false, None)];
    cli_install(&mut app, bundle_run)?;
    let mut bundle_exec = cli_route(
        "bundle-exec",
        &["bundle", "exec"],
        "Run a HARP package entry with argv",
    );
    bundle_exec.spec.arguments = vec![
        cli_argument("archive", true, false),
        cli_argument("argv", false, true),
    ];
    bundle_exec.spec.options = vec![cli_string_option("entry", "--entry", false, None)];
    cli_install(&mut app, bundle_exec)?;

    let mut distribution = cli_route(
        "distribution",
        &["distribution"],
        "Report an unknown distribution operation",
    );
    distribution.spec.passthrough = true;
    distribution.spec.arguments = vec![
        cli_argument("operation", true, false),
        cli_argument("argv", false, true),
    ];
    cli_install(&mut app, distribution)?;
    let mut distribution_build = cli_route(
        "distribution-build",
        &["distribution", "build"],
        "Build a relocatable Hara distribution",
    );
    distribution_build.spec.arguments = vec![cli_argument("project", true, false)];
    distribution_build.spec.options = vec![cli_string_option("output", "--output", false, None)];
    cli_install(&mut app, distribution_build)?;

    for (id, desc) in [
        ("signer", "Manage local signing keys"),
        ("id", "Manage publisher identity"),
    ] {
        let mut delegated = cli_route(id, &[id], desc);
        delegated.spec.passthrough = true;
        delegated.spec.arguments = vec![cli_argument("argv", false, true)];
        cli_install(&mut app, delegated)?;
    }
    let mut publish = cli_route("publish", &["publish"], "Publish a source package");
    publish.spec.arguments = vec![cli_argument("project", false, false)];
    publish.spec.options = vec![
        cli_string_option("tap", "--tap", false, Some("hara")),
        cli_boolean_option("dry-run", "--dry-run"),
        cli_boolean_option("skip-signed-tag", "--skip-signed-tag"),
    ];
    cli_install(&mut app, publish)?;
    Ok(app)
}

fn cli_route(id: &str, path: &[&str], desc: &str) -> Route<CliHandler> {
    Route {
        spec: RouteSpec {
            id: id.into(),
            path: path.iter().map(|value| (*value).into()).collect(),
            aliases: Vec::new(),
            desc: desc.into(),
            options: Vec::new(),
            arguments: Vec::new(),
            passthrough: false,
        },
        handler: cli_command,
    }
}

fn cli_paths(paths: &[&[&str]]) -> Vec<Vec<String>> {
    paths
        .iter()
        .map(|path| path.iter().map(|value| (*value).into()).collect())
        .collect()
}

fn cli_argument(id: &str, required: bool, many: bool) -> ArgumentSpec {
    ArgumentSpec {
        id: id.into(),
        required,
        many,
    }
}

fn cli_string_option(id: &str, long: &str, many: bool, default: Option<&str>) -> OptionSpec {
    OptionSpec {
        id: id.into(),
        long: Some(long.into()),
        short: None,
        kind: OptionKind::String,
        many,
        default: default.map(|value| ParsedValue::String(value.into())),
    }
}

fn cli_boolean_option(id: &str, long: &str) -> OptionSpec {
    OptionSpec {
        id: id.into(),
        long: Some(long.into()),
        short: None,
        kind: OptionKind::Boolean,
        many: false,
        default: None,
    }
}

fn cli_install(app: &mut CommandApp<CliHandler>, route: Route<CliHandler>) -> Result<(), String> {
    app.install(route)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn cli_command(request: &Request) -> Result<Command, String> {
    let argument = |id| cli_string(request.arguments.get(id), id);
    let arguments = |id| cli_strings(request.arguments.get(id), id);
    let option = |id| cli_string(request.options.get(id), id);
    let enabled = |id| cli_bool(request.options.get(id), id);
    match request.route_id.as_str() {
        "help" => Ok(Command::Help),
        "version" => Ok(Command::Version),
        "eval" => Ok(Command::Eval(argument("source")?)),
        "run" => Ok(Command::Run(PathBuf::from(argument("file")?))),
        "repl" => Ok(Command::Repl),
        "test" => Ok(Command::Test {
            project: PathBuf::from(option("project")?),
            files: cli_strings(request.options.get("file"), "file")?
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        }),
        "test-json" => Ok(Command::TestJson {
            suite: PathBuf::from(argument("suite")?),
            groups: arguments("groups")?,
        }),
        "bundle" => Err(format!(
            "unknown hara-native bundle operation: {}; expected build, verify, install, run, or exec",
            argument("operation")?
        )),
        "bundle-build" => Ok(Command::BundleBuild {
            project: PathBuf::from(argument("project")?),
            output: non_empty(option("output")?).map(PathBuf::from),
        }),
        "bundle-verify" => Ok(Command::BundleVerify(PathBuf::from(argument("archive")?))),
        "bundle-install" => Ok(Command::BundleInstall(PathBuf::from(argument("archive")?))),
        "bundle-run" => Ok(Command::BundleRun {
            archive: PathBuf::from(argument("archive")?),
            entry: non_empty(option("entry")?),
        }),
        "bundle-exec" => Ok(Command::BundleExec {
            archive: PathBuf::from(argument("archive")?),
            entry: non_empty(option("entry")?)
                .ok_or_else(|| "hara-native bundle exec requires --entry".to_owned())?,
            argv: arguments("argv")?,
        }),
        "distribution" => Err(format!(
            "unknown hara-native distribution operation: {}; expected build",
            argument("operation")?
        )),
        "distribution-build" => Ok(Command::DistributionBuild {
            project: PathBuf::from(argument("project")?),
            output: non_empty(option("output")?)
                .map(PathBuf::from)
                .ok_or_else(|| "hara-native distribution build requires --output".to_owned())?,
        }),
        "signer" => Ok(Command::Signer(arguments("argv")?)),
        "id" => Ok(Command::Id(arguments("argv")?)),
        "publish" => {
            let tap = option("tap")?;
            if tap.is_empty() || tap.starts_with('-') {
                return Err("hara-native publish --tap requires a tap name".into());
            }
            enabled("dry-run")?;
            enabled("skip-signed-tag")?;
            let _ = non_empty(argument("project")?);
            Ok(Command::Publish)
        }
        route => Err(format!("unhandled hara-native command route: {route}")),
    }
}

fn cli_string(value: Option<&ParsedValue>, id: &str) -> Result<String, String> {
    match value {
        Some(ParsedValue::String(value)) => Ok(value.clone()),
        _ => Err(format!("hara-native command route did not provide {id}")),
    }
}

fn cli_strings(value: Option<&ParsedValue>, id: &str) -> Result<Vec<String>, String> {
    match value {
        Some(ParsedValue::Strings(values)) => Ok(values.clone()),
        _ => Err(format!("hara-native command route did not provide {id}")),
    }
}

fn cli_bool(value: Option<&ParsedValue>, id: &str) -> Result<bool, String> {
    match value {
        Some(ParsedValue::Boolean(value)) => Ok(*value),
        _ => Err(format!("hara-native command route did not provide {id}")),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn cli_parse_error(argv: &[String], error: CommandError) -> String {
    let option = argv.iter().find(|value| value.starts_with('-')).cloned();
    match argv {
        [command, ..] if error.code == ":command/unknown-route" => {
            format!("unknown hara-native command: {command}")
        }
        [bundle, operation, ..]
            if bundle == "bundle"
                && operation == "build"
                && error.code == ":command/unknown-option" =>
        {
            format!(
                "unknown hara-native bundle build option: {}; expected --output",
                option.unwrap_or_default()
            )
        }
        [publish, rest @ ..] if publish == "publish" && error.code == ":command/unknown-option" => {
            format!(
                "unknown hara-native publish option: {}",
                option.unwrap_or_default()
            )
        }
        [publish, rest @ ..]
            if publish == "publish" && rest.last().is_some_and(|value| value == "--tap") =>
        {
            "hara-native publish --tap requires a tap name".into()
        }
        _ => error.message,
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Eval(source) => evaluate(&source),
        Command::Run(path) => evaluate_file(&path),
        Command::Repl => repl(),
        Command::Test { project, files } => run_project_tests(&project, &files).map(|_| ()),
        Command::TestJson { suite, groups } => run_test_suite(&suite, &groups),
        Command::BundleBuild { project, output } => build_bundle(&project, output.as_deref()),
        Command::BundleVerify(archive) => verify_bundle(&archive),
        Command::BundleInstall(archive) => install_bundle(&archive).map(|_| ()),
        Command::BundleRun { archive, entry } => run_bundle(&archive, entry.as_deref()),
        Command::BundleExec {
            archive,
            entry,
            argv,
        } => run_bundle_entry(&archive, &entry, &argv),
        Command::DistributionBuild { project, output } => build_distribution(&project, &output),
        Command::Signer(arguments) => signer::run(arguments),
        Command::Id(arguments) => run_id(&arguments),
        Command::Publish => Err(package::github_workflow_required()),
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

fn run_id(arguments: &[String]) -> Result<(), String> {
    if matches!(arguments.first().map(String::as_str), Some("policy"))
        && matches!(arguments.get(1).map(String::as_str), Some("grant"))
    {
        let root_key_file = id_option(arguments, "--root-key-file")?;
        let root_public_key = signer::public_key_hex(&root_key_file)?;
        let forwarded = without_id_option(&arguments[2..], "--root-key-file")?;
        identity_tool::grant_policy_with_signer(&forwarded, &root_public_key, |bytes| {
            signer::sign_with_key_file(&root_key_file, bytes)
                .map(|signature| signature.iter().map(|byte| format!("{byte:02x}")).collect())
        })
    } else if matches!(arguments.first().map(String::as_str), Some("enroll")) {
        let public_key = signer::public_key_from_environment()?;
        identity_tool::enroll_with_signer(
            &arguments[1..],
            &public_key,
            signer::sign_intent_from_environment,
        )
    } else {
        identity_tool::run(arguments)
    }
}

fn id_option(arguments: &[String], name: &str) -> Result<PathBuf, String> {
    let values = arguments
        .windows(2)
        .filter_map(|window| (window[0] == name).then(|| PathBuf::from(&window[1])))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [value] if value.is_absolute() => Ok(value.clone()),
        [value] => Err(format!(
            "{name} must be an absolute path: {}",
            value.display()
        )),
        [] => Err(format!("hara-native id policy grant requires {name}")),
        _ => Err(format!("{name} may be supplied only once")),
    }
}

fn without_id_option(arguments: &[String], name: &str) -> Result<Vec<String>, String> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == name {
            if arguments.get(index + 1).is_none() {
                return Err(format!("{name} requires a value"));
            }
            index += 2;
        } else {
            output.push(arguments[index].clone());
            index += 1;
        }
    }
    Ok(output)
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

fn build_bundle(project: &Path, output: Option<&Path>) -> Result<(), String> {
    let archive = package::build_path(project, output)?;
    println!("built {}", archive.display());
    Ok(())
}

fn install_bundle(archive: &Path) -> Result<PathBuf, String> {
    verify_bundle(archive)?;
    let installed = package::install_path(archive)?;
    println!("installed {}", installed.display());
    Ok(installed)
}

fn install_bundle_silent(archive: &Path) -> Result<PathBuf, String> {
    PackageManifest::read_archive(archive).map_err(|error| error.to_string())?;
    package::install_path(archive)
}

fn run_bundle(archive: &Path, entry: Option<&str>) -> Result<(), String> {
    let result = execute_bundle(archive, entry, None)?;
    println!("{result}");
    Ok(())
}

fn run_bundle_entry(archive: &Path, entry: &str, argv: &[String]) -> Result<(), String> {
    let result = execute_bundle(archive, Some(entry), Some(argv))?;
    println!("{result}");
    Ok(())
}

fn execute_bundle(
    archive: &Path,
    entry: Option<&str>,
    argv: Option<&[String]>,
) -> Result<String, String> {
    let root = install_bundle(archive)?;
    execute_installed_bundle(&root, entry, argv)
}

fn execute_installed_bundle(
    root: &Path,
    entry: Option<&str>,
    argv: Option<&[String]>,
) -> Result<String, String> {
    execute_installed_bundle_roots(root, &[root.to_path_buf()], entry, argv)
}

fn execute_installed_bundle_roots(
    root: &Path,
    roots: &[PathBuf],
    entry: Option<&str>,
    argv: Option<&[String]>,
) -> Result<String, String> {
    let project = project::read(&root)?;
    let main = project::main_file(&project)?;
    let source = fs::read_to_string(&main)
        .map_err(|error| format!("cannot read {}: {error}", main.display()))?;
    let catalog = project::source_catalog(&project)?;
    let mut runtime = Runtime::core();
    runtime.install_native_file_provider(root.to_string_lossy().as_ref());
    install_native_kernel(
        &mut runtime,
        RuntimeBroker::start_with_source_catalog(
            Some(root.to_path_buf()),
            false,
            false,
            false,
            "interpreter",
            catalog.clone(),
        )?,
    );
    runtime.register_source_catalog(&catalog);
    for package_root in roots {
        runtime.register_installed_package(package_root)?;
    }
    if catalog.path("std.foundation").is_some() {
        runtime.bootstrap_source_foundation()?;
    }
    runtime.eval_native(&source)?;
    let result = match (entry, argv) {
        (Some(symbol), Some(argv)) => {
            let argv = Form::Vector(argv.iter().cloned().map(Form::String).collect()).to_string();
            runtime.eval_native(&format!("({symbol} {argv})"))?
        }
        (Some(symbol), None) => runtime.eval_native(&format!("({symbol})"))?,
        (None, None) => "nil".to_owned(),
        (None, Some(_)) => return Err("package argv requires an entry symbol".into()),
    };
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionHostAction {
    Resp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RespLaunch {
    project: PathBuf,
    root: PathBuf,
    host: String,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompanionCommandResponse {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn companion_host_action(result: &str) -> Result<Option<CompanionHostAction>, String> {
    let form = parse(result)
        .map_err(|error| format!("companion command returned an invalid Hara result: {error}"))?;
    let Form::Map(entries) = form else {
        return Ok(None);
    };
    let action = entries.iter().find_map(|(key, value)| {
        matches!(key, Form::Keyword(name) if name == "hara/host-action").then_some(value)
    });
    match action {
        None => Ok(None),
        Some(Form::Keyword(action)) if action == "resp" => Ok(Some(CompanionHostAction::Resp)),
        Some(Form::Keyword(action)) => Err(format!("unsupported Hara host action: :{action}")),
        Some(_) => Err("Hara host action must be a keyword".into()),
    }
}

fn companion_command_response(result: &str) -> Result<Option<CompanionCommandResponse>, String> {
    let form = parse(result)
        .map_err(|error| format!("companion command returned an invalid Hara result: {error}"))?;
    let Form::Map(entries) = form else {
        return Ok(None);
    };
    let field = |name| {
        entries.iter().find_map(|(key, value)| {
            matches!(key, Form::Keyword(keyword) if keyword == name).then_some(value)
        })
    };
    let (Some(Form::String(stdout)), Some(Form::String(stderr)), Some(Form::Number(exit))) =
        (field("stdout"), field("stderr"), field("exit"))
    else {
        return Ok(None);
    };
    if entries.len() != 3 || !entries.iter().all(|(key, _)| {
        matches!(key, Form::Keyword(keyword) if keyword == "stdout" || keyword == "stderr" || keyword == "exit")
    }) {
        return Ok(None);
    }
    let exit = i32::try_from(*exit)
        .ok()
        .filter(|exit| (0..=255).contains(exit))
        .ok_or_else(|| "companion command response :exit must be between 0 and 255".to_owned())?;
    Ok(Some(CompanionCommandResponse {
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        exit,
    }))
}

fn write_companion_command_response(response: &CompanionCommandResponse) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(response.stdout.as_bytes())
        .map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())?;
    let mut stderr = io::stderr().lock();
    stderr
        .write_all(response.stderr.as_bytes())
        .map_err(|error| error.to_string())?;
    stderr.flush().map_err(|error| error.to_string())
}

fn resp_launch(arguments: &[String]) -> Result<RespLaunch, String> {
    let mut project = None;
    let mut root = None;
    let mut host = None;
    let mut port = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        match argument.as_str() {
            "headless" => index += 1,
            "--project" | "--root" | "--host" | "--port" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| format!("{argument} requires a value"))?;
                match argument.as_str() {
                    "--project" => {
                        if project.replace(PathBuf::from(value)).is_some() {
                            return Err("--project may be supplied only once".into());
                        }
                    }
                    "--root" => {
                        if root.replace(PathBuf::from(value)).is_some() {
                            return Err("--root may be supplied only once".into());
                        }
                    }
                    "--host" => {
                        if host.replace(value.clone()).is_some() {
                            return Err("--host may be supplied only once".into());
                        }
                    }
                    "--port" => {
                        let parsed = value
                            .parse::<u16>()
                            .map_err(|_| format!("--port must be a u16: {value}"))?;
                        if port.replace(parsed).is_some() {
                            return Err("--port may be supplied only once".into());
                        }
                    }
                    _ => unreachable!(),
                }
                index += 2;
            }
            _ => return Err(format!("RESP host does not accept argument: {argument}")),
        }
    }
    let project = match project {
        Some(project) => project,
        None => env::current_dir()
            .map_err(|error| format!("cannot determine RESP project directory: {error}"))?,
    };
    let root = root.unwrap_or_else(|| project.clone());
    let host = host.unwrap_or_else(|| "127.0.0.1".into());
    if host != "127.0.0.1" {
        return Err("RESP host must be the loopback address 127.0.0.1".into());
    }
    Ok(RespLaunch {
        project,
        root,
        host,
        port: port.unwrap_or(0),
    })
}

fn start_companion_resp(runtime_root: &Path, arguments: &[String]) -> Result<RespServer, String> {
    let launch = resp_launch(arguments)?;
    let runtime_project = project::read(runtime_root)?;
    let client_project = project::discover(&launch.project)?;
    let catalog = project::source_catalogs(&[&runtime_project, &client_project])?;
    let root = launch.root.canonicalize().map_err(|error| {
        format!(
            "cannot resolve RESP root {}: {error}",
            launch.root.display()
        )
    })?;
    let broker = RuntimeBroker::start_with_source_catalog(
        Some(root),
        false,
        false,
        false,
        "interpreter",
        catalog,
    )?;
    RespServer::start(&launch.host, launch.port, broker)
}

fn run_companion_resp(runtime_root: &Path, arguments: &[String]) -> Result<(), String> {
    let server = start_companion_resp(runtime_root, arguments)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "HARA RESP {}", server.endpoint()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())?;
    loop {
        std::thread::park();
    }
}

fn build_distribution(project: &Path, output: &Path) -> Result<(), String> {
    let native = env::current_exe()
        .map_err(|error| format!("cannot determine native launcher path: {error}"))?;
    let manifest = distribution::build(project, &native, output)?;
    println!(
        "built distribution {} {} at {}",
        manifest.source_identity,
        manifest.source_version,
        output.display()
    );
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestCounts {
    passed: usize,
    failed: usize,
    error: usize,
    timeout: usize,
    skipped: usize,
    cancelled: usize,
}

impl TestCounts {
    fn failing(&self) -> usize {
        self.failed + self.error + self.timeout
    }

    fn total(&self) -> usize {
        self.passed + self.failed + self.error + self.timeout + self.skipped + self.cancelled
    }

    fn add(&mut self, other: &Self) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.error += other.error;
        self.timeout += other.timeout;
        self.skipped += other.skipped;
        self.cancelled += other.cancelled;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectTestFile {
    path: PathBuf,
    counts: TestCounts,
    detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProjectTestReport {
    files: Vec<ProjectTestFile>,
    counts: TestCounts,
}

fn value_map_get(value: &Value, key: &str) -> Option<Value> {
    let key = Value::Keyword(key.into());
    match value {
        Value::Map(values) => values.get(&key).cloned(),
        _ => None,
    }
}

fn value_count(value: Option<Value>, key: &str) -> Result<usize, String> {
    match value {
        Some(Value::Number(value)) if value >= 0 => Ok(value as usize),
        _ => Err(format!("test summary :counts requires non-negative :{key}")),
    }
}

fn test_counts_from_summary(value: Value) -> Result<TestCounts, String> {
    let status = value_map_get(&value, "status");
    if !matches!(status, Some(Value::Keyword(ref status)) if matches!(status.as_str(), "passed" | "failed"))
    {
        return Err("test summary requires :status :passed or :failed".into());
    }
    let counts = value_map_get(&value, "counts")
        .ok_or_else(|| "test summary requires a :counts map".to_owned())?;
    if !matches!(counts, Value::Map(_)) {
        return Err("test summary :counts must be a map".into());
    }
    Ok(TestCounts {
        passed: value_count(value_map_get(&counts, "passed"), "passed")?,
        failed: value_count(value_map_get(&counts, "failed"), "failed")?,
        error: value_count(value_map_get(&counts, "error"), "error")?,
        timeout: value_count(value_map_get(&counts, "timeout"), "timeout")?,
        skipped: value_count(value_map_get(&counts, "skipped"), "skipped")?,
        cancelled: value_count(value_map_get(&counts, "cancelled"), "cancelled")?,
    })
}

fn test_counts_from_checks(value: Value) -> Result<TestCounts, String> {
    let checks = match value {
        Value::Vector(values) => values.iter().cloned().collect::<Vec<_>>(),
        Value::Tuple(values) => values.iter().cloned().collect::<Vec<_>>(),
        _ => {
            return Err(
                "test file must return a Test/run summary or Test/check Result vector".into(),
            )
        }
    };
    let mut counts = TestCounts::default();
    for check in checks {
        let Value::Result(result) = check else {
            return Err("Test/check output must contain only native Result values".into());
        };
        if result.is_success() && matches!(result.data, Value::Bool(true)) {
            counts.passed += 1;
        } else if result.is_error() {
            if result.is_timeout()
                || result
                    .error
                    .as_ref()
                    .is_some_and(|error| error.message == "asynchronous test did not settle")
            {
                counts.timeout += 1;
            } else {
                counts.error += 1;
            }
        } else {
            counts.failed += 1;
        }
    }
    Ok(counts)
}

fn test_file_counts(value: Value) -> Result<TestCounts, String> {
    let counts = match value {
        Value::Map(_) => test_counts_from_summary(value),
        Value::Vector(_) | Value::Tuple(_) => test_counts_from_checks(value),
        _ => Err("test file must return a Test/run summary or Test/check Result vector".into()),
    }?;
    if counts.total() == 0 {
        return Err("test file must contain Test/check cases or Test/register facts".into());
    }
    Ok(counts)
}

fn form_without_metadata(mut form: &Form) -> &Form {
    while let Form::Metadata(_, value) = form {
        form = value;
    }
    form
}

fn reject_top_level_test_run(source: &str) -> Result<(), String> {
    let forms = read_forms(source).map_err(|error| error.to_string())?;
    for form in forms {
        let Form::List(values) = form_without_metadata(&form.form) else {
            continue;
        };
        let Some(Form::Symbol(name)) = values.first().map(form_without_metadata) else {
            continue;
        };
        if matches!(name.as_str(), "Test/run" | "std.native.Test/run") {
            return Err(format!(
                "Test/run is runner-owned; use Test/check or Test/register at namespace top level (line {})",
                form.span.start.line
            ));
        }
    }
    Ok(())
}

fn selected_test_files(
    project: &project::Project,
    requested: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let all = project::files_in(&project.root, &project.test_paths)?;
    if requested.is_empty() {
        if all.is_empty() {
            return Err(format!(
                "project {} has no .hal test files",
                project.root.display()
            ));
        }
        return Ok(all);
    }
    let canonical_all = all
        .iter()
        .map(|path| {
            path.canonicalize()
                .map(|canonical| (canonical, path.clone()))
                .map_err(|error| format!("cannot resolve test file {}: {error}", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut selected = Vec::new();
    for requested_path in requested {
        let candidate = if requested_path.is_absolute() {
            requested_path.clone()
        } else {
            project.root.join(requested_path)
        };
        let canonical = candidate.canonicalize().map_err(|error| {
            format!(
                "cannot resolve --file {}: {error}",
                requested_path.display()
            )
        })?;
        let Some((_, matched)) = canonical_all
            .iter()
            .find(|(available, _)| available == &canonical)
        else {
            return Err(format!(
                "--file {} is not a .hal file beneath this project's test paths",
                requested_path.display()
            ));
        };
        selected.push(matched.clone());
    }
    selected.sort();
    selected.dedup();
    Ok(selected)
}

fn run_project_tests(
    project_path: &Path,
    requested: &[PathBuf],
) -> Result<ProjectTestReport, String> {
    let project = project::discover(project_path)?;
    let files = selected_test_files(&project, requested)?;
    let catalog = project::source_catalog(&project)?;
    let source_foundation = catalog.path("std.foundation").is_some();
    let mut report = ProjectTestReport::default();
    for path in files {
        let outcome = (|| -> Result<TestCounts, String> {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            reject_top_level_test_run(&source)?;
            let mut runtime = Runtime::core();
            runtime.install_native_file_provider(project.root.to_string_lossy().as_ref());
            install_native_kernel(
                &mut runtime,
                RuntimeBroker::start_with_source_catalog(
                    Some(project.root.clone()),
                    false,
                    false,
                    false,
                    "interpreter",
                    catalog.clone(),
                )?,
            );
            runtime.register_source_catalog(&catalog);
            if source_foundation {
                runtime.bootstrap_source_foundation()?;
            }
            runtime.eval_native_value(&source)?;
            test_file_counts(runtime.eval_native_value("(std.native.Test/run)")?)
        })();
        let file = match outcome {
            Ok(counts) => ProjectTestFile {
                path,
                counts,
                detail: None,
            },
            Err(error) => ProjectTestFile {
                path,
                counts: TestCounts {
                    error: 1,
                    ..TestCounts::default()
                },
                detail: Some(error),
            },
        };
        report.counts.add(&file.counts);
        let status = if file.counts.failing() == 0 {
            "PASS"
        } else {
            "FAIL"
        };
        println!(
            "{status}  {} passed={} failed={} error={} timeout={} skipped={} cancelled={}",
            file.path.display(),
            file.counts.passed,
            file.counts.failed,
            file.counts.error,
            file.counts.timeout,
            file.counts.skipped,
            file.counts.cancelled,
        );
        if let Some(detail) = &file.detail {
            println!("      {detail}");
        }
        report.files.push(file);
    }
    println!(
        "SUMMARY files={} passed={} failed={} error={} timeout={} skipped={} cancelled={}",
        report.files.len(),
        report.counts.passed,
        report.counts.failed,
        report.counts.error,
        report.counts.timeout,
        report.counts.skipped,
        report.counts.cancelled,
    );
    if report.counts.failing() == 0 {
        Ok(report)
    } else {
        Err(format!(
            "project tests failed: {} failing fact(s) or file(s)",
            report.counts.failing()
        ))
    }
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
    let arguments: Vec<_> = env::args().skip(1).collect();
    match run_sealed_distribution(&arguments) {
        Ok(Some(exit)) => {
            if exit != 0 {
                std::process::exit(exit);
            }
            return;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("hara: {error}");
            std::process::exit(1);
        }
    }
    match run_companion_distribution(&arguments) {
        Ok(Some(exit)) => {
            if exit != 0 {
                std::process::exit(exit);
            }
            return;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("hara: {error}");
            std::process::exit(1);
        }
    }
    let command = if arguments.is_empty() && signer::is_configured() {
        Ok(Command::Signer(Vec::new()))
    } else {
        parse_arguments(arguments)
    }
    .unwrap_or_else(|error| {
        eprintln!("hara-native: {error}");
        std::process::exit(2);
    });
    if let Err(error) = run(command) {
        eprintln!("hara-native: {error}");
        std::process::exit(1);
    }
}

fn run_sealed_distribution(arguments: &[String]) -> Result<Option<i32>, String> {
    let native = env::current_exe()
        .map_err(|error| format!("cannot determine native launcher path: {error}"))?;
    let Some(installed) = distribution::install_sealed(&native)? else {
        return Ok(None);
    };
    run_installed_distribution_roots(
        &installed.primary,
        &installed.roots,
        &installed.manifest.entry,
        arguments,
    )
    .map(Some)
}

fn run_companion_distribution(arguments: &[String]) -> Result<Option<i32>, String> {
    let native = env::current_exe()
        .map_err(|error| format!("cannot determine native launcher path: {error}"))?;
    let Some(root) = companion_root(&native) else {
        return Ok(None);
    };
    if !root.join(distribution::MANIFEST_PATH).is_file() {
        return Ok(None);
    }
    let manifest = distribution::verify(&root, &native)?;
    let archive = root.join(&manifest.archive);
    let installed = install_bundle_silent(&archive)?;
    run_installed_distribution(&installed, &manifest.entry, arguments).map(Some)
}

fn run_installed_distribution(
    installed: &Path,
    entry: &str,
    arguments: &[String],
) -> Result<i32, String> {
    run_installed_distribution_roots(installed, &[installed.to_path_buf()], entry, arguments)
}

fn run_installed_distribution_roots(
    installed: &Path,
    roots: &[PathBuf],
    entry: &str,
    arguments: &[String],
) -> Result<i32, String> {
    let result = execute_installed_bundle_roots(installed, roots, Some(entry), Some(arguments))?;
    match companion_host_action(&result)? {
        Some(CompanionHostAction::Resp) => {
            run_companion_resp(installed, arguments)?;
            Ok(0)
        }
        None => match companion_command_response(&result)? {
            Some(response) => {
                let exit = response.exit;
                write_companion_command_response(&response)?;
                Ok(exit)
            }
            None => {
                println!("{result}");
                Ok(0)
            }
        },
    }
}

fn companion_root(native: &Path) -> Option<PathBuf> {
    let bin = native.parent()?;
    if bin.file_name()?.to_string_lossy() != "bin" {
        return None;
    }
    bin.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::{
        companion_command_response, companion_host_action, companion_root, parse_arguments,
        parse_test_suite, reject_top_level_test_run, resp_launch, run_id, run_project_tests,
        run_test_suite, select_test_cases, signer, start_companion_resp, Command,
        CompanionHostAction,
    };
    use hara_native::{
        identity_tool, package,
        package_manifest::PackageManifest,
        resp::{RespConnection, RespValue},
        tap,
    };
    use std::cell::RefCell;
    use std::fs;
    use std::net::TcpStream;
    use std::process::Command as ProcessCommand;

    fn suite_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hara-native-test-suite-{}-{name}.json",
            std::process::id()
        ))
    }

    fn test_git(
        root: &std::path::Path,
        arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> Result<String, String> {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        let output = ProcessCommand::new("git")
            .args(&arguments)
            .current_dir(root)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                arguments
                    .iter()
                    .map(|argument| argument.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    #[test]
    fn parses_source_and_bundle_commands() {
        assert!(matches!(
            parse_arguments(["eval".into(), "(+ 20 22)".into()]),
            Ok(Command::Eval(source)) if source == "(+ 20 22)"
        ));
        assert!(matches!(
            parse_arguments([
                "test".into(),
                "--project".into(),
                "demo".into(),
                "--file".into(),
                "test/demo/one_test.hal".into(),
                "--file".into(),
                "test/demo/two_test.hal".into(),
            ]),
            Ok(Command::Test { project, files })
                if project == std::path::PathBuf::from("demo")
                    && files == [
                        std::path::PathBuf::from("test/demo/one_test.hal"),
                        std::path::PathBuf::from("test/demo/two_test.hal"),
                    ]
        ));
        assert!(matches!(
            parse_arguments(["test-json".into(), "suite.json".into(), "smoke".into()]),
            Ok(Command::TestJson { suite, groups })
                if suite == std::path::PathBuf::from("suite.json") && groups == ["smoke"]
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
        assert!(matches!(
            parse_arguments([
                "bundle".into(),
                "exec".into(),
                "hara-cli.harp".into(),
                "--entry".into(),
                "tool.cli.main/run".into(),
                "--".into(),
                "--version".into(),
            ]),
            Ok(Command::BundleExec { archive, entry, argv })
                if archive.to_string_lossy() == "hara-cli.harp"
                    && entry == "tool.cli.main/run"
                    && argv == ["--version"]
        ));
        assert!(matches!(
            parse_arguments([
                "distribution".into(),
                "build".into(),
                "source".into(),
                "--output".into(),
                "target/hara".into(),
            ]),
            Ok(Command::DistributionBuild { project, output })
                if project == std::path::PathBuf::from("source")
                    && output == std::path::PathBuf::from("target/hara")
        ));
        assert!(matches!(
            parse_arguments([
                "bundle".into(),
                "build".into(),
                "examples/smoke-answer".into(),
                "--output".into(),
                "target/smoke-answer.harp".into(),
            ]),
            Ok(Command::BundleBuild { project, output: Some(output) })
                if project == std::path::PathBuf::from("examples/smoke-answer")
                    && output == std::path::PathBuf::from("target/smoke-answer.harp")
        ));
        assert!(matches!(
            parse_arguments([
                "signer".into(),
                "public-key".into(),
                "--key-file".into(),
                "/private/key".into(),
            ]),
            Ok(Command::Signer(arguments))
                if arguments == ["public-key", "--key-file", "/private/key"]
        ));
        assert!(matches!(
            parse_arguments(["id".into(), "enroll".into(), "--owner".into(), "octo".into()]),
            Ok(Command::Id(arguments)) if arguments == ["enroll", "--owner", "octo"]
        ));
        assert!(matches!(
            parse_arguments([
                "publish".into(),
                "--tap".into(),
                "partner".into(),
                "--dry-run".into(),
                "package-root".into(),
            ]),
            Ok(Command::Publish)
        ));
    }

    #[test]
    fn derives_a_companion_distribution_root_only_from_a_bin_launcher() {
        assert_eq!(
            companion_root(std::path::Path::new("target/hara/bin/hara")),
            Some(std::path::PathBuf::from("target/hara"))
        );
        assert_eq!(
            companion_root(std::path::Path::new("target/hara-native")),
            None
        );
    }

    #[test]
    fn recognizes_the_source_owned_resp_host_action() {
        assert_eq!(
            companion_host_action("{:hara/host-action :resp}").unwrap(),
            Some(CompanionHostAction::Resp)
        );
        assert_eq!(companion_host_action("\"hara 0.1.0\"").unwrap(), None);
        let error = companion_host_action("{:hara/host-action :other}").unwrap_err();
        assert!(error.contains("unsupported Hara host action"));
    }

    #[test]
    fn recognizes_standard_command_responses_without_claiming_other_maps() {
        assert_eq!(
            companion_command_response("{:stdout \"ok\\n\" :stderr \"\" :exit 0}").unwrap(),
            Some(super::CompanionCommandResponse {
                stdout: "ok\n".into(),
                stderr: "".into(),
                exit: 0,
            })
        );
        assert_eq!(
            companion_command_response("{:hara/host-action :resp}").unwrap(),
            None
        );
        assert!(
            companion_command_response("{:stdout \"\" :stderr \"bad\" :exit 999}")
                .unwrap_err()
                .contains(":exit")
        );
    }

    #[test]
    fn parses_only_loopback_resp_transport_options() {
        let launch = resp_launch(&[
            "--project".into(),
            "project-root".into(),
            "--root".into(),
            "execution-root".into(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            "0".into(),
            "headless".into(),
        ])
        .unwrap();
        assert_eq!(launch.project, std::path::PathBuf::from("project-root"));
        assert_eq!(launch.root, std::path::PathBuf::from("execution-root"));
        assert_eq!(launch.host, "127.0.0.1");
        assert_eq!(launch.port, 0);
        assert!(
            resp_launch(&["--host".into(), "0.0.0.0".into(), "headless".into()])
                .unwrap_err()
                .contains("loopback")
        );
        assert!(
            resp_launch(&["--port".into(), "not-a-port".into(), "headless".into()])
                .unwrap_err()
                .contains("u16")
        );
    }

    #[test]
    fn resp_server_mounts_the_client_project_after_source_bootstrap() {
        let root = std::env::temp_dir().join(format!(
            "hara-native-companion-resp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runtime = root.join("runtime");
        let client = root.join("client");
        let result = (|| -> Result<(), String> {
            fs::create_dir_all(runtime.join("src/std")).map_err(|error| error.to_string())?;
            fs::create_dir_all(client.join("src/demo")).map_err(|error| error.to_string())?;
            fs::write(
                runtime.join("project.edn"),
                "{:hara/type :project :hara/version \"1.0.0\" :project/id hara/foundation :project/version \"0.1.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/main std.foundation :project/capabilities #{}}\n",
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                runtime.join("src/std/foundation.hal"),
                "(ns std.foundation)\n(defmacro intern-in [_] nil)\n",
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                client.join("project.edn"),
                "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/client :project/version \"0.1.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/main demo.app :project/capabilities #{}}\n",
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                client.join("src/demo/app.hal"),
                "(ns demo.app)\n(defn answer [] 42)\n",
            )
            .map_err(|error| error.to_string())?;
            let arguments = vec![
                "--project".into(),
                client.display().to_string(),
                "--root".into(),
                client.display().to_string(),
                "--host".into(),
                "127.0.0.1".into(),
                "--port".into(),
                "0".into(),
                "headless".into(),
            ];
            let mut server = start_companion_resp(&runtime, &arguments)?;
            let response = (|| -> Result<RespValue, String> {
                let project = client.display().to_string();
                let mut connection = RespConnection::new(
                    TcpStream::connect(server.endpoint())
                        .map_err(|error| format!("RESP connection failed: {error}"))?,
                )?;
                connection.write(&RespValue::array([
                    "HELLO",
                    "4",
                    "EMACS",
                    "HARA-MODE",
                    "PROJECT",
                    &project,
                ]))?;
                let hello = connection
                    .read()?
                    .ok_or_else(|| "RESP server closed during HELLO".to_owned())?;
                if !matches!(hello, RespValue::Array(Some(_))) {
                    return Err(format!("unexpected RESP HELLO: {hello:?}"));
                }
                connection.write(&RespValue::array([
                    "EVAL",
                    "REQ-1",
                    "(require 'demo.app) (demo.app/answer)",
                ]))?;
                connection
                    .read()?
                    .ok_or_else(|| "RESP server closed during EVAL".to_owned())
            })();
            server.stop();
            let response = response?;
            let expected = RespValue::array(["RESULT", "REQ-1", "42"]);
            if response != expected {
                return Err(format!("unexpected RESP EVAL: {response:?}"));
            }
            Ok(())
        })();
        let cleanup = fs::remove_dir_all(&root);
        cleanup.map_err(|error| error.to_string()).unwrap();
        result.unwrap();
    }

    #[test]
    fn bundle_exec_makes_intern_in_available_after_require_and_passes_argv() {
        let root =
            std::env::temp_dir().join(format!("hara-native-bundle-exec-{}", std::process::id()));
        let store = root.join("store");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/demo/cli")).unwrap();
        fs::create_dir_all(root.join("src/std")).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/cli :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/main demo.cli :project/capabilities #{}}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/std/foundation.hal"),
            "(ns std.foundation)\n(defmacro intern-in [_] '(def main demo.cli.internal/main))\n",
        )
        .unwrap();
        fs::write(
            root.join("src/demo/cli/internal.hal"),
            "(ns demo.cli.internal)\n(defn main [argv] argv)\n",
        )
        .unwrap();
        fs::write(
            root.join("src/demo/cli.hal"),
            "(ns demo.cli)\n(require 'demo.cli.internal)\n(intern-in [main demo.cli.internal/main])\n",
        )
        .unwrap();
        let archive = package::build_path(&root, None).unwrap();
        let installed = package::install_path_at(&archive, &store).unwrap();
        let result = super::execute_installed_bundle(
            &installed,
            Some("demo.cli/main"),
            Some(&["--version".into(), "project".into()]),
        )
        .unwrap();
        assert_eq!(result, "[\"--version\" \"project\"]");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sealed_bundle_execution_mounts_companion_package_content_for_package_read() {
        let root = std::env::temp_dir().join(format!(
            "hara-native-package-content-{}",
            std::process::id()
        ));
        let store = root.join("store");
        let primary = root.join("primary");
        let specs = root.join("specs");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(primary.join("src/demo")).unwrap();
        fs::create_dir_all(specs.join("content")).unwrap();
        fs::write(
            primary.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/cli :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/main demo.cli :project/capabilities #{:kernel}}\n",
        )
        .unwrap();
        fs::write(
            primary.join("src/demo/cli.hal"),
            "(ns demo.cli)\n(defn main [_] (String/decode-utf8 (Package/read (Package/find \"hara:fixture/specs\") \"content/suite.edn\")))\n",
        )
        .unwrap();
        fs::write(
            specs.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id fixture/specs :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/artifact-paths [\"content\"] :project/capabilities #{:kernel}}\n",
        )
        .unwrap();
        fs::write(specs.join("content/suite.edn"), "{:suite/id :fixture/specs}\n").unwrap();

        let primary_archive = package::build_path(&primary, None).unwrap();
        let specs_archive = package::build_path(&specs, None).unwrap();
        let primary_root = package::install_path_at(&primary_archive, &store).unwrap();
        let specs_root = package::install_path_at(&specs_archive, &store).unwrap();
        let result = super::execute_installed_bundle_roots(
            &primary_root,
            &[primary_root.clone(), specs_root],
            Some("demo.cli/main"),
            Some(&[]),
        )
        .unwrap();
        assert_eq!(result, "\"{:suite/id :fixture/specs}\\n\"");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_bundle_operations() {
        let error =
            parse_arguments(["bundle".into(), "publish".into(), "x.harp".into()]).unwrap_err();
        assert!(error.contains("unknown hara-native bundle operation"));
        let error = parse_arguments([
            "bundle".into(),
            "build".into(),
            "examples/smoke-answer".into(),
            "--archive".into(),
        ])
        .unwrap_err();
        assert!(error.contains("unknown hara-native bundle build option"));
    }

    #[test]
    fn rejects_unknown_publish_options() {
        let error = parse_arguments(["publish".into(), "--archive".into()]).unwrap_err();
        assert!(error.contains("unknown hara-native publish option"));
        let error =
            parse_arguments(["publish".into(), "--tap".into(), "--dry-run".into()]).unwrap_err();
        assert!(error.contains("publish --tap requires a tap name"));
    }

    #[test]
    fn accepts_signed_tag_skip_for_a_publish_request() {
        let parsed = parse_arguments([
            "publish".into(),
            "--skip-signed-tag".into(),
            "examples/smoke-answer".into(),
        ]);
        assert!(matches!(parsed, Ok(Command::Publish)));

        assert!(matches!(
            parse_arguments([
                "publish".into(),
                "--dry-run".into(),
                "--skip-signed-tag".into(),
            ]),
            Ok(Command::Publish)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn offline_policy_grant_command_only_writes_after_the_explicit_non_dry_run() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "hara-native-policy-grant-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let root_key = root.join("root.ed25519");
        fs::write(&root_key, [9_u8; 32]).unwrap();
        fs::set_permissions(&root_key, fs::Permissions::from_mode(0o600)).unwrap();
        let root_public = signer::public_key_hex(&root_key).unwrap();
        let policy = root.join("identity.edn");
        let source = format!(
            "{{:identity/format 1 :identity/root-key \"{root_public}\" :publisher-keys {{}}}}\n"
        );
        fs::write(&policy, &source).unwrap();
        let arguments = vec![
            "policy".into(),
            "grant".into(),
            "--identity".into(),
            policy.to_string_lossy().into_owned(),
            "--root-key-file".into(),
            root_key.to_string_lossy().into_owned(),
            "--key-id".into(),
            "hoebat-2026-01".into(),
            "--public-key".into(),
            "ab".repeat(32),
            "--github-subject".into(),
            "1455572".into(),
            "--coordinate".into(),
            "hara:hara-native/smoke-answer".into(),
            "--authorization-public-key".into(),
            "cd".repeat(32),
            "--dry-run".into(),
        ];
        let result = run_id(&arguments);
        assert_eq!(fs::read_to_string(&policy).unwrap(), source);
        assert!(!root.join("identity.edn.sig").exists());
        fs::remove_dir_all(&root).unwrap();
        result.unwrap();
    }

    #[test]
    fn untagged_source_requires_a_clean_remote_default_head() {
        let root = std::env::temp_dir().join(format!(
            "hara-native-publish-untagged-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let remote = root.with_extension("origin.git");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&remote);

        let result = (|| -> Result<(), String> {
            fs::create_dir_all(&root).map_err(|error| error.to_string())?;
            test_git(&root, ["init"])?;
            test_git(&root, ["config", "user.name", "Hara Native Test"])?;
            test_git(&root, ["config", "user.email", "test@invalid"])?;
            fs::write(root.join("README"), "fixture\n").map_err(|error| error.to_string())?;
            test_git(&root, ["add", "README"])?;
            test_git(&root, ["commit", "-m", "fixture"])?;
            test_git(&root, ["branch", "-M", "main"])?;
            let remote_path = remote
                .to_str()
                .ok_or("temporary remote path is not valid UTF-8")?;
            test_git(&root, ["init", "--bare", remote_path])?;
            test_git(&root, ["remote", "add", "origin", remote_path])?;
            test_git(&root, ["push", "--set-upstream", "origin", "main"])?;
            test_git(&remote, ["symbolic-ref", "HEAD", "refs/heads/main"])?;

            let head = test_git(&root, ["rev-parse", "HEAD"])?;
            let skipped = package::source_release_commit(&root, "v0.1.0", true)?;
            if skipped != head {
                return Err(format!(
                    "untagged source resolved {skipped}, expected {head}"
                ));
            }

            fs::write(root.join("README"), "dirty\n").map_err(|error| error.to_string())?;
            let error = package::source_release_commit(&root, "v0.1.0", true).unwrap_err();
            if !error.contains("clean worktree") {
                return Err(format!("dirty worktree check failed unexpectedly: {error}"));
            }
            test_git(&root, ["checkout", "--", "README"])?;

            fs::write(root.join("UNPUSHED"), "fixture\n").map_err(|error| error.to_string())?;
            test_git(&root, ["add", "UNPUSHED"])?;
            test_git(&root, ["commit", "-m", "unpublished"])?;
            let error = package::source_release_commit(&root, "v0.1.0", true).unwrap_err();
            if !error.contains("remote default branch") {
                return Err(format!("remote head check failed unexpectedly: {error}"));
            }
            Ok(())
        })();
        let cleanup = fs::remove_dir_all(&root);
        let remote_cleanup = fs::remove_dir_all(&remote);

        cleanup.map_err(|error| error.to_string()).unwrap();
        remote_cleanup.map_err(|error| error.to_string()).unwrap();
        result.unwrap();
    }

    #[test]
    fn official_tap_fetches_its_signed_policy_from_the_policy_repository() {
        let root =
            std::env::temp_dir().join(format!("hara-native-official-tap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let fingerprint = format!("sha256:{}", "11".repeat(32));

        let result = (|| -> Result<(), String> {
            let configured = tap::bootstrap_with_official_root(&root, "hara", &fingerprint)?;
            if configured.identity.as_slice() != ["https://github.com/hara-lang/hara-identity.git"]
            {
                return Err(format!(
                    "unexpected official policy endpoint: {:?}",
                    configured.identity
                ));
            }
            if tap::load(&root)?.get("hara").map(|tap| &tap.identity) != Some(&configured.identity)
            {
                return Err("official policy endpoint was not persisted".into());
            }
            Ok(())
        })();
        let cleanup = fs::remove_dir_all(&root);

        cleanup.map_err(|error| error.to_string()).unwrap();
        result.unwrap();
    }

    #[test]
    fn enrollment_dry_run_signs_the_exact_canonical_request_in_process() {
        let args = vec![
            "--owner".into(),
            "alice".into(),
            "--tap".into(),
            "hara".into(),
            "--challenge".into(),
            "challenge-1".into(),
            "--dry-run".into(),
        ];
        let signed = RefCell::new(None);
        identity_tool::enroll_with_signer(&args, &"ab".repeat(32), |intent| {
            *signed.borrow_mut() = Some(String::from_utf8(intent.to_vec()).unwrap());
            Ok(("alice-2026".into(), "cd".repeat(64)))
        })
        .unwrap();
        assert_eq!(
            signed.into_inner(),
            Some(identity_tool::canonical_enrollment(
                "hara",
                "alice",
                &"ab".repeat(32),
                "challenge-1"
            ))
        );
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
    fn project_test_runner_discovers_files_requires_sources_and_normalizes_structured_outputs() {
        let root =
            std::env::temp_dir().join(format!("hara-native-project-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/fixture")).unwrap();
        fs::create_dir_all(root.join("test/fixture")).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id fixture/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [] :project/capabilities #{}}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/fixture/math.hal"),
            "(ns fixture.math)\n(defn advance [value] (+ value 1))\n",
        )
        .unwrap();
        fs::write(
            root.join("test/fixture/registered_test.hal"),
            "(ns fixture.registered-test (:require [fixture.math :as math]))\n(Test/register {:desc \"advance increments\" :test (fn [] (math/advance 41)) :expected 42 :meta {:refer (quote fixture.math/advance) :id (quote advance-increments)}})\n",
        )
        .unwrap();
        fs::write(
            root.join("test/fixture/check_test.hal"),
            "(ns fixture.check-test)\n(Test/check [{:desc \"first check result\" :test (fn [] (+ 20 22)) :expected 42}])\n(Test/check [{:desc \"second check result\" :test (fn [] (+ 1 1)) :expected 2}])\n",
        )
        .unwrap();

        let report = run_project_tests(&root, &[]).unwrap();
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.counts.passed, 3);
        assert_eq!(report.counts.failing(), 0);

        let selected = run_project_tests(
            &root,
            &[std::path::PathBuf::from("test/fixture/check_test.hal")],
        )
        .unwrap();
        assert_eq!(selected.files.len(), 1);
        assert_eq!(selected.counts.passed, 2);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_test_runner_bootstraps_source_owned_foundation() {
        let root = std::env::temp_dir().join(format!(
            "hara-native-foundation-project-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/std/foundation")).unwrap();
        fs::create_dir_all(root.join("test/fixture")).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id fixture/foundation :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [] :project/capabilities #{}}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/std/foundation.hal"),
            "(ns std.foundation)\n(defn root-answer [] 42)\n",
        )
        .unwrap();
        fs::write(
            root.join("src/std/foundation/string.hal"),
            "(ns std.foundation.string (:config {:set-global-alias str}))\n(defn upper [value] value)\n",
        )
        .unwrap();
        for name in ["promise", "bytes", "coroutine", "pretty"] {
            fs::write(
                root.join(format!("src/std/foundation/{name}.hal")),
                format!("(ns std.foundation.{name})\n(def loaded true)\n"),
            )
            .unwrap();
        }
        fs::write(
            root.join("test/fixture/foundation_test.hal"),
            "(ns fixture.foundation-test (:require [std.foundation :as foundation] [std.foundation.string :as str]))\n(Test/register {:desc \"loads source Foundation\" :test (fn [] [(foundation/root-answer) (str/upper \"hara\")]) :expected [42 \"hara\"] :meta {:refer (quote std.foundation/root-answer) :id (quote source-foundation)}})\n",
        )
        .unwrap();

        let report = run_project_tests(&root, &[]).unwrap();
        assert_eq!(report.counts.passed, 1);
        assert_eq!(report.counts.failing(), 0);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_test_runner_rejects_plain_values_and_files_outside_test_paths() {
        let root = std::env::temp_dir().join(format!(
            "hara-native-project-test-rejection-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/fixture")).unwrap();
        fs::create_dir_all(root.join("test/fixture")).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id fixture/rejection :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [] :project/capabilities #{}}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/fixture/plain.hal"),
            "(ns fixture.plain)\n42\n",
        )
        .unwrap();
        fs::write(
            root.join("test/fixture/plain_test.hal"),
            "(ns fixture.plain-test)\n42\n",
        )
        .unwrap();

        let error = run_project_tests(&root, &[]).unwrap_err();
        assert!(error.contains("project tests failed"));
        let error = run_project_tests(&root, &[std::path::PathBuf::from("src/fixture/plain.hal")])
            .unwrap_err();
        assert!(error.contains("not a .hal file beneath this project's test paths"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_test_runner_reserves_top_level_test_run_for_the_cli() {
        let error = reject_top_level_test_run(
            "(ns fixture.legacy-test)\n^{:refer fixture/legacy}\n(Test/run)\n",
        )
        .unwrap_err();
        assert!(error.contains("Test/run is runner-owned"));
        assert!(error.contains("line 2"));
        assert!(reject_top_level_test_run(
            "(ns fixture.helper)\n(defn invoke-runner [] (Test/run))\n",
        )
        .is_ok());
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
