use super::*;
use crate::package_manifest::{
    PackageArtifactType, PackageManifest, PackageRuntime, PackageRuntimeRequirements,
    PackageSelection,
};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hara-package-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id example/app :project/version \"1.2.3\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [\"extensions\"] :project/capabilities #{} :project/dependencies {\"hara:hara/graph\" {:version \"^1.2.0\"}}}").unwrap();
    fs::write(
        root.join("project.lock.edn"),
        "{:lock/format \"0.0.1\" :packages {}}\n",
    )
    .unwrap();
    root
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn write_archive(path: &Path, entries: &[(String, Vec<u8>)]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, bytes) in entries {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn runtime_archive(
    root: &Path,
    declared_artifact: &[u8],
    archived_artifact: &[u8],
    extra_file: bool,
) -> PathBuf {
    let project = br#"{:hara/type :project
 :hara/version "1.0.0"
 :project/id "hara:example/provider"
 :project/version "1.0.0"
 :project/source-paths []
 :project/test-paths []
 :project/extension-paths []
 :project/capabilities #{}
 :project/dependencies {}}
"#;
    let project_digest = digest(project);
    let artifact_digest = digest(declared_artifact);
    let package = format!(
        r#"{{:harp/format "0.0.0-alpha"
 :package {{:identity "hara:example/provider"
           :version "1.0.0"
           :provenance
           {{:repository "https://github.com/example/provider"
            :commit "0123456789abcdef0123456789abcdef01234567"}}}}
 :files
 {{"artifacts/provider.hta" {{:sha256 "{artifact_digest}" :size {artifact_size}}}
  "project.edn" {{:sha256 "{project_digest}" :size {project_size}}}}}
 :wasm-imports
 {{:provider
   {{:variant/artifact
     {{:artifact/type :hta
      :artifact/path "artifacts/provider.hta"
      :artifact/sha256 "{artifact_digest}"
      :artifact/target "wasm32-wasi-preview1"
      :artifact/abi "hta.v1"
      :artifact/entry-point "provider_init"}}
    :variant/required-capabilities #{{:db/connect}}
    :variant/host-calls #{{:db/socket}}
    :variant/exports #{{:provider/open :provider/close}}
    :variant/dependencies {{}}
    :variant/lifecycle
    {{:lifecycle/load :idempotent
     :lifecycle/close :idempotent
     :lifecycle/session-isolation true
     :lifecycle/async true
     :lifecycle/cancellation true}}}}}}}}"#,
        artifact_size = declared_artifact.len(),
        project_size = project.len()
    );
    let archive = root.join(format!(
        "runtime-{}.harp",
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut entries = vec![
        ("package.edn".to_owned(), package.into_bytes()),
        (
            "artifacts/provider.hta".to_owned(),
            archived_artifact.to_vec(),
        ),
        ("project.edn".to_owned(), project.to_vec()),
    ];
    if extra_file {
        entries.push(("hidden/payload.bin".to_owned(), b"hidden".to_vec()));
    }
    write_archive(&archive, &entries);
    archive
}

fn wasm_requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        supported_targets: ["wasm32-wasi-preview1".to_owned()].into_iter().collect(),
        supported_abis: ["hta.v1".to_owned()].into_iter().collect(),
        available_capabilities: ["db/connect".to_owned()].into_iter().collect(),
        allowed_host_calls: ["db/socket".to_owned()].into_iter().collect(),
    }
}

#[test]
fn generic_artifact_builder_only_assembles_supplied_bytes() {
    let root = fixture();
    let archive = root.join("generic.harp");
    let output = build_artifact(
        ArtifactSpec {
            identity: "hara:example/generic".into(),
            version: "1.0.0".into(),
            name: Some("generic".into()),
            files: vec![ArtifactFile {
                path: "payload.bin".into(),
                bytes: b"payload".to_vec(),
            }],
            resources: [("demo.payload".into(), "payload.bin".into())]
                .into_iter()
                .collect(),
            bytecode: None,
            extensions: "{}".into(),
        },
        &archive,
    )
    .unwrap();
    assert_eq!(output, archive);
    let manifest = PackageManifest::read_archive(&archive).unwrap();
    assert_eq!(manifest.identity, "hara:example/generic");
    assert_eq!(manifest.name.as_deref(), Some("generic"));
    assert_eq!(manifest.resources["demo.payload"], Path::new("payload.bin"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_artifact_builder_rejects_unsafe_and_duplicate_paths() {
    let root = fixture();
    let unsafe_error = build_artifact(
        ArtifactSpec {
            identity: "hara:example/generic".into(),
            version: "1.0.0".into(),
            name: None,
            files: vec![ArtifactFile {
                path: "../payload.bin".into(),
                bytes: b"payload".to_vec(),
            }],
            resources: BTreeMap::new(),
            bytecode: None,
            extensions: "{}".into(),
        },
        &root.join("unsafe.harp"),
    )
    .unwrap_err();
    assert!(unsafe_error.contains("unsafe package archive path"));

    let duplicate_error = build_artifact(
        ArtifactSpec {
            identity: "hara:example/generic".into(),
            version: "1.0.0".into(),
            name: None,
            files: vec![
                ArtifactFile {
                    path: "payload.bin".into(),
                    bytes: b"one".to_vec(),
                },
                ArtifactFile {
                    path: "payload.bin".into(),
                    bytes: b"two".to_vec(),
                },
            ],
            resources: BTreeMap::new(),
            bytecode: None,
            extensions: "{}".into(),
        },
        &root.join("duplicate.harp"),
    )
    .unwrap_err();
    assert!(duplicate_error.contains("duplicate package archive path"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_artifacts_install_without_a_project_declaration() {
    let root = fixture();
    let archive = root.join("generic-install.harp");
    build_artifact(
        ArtifactSpec {
            identity: "hara:example/generic-install".into(),
            version: "1.0.0".into(),
            name: None,
            files: vec![ArtifactFile {
                path: "payload.bin".into(),
                bytes: b"payload".to_vec(),
            }],
            resources: [("demo.payload".into(), "payload.bin".into())]
                .into_iter()
                .collect(),
            bytecode: None,
            extensions: "{}".into(),
        },
        &archive,
    )
    .unwrap();
    let distribution = root.join("generic-install-dist");
    let installed = install_archive_at(&archive, &distribution).unwrap();
    assert_eq!(fs::read(installed.join("payload.bin")).unwrap(), b"payload");
    assert!(distribution
        .join("packages/hara/example/generic-install/1.0.0.edn")
        .is_file());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "bytecode-vm")]
#[test]
fn precompile_accepts_only_caller_supplied_module_plans() {
    let modules = vec![PrecompileModule {
        namespace: "demo.main".into(),
        source: "(ns demo.main) (def answer 42)".into(),
    }];
    let bytes = precompile(&modules, &modules).unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(bytes, precompile(&modules, &modules).unwrap());
}

#[test]
fn rejects_missing_project_keys_and_bad_ranges() {
    let root = fixture();
    fs::write(root.join("project.edn"), "{:hara/type :project}").unwrap();
    assert!(read_project(&root).unwrap_err().contains(":hara/version"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn validates_typed_recipes_and_installs_content_addressed_roots() {
    let root = fixture();
    fs::write(root.join("project.receipe.edn"), "{:recipe/format \"0.0.0-alpha\" :recipe/adapter :hal :recipe/toolchain {} :recipe/inputs {} :recipe/outputs []}\n").unwrap();
    let source = fs::read_to_string(root.join("project.edn")).unwrap();
    fs::write(
        root.join("project.edn"),
        source.trim().strip_suffix('}').unwrap().to_owned()
            + " :project/recipe \"project.receipe.edn\"}\n",
    )
    .unwrap();
    let project = read_project(&root).unwrap();
    assert_eq!(
        validate_recipe(&project).unwrap(),
        root.join("project.receipe.edn")
    );
    let archive = root.join("package.harp");
    build_artifact(
        ArtifactSpec {
            identity: "hara:example/app".into(),
            version: "1.2.3".into(),
            name: None,
            files: vec![
                ArtifactFile {
                    path: "project.edn".into(),
                    bytes: fs::read(root.join("project.edn")).unwrap(),
                },
                ArtifactFile {
                    path: "project.receipe.edn".into(),
                    bytes: fs::read(root.join("project.receipe.edn")).unwrap(),
                },
            ],
            resources: BTreeMap::new(),
            bytecode: None,
            extensions: "{}".into(),
        },
        &archive,
    )
    .unwrap();
    let dist = root.join("dist");
    let installed = install_archive_at(&archive, &dist).unwrap();
    assert!(installed.join("project.receipe.edn").is_file());
    assert!(dist.join("packages/hara/example/app/1.2.3.edn").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_errors_include_the_registry_problem_body() {
    assert_eq!(
        publication_request_error(
            b"curl: (22) The requested URL returned error: 400\n",
            b"{\"error\":{\"code\":\"PUBLICATION_REJECTED\",\"message\":\"Publication authorization signature is invalid\"}}\n",
        ),
        "publication request failed: curl: (22) The requested URL returned error: 400: {\"error\":{\"code\":\"PUBLICATION_REJECTED\",\"message\":\"Publication authorization signature is invalid\"}}"
    );
}

#[test]
fn publication_error_diagnostics_are_bounded() {
    let response = vec![b'x'; MAX_PUBLICATION_DIAGNOSTIC_BYTES + 1];
    let diagnostic = publication_request_error(b"", &response);
    assert_eq!(
        diagnostic.len(),
        "publication request failed: ".len() + MAX_PUBLICATION_DIAGNOSTIC_BYTES + "…".len()
    );
    assert!(diagnostic.ends_with('…'));
}

#[test]
fn direct_package_publication_requires_the_source_workflow() {
    let error = run(&["publish".into()]).unwrap_err();
    assert!(error.contains("publication-github-workflow-required"));
}

#[test]
fn selects_verified_runtime_archive_before_and_after_installation() {
    let root = fixture();
    let archive = runtime_archive(&root, b"wasm", b"wasm", false);
    let verified =
        PackageManifest::select_hta_require_archive(&archive, "provider", &wasm_requirements())
            .unwrap();
    let PackageSelection::Variant(variant) = &verified.selection else {
        panic!("expected Wasm import");
    };
    assert_eq!(variant.artifact.artifact_type, PackageArtifactType::Hta);
    assert_eq!(variant.artifact.path, Path::new("artifacts/provider.hta"));

    let dist = root.join("runtime-dist");
    let installed = install_archive_at(&archive, &dist).unwrap();
    let installed_manifest = PackageManifest::read(&installed.join("package.edn")).unwrap();
    let installed_selection = installed_manifest
        .select_hta_require("provider", &wasm_requirements())
        .unwrap();
    assert_eq!(installed_selection, verified.selection);
    assert!(dist
        .join("packages/hara/example/provider/1.0.0.edn")
        .is_file());

    let error = PackageManifest::select_archive(
        &archive,
        PackageRuntime::Jvm,
        &PackageRuntimeRequirements::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "package/missing-flavor");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_tampered_and_undeclared_archive_payloads_before_activation() {
    let root = fixture();
    let tampered = runtime_archive(&root, b"wasm", b"evil", false);
    let error = PackageManifest::read_archive(&tampered).unwrap_err();
    assert_eq!(error.code, "package/digest-mismatch");
    assert!(inspect_path(&tampered)
        .unwrap_err()
        .contains("package/digest-mismatch"));

    let dist = root.join("tampered-dist");
    let install_error = install_archive_at(&tampered, &dist).unwrap_err();
    assert!(install_error.contains("package/digest-mismatch"));
    let archive_cache = dist.join("archives/sha256");
    assert!(!archive_cache.exists() || fs::read_dir(archive_cache).unwrap().next().is_none());
    let roots = dist.join("roots/sha256");
    assert!(!roots.exists() || fs::read_dir(roots).unwrap().next().is_none());

    let undeclared = runtime_archive(&root, b"wasm", b"wasm", true);
    let error = PackageManifest::read_archive(&undeclared).unwrap_err();
    assert_eq!(error.code, "package/invalid-manifest");
    assert!(error.detail.contains("undeclared file"));
    let undeclared_dist = root.join("undeclared-dist");
    assert!(install_archive_at(&undeclared, &undeclared_dist)
        .unwrap_err()
        .contains("undeclared file"));
    assert!(!undeclared_dist.join("packages").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_tampering_in_an_existing_content_addressed_root() {
    let root = fixture();
    let archive = runtime_archive(&root, b"wasm", b"wasm", false);
    let dist = root.join("existing-dist");
    let installed = install_archive_at(&archive, &dist).unwrap();
    fs::write(installed.join("artifacts/provider.hta"), b"evil").unwrap();
    let error = install_archive_at(&archive, &dist).unwrap_err();
    assert!(error.contains("package/digest-mismatch") || error.contains("package/size-mismatch"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_shell_recipe_escape_hatches() {
    let root = fixture();
    fs::write(root.join("project.receipe.edn"), "{:recipe/format \"0.0.0-alpha\" :recipe/adapter :hal :recipe/toolchain {} :recipe/inputs {:command [\"sh\"]} :recipe/outputs []}\n").unwrap();
    let source = fs::read_to_string(root.join("project.edn")).unwrap();
    fs::write(
        root.join("project.edn"),
        source.trim().strip_suffix('}').unwrap().to_owned()
            + " :project/recipe \"project.receipe.edn\"}\n",
    )
    .unwrap();
    assert!(validate_recipe(&read_project(&root).unwrap())
        .unwrap_err()
        .contains("cannot declare commands"));
    fs::remove_dir_all(root).unwrap();
}
