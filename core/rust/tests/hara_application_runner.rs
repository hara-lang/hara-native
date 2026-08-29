use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "hara-app-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn run(project: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hara-app"))
        .arg("--project")
        .arg(project)
        .arg("--allow-file")
        .arg("--allow-process")
        .arg("--")
        .args(arguments)
        .output()
        .unwrap()
}

fn write_project(root: &Path, source: &str) {
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::create_dir_all(root.join("packages/core/src/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"packages/core/src\" \"src\"] :project/test-paths [] :project/extension-paths [] :project/main demo.main :project/capabilities #{:file :process}}",
    )
    .unwrap();
    fs::write(
        root.join("packages/core/src/demo/helper.hal"),
        "(ns demo.helper) (defn marker [] \"helper-loaded\")",
    )
    .unwrap();
    fs::write(root.join("src/demo/main.hal"), source).unwrap();
}

#[test]
fn forwards_request_and_emits_the_application_result() {
    let root = temp("result");
    write_project(
        &root,
        "(ns demo.main (:require [demo.helper :as helper] [hara.cli.application :as application]))\n{:hara.app/stdout (str application/request) :hara.app/stderr (helper/marker) :hara.app/exit 7}",
    );
    let output = run(&root, &["alpha", "beta"]);
    assert_eq!(output.status.code(), Some(7));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains(":hara.cli/arguments [\"alpha\" \"beta\"]"));
    assert!(stdout.contains(":hara.cli/project \"demo/app\""));
    assert!(stdout.contains(":hara.cli/capabilities #{:file :process}"));
    assert_eq!(stderr, "helper-loaded");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prints_plain_values_and_rejects_malformed_envelopes() {
    let plain = temp("plain");
    write_project(&plain, "(ns demo.main)\n42");
    let output = run(&plain, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
    fs::remove_dir_all(plain).unwrap();

    let malformed = temp("malformed");
    write_project(
        &malformed,
        "(ns demo.main)\n{:hara.app/stdout 1 :hara.app/exit 0}",
    );
    let output = run(&malformed, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains(":hara.app/stdout"));
    fs::remove_dir_all(malformed).unwrap();
}
