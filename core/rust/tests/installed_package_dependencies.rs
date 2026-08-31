use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "hara-installed-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn run(dist: &Path, args: &[&str], input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hara"));
    command.env("HARA_DIST_HOME", dist).args(args);
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn success(dist: &Path, args: &[&str], input: Option<&str>) -> String {
    let output = run(dist, args, input);
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn write_package(
    root: &Path,
    id: &str,
    version: &str,
    namespace: &str,
    source: &str,
    dependencies: &[(&str, &str)],
) {
    let path = root
        .join("src")
        .join(format!("{}.hal", namespace.replace('.', "/")));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
    let dependencies = dependencies
        .iter()
        .map(|(coordinate, requirement)| format!(" {coordinate} {{:version \"{requirement}\"}}"))
        .collect::<String>();
    fs::write(
        root.join("project.edn"),
        format!(
            "{{:hara/type :project :hara/version \"1.0.0\" :project/id {id} :project/version \"{version}\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{{}} :project/dependencies {{{dependencies}}}}}\n"
        ),
    )
    .unwrap();
    if !dependencies.is_empty() {
        fs::write(
            root.join("project.lock.edn"),
            "{:lock/format \"0.0.1\" :packages {}}\n",
        )
        .unwrap();
    }
}

fn install(dist: &Path, package: &Path) {
    let archive = package.join("target/package.harp");
    success(
        dist,
        &[
            "package",
            "build",
            package.to_str().unwrap(),
            "--output",
            archive.to_str().unwrap(),
        ],
        None,
    );
    success(
        dist,
        &["package", "install", archive.to_str().unwrap()],
        None,
    );
}

fn write_consumer(root: &Path, dependencies: &[(&str, &str)], require: &str, call: &str) {
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::create_dir_all(root.join("test/demo")).unwrap();
    let dependencies = dependencies
        .iter()
        .map(|(coordinate, requirement)| format!(" {coordinate} {{:version \"{requirement}\"}}"))
        .collect::<String>();
    fs::write(
        root.join("project.edn"),
        format!(
            "{{:hara/type :project :hara/version \"1.0.0\" :project/id demo/consumer :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [] :project/capabilities #{{}} :project/dependencies {{{dependencies}}}}}\n"
        ),
    )
    .unwrap();
    fs::write(
        root.join("src/demo/app.hal"),
        format!("(ns demo.app (:require [{require} :as dependency])) (defn answer [] ({call}))"),
    )
    .unwrap();
    fs::write(
        root.join("test/demo/app_test.hal"),
        "(ns demo.app-test (:require [demo.app :as app])) (std.lib.test/print-results [(std.lib.test/check \"installed package is active\" (app/answer) 42)])",
    )
    .unwrap();
}

#[test]
fn activates_highest_matching_transitive_installed_package() {
    let root = temp("activation");
    let dist = root.join("dist");
    let core_010 = root.join("core-0.1.0");
    let core_020 = root.join("core-0.2.0");
    let adapter = root.join("adapter");
    let consumer = root.join("consumer");

    write_package(
        &core_010,
        "greenways/historia-core",
        "0.1.0",
        "historia.core.api",
        "(ns historia.core.api) (defn answer [] 41)",
        &[],
    );
    write_package(
        &core_020,
        "greenways/historia-core",
        "0.2.0",
        "historia.core.api",
        "(ns historia.core.api) (defn answer [] 42)",
        &[],
    );
    write_package(
        &adapter,
        "greenways/historia-adapter",
        "0.1.0",
        "historia.adapter",
        "(ns historia.adapter (:require [historia.core.api :as core])) (defn answer [] (core/answer))",
        &[("greenways/historia-core", ">=0.1.0, <1.0.0")],
    );
    install(&dist, &core_010);
    install(&dist, &core_020);
    install(&dist, &adapter);
    write_consumer(
        &consumer,
        &[("greenways/historia-adapter", "^0.1.0")],
        "historia.adapter",
        "dependency/answer",
    );

    success(
        &dist,
        &["--project", consumer.to_str().unwrap(), "check"],
        None,
    );
    success(
        &dist,
        &["--project", consumer.to_str().unwrap(), "test"],
        None,
    );
    let evaluated = success(
        &dist,
        &[
            "--project",
            consumer.to_str().unwrap(),
            "eval",
            "(ns demo.probe (:require [demo.app :as app])) (app/answer)",
        ],
        None,
    );
    assert!(evaluated.lines().any(|line| line.trim() == "42"));
    let repl = success(
        &dist,
        &[
            "--project",
            consumer.to_str().unwrap(),
            "--offline",
            "--no-splash",
            "--no-history",
            "repl",
        ],
        Some("(ns demo.repl (:require [demo.app :as app]))\n(app/answer)\n/quit\n"),
    );
    assert!(repl.contains("42"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_conflicting_and_cyclic_installed_dependencies() {
    let root = temp("errors");
    let dist = root.join("dist");
    let missing = root.join("missing");
    write_consumer(
        &missing,
        &[("greenways/missing", "^1.0.0")],
        "greenways.missing",
        "dependency/answer",
    );
    let output = run(
        &dist,
        &[
            "--project",
            missing.to_str().unwrap(),
            "eval",
            "(require [demo.app])",
        ],
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no installed version"));

    let core_015 = root.join("core-0.1.5");
    let core_025 = root.join("core-0.2.5");
    let left = root.join("left");
    let right = root.join("right");
    let conflict = root.join("conflict");
    write_package(
        &core_015,
        "greenways/core",
        "0.1.5",
        "greenways.core",
        "(ns greenways.core)",
        &[],
    );
    write_package(
        &core_025,
        "greenways/core",
        "0.2.5",
        "greenways.core",
        "(ns greenways.core)",
        &[],
    );
    write_package(
        &left,
        "greenways/left",
        "1.0.0",
        "greenways.left",
        "(ns greenways.left)",
        &[("greenways/core", "~0.1.0")],
    );
    write_package(
        &right,
        "greenways/right",
        "1.0.0",
        "greenways.right",
        "(ns greenways.right)",
        &[("greenways/core", "~0.2.0")],
    );
    for package in [&core_015, &core_025, &left, &right] {
        install(&dist, package);
    }
    write_consumer(
        &conflict,
        &[("greenways/left", "^1.0.0"), ("greenways/right", "^1.0.0")],
        "greenways.left",
        "dependency/missing",
    );
    let output = run(
        &dist,
        &[
            "--project",
            conflict.to_str().unwrap(),
            "eval",
            "(require [demo.app])",
        ],
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("no installed version of hara:greenways/core"));

    let cycle_a = root.join("cycle-a");
    let cycle_b = root.join("cycle-b");
    let cycle = root.join("cycle-consumer");
    write_package(
        &cycle_a,
        "greenways/cycle-a",
        "1.0.0",
        "greenways.cycle-a",
        "(ns greenways.cycle-a)",
        &[("greenways/cycle-b", "^1.0.0")],
    );
    write_package(
        &cycle_b,
        "greenways/cycle-b",
        "1.0.0",
        "greenways.cycle-b",
        "(ns greenways.cycle-b)",
        &[("greenways/cycle-a", "^1.0.0")],
    );
    install(&dist, &cycle_a);
    install(&dist, &cycle_b);
    write_consumer(
        &cycle,
        &[("greenways/cycle-a", "^1.0.0")],
        "greenways.cycle-a",
        "dependency/missing",
    );
    let output = run(
        &dist,
        &[
            "--project",
            cycle.to_str().unwrap(),
            "eval",
            "(require [demo.app])",
        ],
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dependency cycle"));
    fs::remove_dir_all(root).unwrap();
}
