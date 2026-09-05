use hara_native::{
    kernel::{halc, parse_forms},
    package::{build_artifact, ArtifactFile, ArtifactSpec},
    vm::{compile_bytecode_bundle, ModuleSource},
    work::plan::WorkPlan,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

const ASSET_DIRECTORY: &str = "assets/host-fixtures";
const HBX_FIXTURE: &str = "portable-module.hbx";
const HALC_FIXTURE: &str = "portable-module.halc";
const HTA_FIXTURE: &str = "portable-work-plan.hta";
const HARP_FIXTURE: &str = "portable-package.harp";

const HBX_SOURCE: &str = "(ns fixture.hbx) (def answer 42)";
const HALC_SOURCE: &str = "(ns fixture.halc) (def answer 42)";
const HARP_SOURCE: &str = "(ns fixture.harp) (def answer 42)";
const HARP_PROJECT: &str = "{:hara/type :project :hara/version \"1.0.0\" :project/id fixture/rust-host-fixtures :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}\n";

struct Fixture {
    name: &'static str,
    bytes: Vec<u8>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-native-host-fixtures: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = env::args().nth(1).unwrap_or_else(|| "check".into());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = fixtures()?;
    match command.as_str() {
        "generate" => write_fixtures(&root, &fixtures),
        "check" => check_fixtures(&root, &fixtures),
        _ => Err("usage: hara-native-host-fixtures [generate|check]".into()),
    }
}

fn fixtures() -> Result<Vec<Fixture>, String> {
    let hbx = compile_bytecode_bundle(&[ModuleSource {
        resource: "fixture.hbx",
        source: HBX_SOURCE,
    }])?;
    let halc = halc::encode_halc_module(
        "fixture.halc",
        "fixture/halc.hal",
        HALC_SOURCE,
        parse_forms(HALC_SOURCE)?,
    )?;
    let hta = WorkPlan::pure("fixture/answer")?.encode_hta()?;
    let harp = build_harp_fixture()?;
    Ok(vec![
        Fixture {
            name: HBX_FIXTURE,
            bytes: hbx,
        },
        Fixture {
            name: HALC_FIXTURE,
            bytes: halc,
        },
        Fixture {
            name: HTA_FIXTURE,
            bytes: hta,
        },
        Fixture {
            name: HARP_FIXTURE,
            bytes: harp,
        },
    ])
}

fn write_fixtures(root: &Path, fixtures: &[Fixture]) -> Result<(), String> {
    let directory = root.join(ASSET_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    for fixture in fixtures {
        let path = directory.join(fixture.name);
        fs::write(&path, &fixture.bytes).map_err(|error| error.to_string())?;
        println!("wrote {} bytes to {}", fixture.bytes.len(), path.display());
    }
    Ok(())
}

fn check_fixtures(root: &Path, fixtures: &[Fixture]) -> Result<(), String> {
    let directory = root.join(ASSET_DIRECTORY);
    for fixture in fixtures {
        let path = directory.join(fixture.name);
        let tracked = fs::read(&path).map_err(|error| error.to_string())?;
        if tracked != fixture.bytes {
            return Err(format!("{} is stale; run with generate", path.display()));
        }
        println!("{} is current ({} bytes)", path.display(), tracked.len());
    }
    Ok(())
}

fn build_harp_fixture() -> Result<Vec<u8>, String> {
    let root = env::temp_dir().join(format!("hara-native-host-fixtures-{}", process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
    }
    let result = (|| {
        let project = root.join("project");
        let source = project.join("src/fixture/harp/main.hal");
        let archive = root.join(HARP_FIXTURE);
        fs::create_dir_all(source.parent().expect("fixture source has parent"))
            .map_err(|error| error.to_string())?;
        fs::write(project.join("project.edn"), HARP_PROJECT).map_err(|error| error.to_string())?;
        fs::write(source, HARP_SOURCE).map_err(|error| error.to_string())?;
        build_artifact(
            ArtifactSpec {
                identity: "hara:fixture/rust-host-fixtures".into(),
                version: "1.0.0".into(),
                name: None,
                files: vec![
                    ArtifactFile {
                        path: "project.edn".into(),
                        bytes: HARP_PROJECT.as_bytes().to_vec(),
                    },
                    ArtifactFile {
                        path: "src/fixture/harp/main.hal".into(),
                        bytes: HARP_SOURCE.as_bytes().to_vec(),
                    },
                ],
                resources: [("fixture.harp".into(), "src/fixture/harp/main.hal".into())]
                    .into_iter()
                    .collect(),
                bytecode: None,
                extensions: "{}".into(),
            },
            &archive,
        )?;
        fs::read(archive).map_err(|error| error.to_string())
    })();
    let cleanup = fs::remove_dir_all(&root).map_err(|error| error.to_string());
    match (result, cleanup) {
        (Ok(bytes), Ok(())) => Ok(bytes),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}
