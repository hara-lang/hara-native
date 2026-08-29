use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "hara-external-cli-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn boots_external_project_without_repository_or_lite_project() {
    let root = temp("help");
    let project = root.join("consumer");
    let unrelated = root.join("unrelated");
    fs::create_dir_all(project.join("src/demo")).unwrap();
    fs::create_dir_all(project.join("src/tool/cli")).unwrap();
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(
        project.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/consumer :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {}}\n",
    )
    .unwrap();
    fs::write(
        project.join("src/demo/app.hal"),
        "(ns demo.app)\n\n(defn answer [] 42)\n",
    )
    .unwrap();
    fs::write(
        project.join("src/tool/cli/main.hal"),
        "(ns tool.cli.main)\n\n(defn main [& _] (throw (ex-info \"consumer CLI shadowed Hara\" {})))\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_hara"))
        .current_dir(&unrelated)
        .env_remove("HARA_LITE_PROJECT")
        .env("HARA_DIST_HOME", root.join("dist"))
        .args(["--project", project.to_str().unwrap(), "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "external CLI bootstrap failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Hara CLI"));
    fs::remove_dir_all(root).unwrap();
}
