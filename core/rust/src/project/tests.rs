use super::*;
use crate::{
    package::{self, ArtifactFile, ArtifactSpec},
    package_manifest::PackageManifest,
};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "hara-project-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn build_package_archive(
    root: &Path,
    archive: &Path,
    identity: &str,
    version: &str,
    files: &[&str],
    resources: Vec<(String, String)>,
) {
    let files = files
        .iter()
        .map(|path| ArtifactFile {
            path: (*path).to_owned(),
            bytes: fs::read(root.join(path)).unwrap(),
        })
        .collect();
    package::build_artifact(
        ArtifactSpec {
            identity: identity.to_owned(),
            version: version.to_owned(),
            name: None,
            files,
            resources: resources.into_iter().collect::<BTreeMap<_, _>>(),
            bytecode: None,
            extensions: "{}".into(),
        },
        archive,
    )
    .unwrap();
}

#[test]
fn scaffolds_discovers_and_edits_dependencies() {
    let root = temp("app");
    let project = new_app(&root, "hello-app").unwrap();
    assert_eq!(
        discover(&root.join("src/hello_app")).unwrap().id,
        "hello-app"
    );
    set_dependency(&project, "hara:hara/graph", Some("^1.2.0")).unwrap();
    assert_eq!(
        read(&root).unwrap().dependencies["hara:hara/graph"],
        "^1.2.0"
    );
    set_dependency(&project, "hara:hara/graph", None).unwrap();
    assert!(read(&root).unwrap().dependencies.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_escaping_source_paths() {
    let root = temp("unsafe");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1\" :project/id x :project/version \"1.0.0\" :project/source-paths [\"../src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}").unwrap();
    assert!(read(&root).unwrap_err().contains("cannot escape"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_tag_defaults_to_the_bare_project_version_and_allows_an_override() {
    let root = temp("release-tag");
    fs::create_dir_all(&root).unwrap();
    let base = "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.2.3\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}";
    fs::write(root.join("project.edn"), base).unwrap();
    assert_eq!(read(&root).unwrap().release_tag, "1.2.3");
    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned() + " :project/release-tag \"demo-app-1.2.3\"}",
    )
    .unwrap();
    assert_eq!(read(&root).unwrap().release_tag, "demo-app-1.2.3");
    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned() + " :project/release-tag \"v1..2.3\"}",
    )
    .unwrap();
    assert!(read(&root).unwrap_err().contains("release-tag"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_native_is_an_exact_host_version_requirement() {
    let root = temp("native-requirement");
    fs::create_dir_all(&root).unwrap();
    let base = "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.2.3\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}";
    let host = env!("CARGO_PKG_VERSION");
    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned()
            + &format!(" :project/native {{:version \"{host}\"}}}}"),
    )
    .unwrap();
    let project = read(&root).unwrap();
    assert_eq!(project.native.unwrap().version.to_string(), host);

    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned() + " :project/native {:version \"^0.1.14\"}}",
    )
    .unwrap();
    assert!(read(&root)
        .unwrap_err()
        .contains(":project/native :version must be exact SemVer"));

    let incompatible = if host == "0.0.0" { "0.0.1" } else { "0.0.0" };
    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned()
            + &format!(" :project/native {{:version \"{incompatible}\"}}}}"),
    )
    .unwrap();
    assert!(read(&root).unwrap_err().contains("requires hara-native"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn creates_and_validates_an_empty_lock() {
    let root = temp("lock");
    let project = new_app(&root, "lock-app").unwrap();
    let lock = sync_lock(&project, LockMode::Default).unwrap();
    assert_eq!(
        fs::read_to_string(&lock).unwrap(),
        "{:lock/format \"0.0.1\" :packages {}}\n"
    );
    sync_lock(&project, LockMode::Frozen).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn registers_project_sources_for_cross_file_requires() {
    let root = temp("resources");
    fs::create_dir_all(root.join("packages/core/src/demo")).unwrap();
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"packages/core/src\" \"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}").unwrap();
    fs::write(
        root.join("packages/core/src/demo/helper.hal"),
        "(ns demo.helper) (defn answer [] 40)",
    )
    .unwrap();
    fs::write(
        root.join("src/demo/app.hal"),
        "(ns demo.app (:require [demo.helper :as helper])) (defn answer [] (+ 2 (helper/answer)))",
    )
    .unwrap();
    let project = read(&root).unwrap();
    assert_eq!(
        source_resources(&project)
            .unwrap()
            .into_iter()
            .map(|(namespace, _)| namespace)
            .collect::<Vec<_>>(),
        vec!["demo.helper".to_owned(), "demo.app".to_owned()]
    );
    let mut runtime = Runtime::new();
    register_sources(&project, &mut runtime).unwrap();
    assert_eq!(
        runtime
            .eval_native("(ns demo.main (:require [demo.app :as app])) (app/answer)")
            .unwrap(),
        "42"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checkout_dependency_resolves_without_an_installed_registration() {
    let root = temp("checkout-source");
    let checkout = root.join("checkouts/demo-lib");
    let distribution = root.join("distribution");
    fs::create_dir_all(checkout.join("src/demo")).unwrap();
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {\"hara:demo/lib\" {:version \"=1.0.0\"}}}",
    )
    .unwrap();
    fs::write(
        checkout.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/lib\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        checkout.join("src/demo/lib.hal"),
        "(ns demo.lib) (defn answer [] 42)",
    )
    .unwrap();

    let project = read(&root).unwrap();
    let catalog = source_catalog_at(&project, &distribution).unwrap();
    assert_eq!(
        catalog.path("demo.lib"),
        Some(checkout.join("src/demo/lib.hal").canonicalize().unwrap())
    );
    let mut runtime = Runtime::core();
    runtime.register_source_catalog(&catalog);
    assert_eq!(
        runtime
            .eval_native("(ns demo.app (:require [demo.lib :as lib])) (lib/answer)")
            .unwrap(),
        "42"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checkout_dependency_overrides_an_installed_package() {
    let root = temp("checkout-precedence");
    let installed = root.join("installed");
    let package = root.join("package");
    let consumer = root.join("consumer");
    let checkout = consumer.join("checkouts/demo-lib");
    fs::create_dir_all(package.join("src/demo")).unwrap();
    fs::create_dir_all(checkout.join("src/demo")).unwrap();
    fs::create_dir_all(&consumer).unwrap();
    fs::write(
        package.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/lib\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        package.join("src/demo/lib.hal"),
        "(ns demo.lib) (defn answer [] :installed)",
    )
    .unwrap();
    fs::write(
        checkout.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/lib\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        checkout.join("src/demo/lib.hal"),
        "(ns demo.lib) (defn answer [] :checkout)",
    )
    .unwrap();
    fs::write(
        consumer.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {\"hara:demo/lib\" {:version \"=1.0.0\"}}}",
    )
    .unwrap();

    let archive = package.join("target/lib.harp");
    build_package_archive(
        &package,
        &archive,
        "hara:demo/lib",
        "1.0.0",
        &["project.edn", "src/demo/lib.hal"],
        vec![("demo.lib".into(), "src/demo/lib.hal".into())],
    );
    package::install_path_at(&archive, &installed).unwrap();
    let project = read(&consumer).unwrap();
    let catalog = source_catalog_at(&project, &installed).unwrap();
    assert_eq!(
        catalog.path("demo.lib"),
        Some(checkout.join("src/demo/lib.hal").canonicalize().unwrap())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checkout_dependency_rejects_a_version_mismatch() {
    let root = temp("checkout-version");
    let checkout = root.join("checkouts/demo-lib");
    fs::create_dir_all(checkout.join("src/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {\"hara:demo/lib\" {:version \"=2.0.0\"}}}",
    )
    .unwrap();
    fs::write(
        checkout.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/lib\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    let error = source_catalog_at(&read(&root).unwrap(), &root.join("distribution")).unwrap_err();
    assert!(error.contains("checkout"));
    assert!(error.contains("does not satisfy [=2.0.0]"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checkout_dependency_rejects_duplicate_coordinates() {
    let root = temp("checkout-duplicate");
    for name in ["first", "second"] {
        let checkout = root.join("checkouts").join(name);
        fs::create_dir_all(checkout.join("src")).unwrap();
        fs::write(
            checkout.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/lib\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
        )
        .unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    let error = checkout_projects(&read(&root).unwrap()).unwrap_err();
    assert!(error.contains("multiple checkouts provide hara:demo/lib"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn installed_semantic_harps_resolve_and_load_through_an_explicit_store() {
    let root = temp("installed-semantic-harps");
    let base = root.join("base");
    let client = root.join("client");
    let consumer = root.join("consumer");
    let distribution = root.join("distribution");
    fs::create_dir_all(base.join("src/demo")).unwrap();
    fs::create_dir_all(client.join("src/demo")).unwrap();
    fs::create_dir_all(client.join("config")).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    fs::write(
        base.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/base\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        base.join("src/demo/base.hal"),
        "(ns demo.base) (defn answer [] 42)",
    )
    .unwrap();
    fs::write(
        client.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/client\" :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {\"hara:demo/base\" {:version \"=1.0.0\"}} :project/package {:name \"demo.client\" :profile \"config/packages.edn\"}}",
    )
    .unwrap();
    fs::write(
        client.join("config/packages.edn"),
        "{demo.client {:include [[demo.client :complete]]}}",
    )
    .unwrap();
    fs::write(
        client.join("project.lock.edn"),
        "{:lock/format \"0.0.1\" :packages {}}",
    )
    .unwrap();
    // The client carries the dependency source only as compilation context.
    // Its profile must exclude that context from the installed HARP payload.
    fs::write(
        client.join("src/demo/base.hal"),
        "(ns demo.base) (defn answer [] 42)",
    )
    .unwrap();
    fs::write(
        client.join("src/demo/client.hal"),
        "(ns demo.client (:require [demo.base :as base])) (defn answer [] (base/answer))",
    )
    .unwrap();
    fs::write(
        consumer.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id \"hara:demo/consumer\" :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/dependencies {\"hara:demo/client\" {:version \"=1.0.0\"}}}",
    )
    .unwrap();

    let base_archive = base.join("target/base.harp");
    build_package_archive(
        &base,
        &base_archive,
        "hara:demo/base",
        "1.0.0",
        &["project.edn", "src/demo/base.hal"],
        vec![("demo.base".into(), "src/demo/base.hal".into())],
    );
    let base_root = package::install_path_at(&base_archive, &distribution).unwrap();
    let client_archive = client.join("target/client.harp");
    build_package_archive(
        &client,
        &client_archive,
        "hara:demo/client",
        "1.0.0",
        &[
            "project.edn",
            "config/packages.edn",
            "project.lock.edn",
            "src/demo/client.hal",
        ],
        vec![("demo.client".into(), "src/demo/client.hal".into())],
    );
    let client_manifest = PackageManifest::read_archive(&client_archive).unwrap();
    assert_eq!(client_manifest.identity, "hara:demo/client");
    assert!(client_manifest.resources.contains_key("demo.client"));
    assert!(!client_manifest.resources.contains_key("demo.base"));
    let client_root = package::install_path_at(&client_archive, &distribution).unwrap();

    let catalog = source_catalog_at(&read(&consumer).unwrap(), &distribution).unwrap();
    assert_eq!(
        catalog.path("demo.base"),
        Some(base_root.join("src/demo/base.hal").canonicalize().unwrap())
    );
    assert_eq!(
        catalog.path("demo.client"),
        Some(
            client_root
                .join("src/demo/client.hal")
                .canonicalize()
                .unwrap()
        )
    );
    let mut runtime = Runtime::core();
    runtime.register_installed_package(&base_root).unwrap();
    runtime.register_installed_package(&client_root).unwrap();
    runtime.register_source_catalog(&catalog);
    assert_eq!(
        runtime
            .eval_native("(ns demo.consumer (:require [demo.client :as client])) (client/answer)")
            .unwrap(),
        "42"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_catalog_does_not_mutate_an_installed_package_root() {
    let root = temp("immutable-installed-source").join("roots/sha256/package");
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        root.join("src/demo/main.hal"),
        "(ns demo.main) (def answer 42)",
    )
    .unwrap();

    let project = read(&root).unwrap();
    assert_eq!(
        source_catalog(&project).unwrap().namespaces(),
        vec!["demo.main"]
    );
    assert!(!root
        .join("target/hara-cache/source-catalog-v1.index")
        .exists());
    fs::remove_dir_all(root.ancestors().nth(3).unwrap()).unwrap();
}

#[test]
fn source_catalog_defers_unrelated_legacy_source_discovery() {
    let root = temp("lazy-source-catalog");
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::create_dir_all(root.join("src/example")).unwrap();
    fs::create_dir_all(root.join("src/lang")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        root.join("src/demo/main.hal"),
        "(ns demo.main) (def answer 42)",
    )
    .unwrap();
    // A migration-incomplete source must not block a conventional require
    // from another source family during project startup.
    fs::write(root.join("src/lang/unmigrated.hal"), "not a namespace").unwrap();

    let catalog = source_catalog(&read(&root).unwrap()).unwrap();
    assert_eq!(
        catalog.path("demo.main"),
        Some(root.join("src/demo/main.hal").canonicalize().unwrap())
    );
    let mut runtime = Runtime::core();
    runtime.register_source_catalog(&catalog);
    assert_eq!(
        runtime
            .eval_native("(ns demo.check (:require [demo.main :as main])) main/answer")
            .unwrap(),
        "42"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_catalog_falls_back_to_legacy_path_discovery_on_demand() {
    let root = temp("legacy-source-catalog");
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        root.join("src/demo/legacy_file.hal"),
        "(ns demo.legacy-file)",
    )
    .unwrap();

    let catalog = source_catalog(&read(&root).unwrap()).unwrap();
    assert_eq!(
        catalog.path("demo.legacy-file"),
        Some(
            root.join("src/demo/legacy_file.hal")
                .canonicalize()
                .unwrap()
        )
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_catalog_family_content_fingerprint_ignores_unselected_source_changes() {
    let root = temp("source-family-content-fingerprint");
    fs::create_dir_all(root.join("src/lang")).unwrap();
    fs::create_dir_all(root.join("src/code")).unwrap();
    fs::create_dir_all(root.join("src/example")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        root.join("src/example/spec.hal"),
        "(ns example.spec) (def answer 42)",
    )
    .unwrap();
    fs::write(
        root.join("src/code/deploy.hal"),
        "(ns code.deploy) (def stage :one)",
    )
    .unwrap();

    let catalog = source_catalog(&read(&root).unwrap()).unwrap();
    let original = catalog.content_fingerprint_prefixes(&["example"]).unwrap();

    fs::write(
        root.join("src/code/deploy.hal"),
        "(ns code.deploy) (def stage :two)",
    )
    .unwrap();
    assert_eq!(
        catalog.content_fingerprint_prefixes(&["example"]).unwrap(),
        original
    );

    fs::write(
        root.join("src/example/spec.hal"),
        "(ns example.spec) (def answer 43)",
    )
    .unwrap();
    assert_ne!(
        catalog.content_fingerprint_prefixes(&["example"]).unwrap(),
        original
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_profile_excludes_legacy_source_subtrees() {
    let root = temp("native-source-excludes");
    fs::create_dir_all(root.join("src/example")).unwrap();
    fs::create_dir_all(root.join("src/std")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/runtime-profiles {:rust {:runtime/source-excludes [\"src/std\"]}}}",
    )
    .unwrap();
    fs::write(
        root.join("src/example/runtime.hal"),
        "(ns example.runtime) (def answer 42)",
    )
    .unwrap();
    fs::write(
        root.join("src/std/foundation.hal"),
        "(ns std.foundation) (def legacy-foundation true)",
    )
    .unwrap();

    let project = read(&root).unwrap();
    assert_eq!(project.source_excludes, vec![PathBuf::from("src/std")]);
    let catalog = source_catalog(&project).unwrap();
    assert_eq!(catalog.path("std.foundation"), None);
    assert_eq!(
        catalog.path("example.runtime"),
        Some(root.join("src/example/runtime.hal").canonicalize().unwrap())
    );
    assert_eq!(
        source_resources(&project)
            .unwrap()
            .into_iter()
            .map(|(namespace, _)| namespace)
            .collect::<Vec<_>>(),
        vec!["example.runtime".to_owned()]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deploy_source_excludes_remove_checkout_only_trees_from_native_sources() {
    let root = temp("deploy-source-excludes");
    fs::create_dir_all(root.join("src/example")).unwrap();
    fs::create_dir_all(root.join("src-build/play")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\" \"src-build\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/deploy {:source-excludes [\"src-build\"]}}",
    )
    .unwrap();
    fs::write(
        root.join("src/example/runtime.hal"),
        "(ns example.runtime) (def answer 42)",
    )
    .unwrap();
    fs::write(
        root.join("src-build/play/example.hal"),
        "(ns play.example) (def answer 7)",
    )
    .unwrap();

    let project = read(&root).unwrap();
    assert_eq!(project.source_excludes, vec![PathBuf::from("src-build")]);
    let catalog = source_catalog(&project).unwrap();
    assert!(catalog.path("play.example").is_none());
    assert!(catalog.path("example.runtime").is_some());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_catalog_bootstraps_the_fixed_foundation_family_without_a_full_index() {
    let root = temp("lazy-foundation-bootstrap");
    fs::create_dir_all(root.join("src/std/foundation")).unwrap();
    fs::create_dir_all(root.join("src/example")).unwrap();
    fs::create_dir_all(root.join("src/lang")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}",
    )
    .unwrap();
    fs::write(
        root.join("src/std/foundation.hal"),
        "(ns std.foundation) (defn set [value] value)",
    )
    .unwrap();
    fs::write(
        root.join("src/std/foundation/string.hal"),
        "(ns std.foundation.string (:config {:set-global-alias str})) (def answer (set 42))",
    )
    .unwrap();
    fs::write(root.join("src/lang/unmigrated.hal"), "not a namespace").unwrap();

    let catalog = source_catalog(&read(&root).unwrap()).unwrap();
    let mut runtime = Runtime::core();
    runtime.register_source_catalog(&catalog);
    runtime.bootstrap_source_foundation().unwrap();
    assert_eq!(runtime.eval_native("str/answer").unwrap(), "42");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_discovery_ignores_editor_artifacts() {
    let root = temp("editor-artifacts");
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::write(root.join("src/demo/core.hal"), "(ns demo.core)").unwrap();
    fs::write(root.join("src/demo/.#core.hal"), "unreadable editor lock").unwrap();
    fs::write(root.join("src/demo/#core.hal#"), "invalid editor backup").unwrap();
    assert_eq!(
        files_in(&root, &[PathBuf::from("src")]).unwrap(),
        vec![root.join("src/demo/core.hal")]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolves_language_profiles_with_main_and_options_inheritance() {
    let root = temp("profiles");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/main demo.core/app :project/default-profile :web :project/profiles {:web {:profile/language :hoplite :profile/options {:port 8080}} :admin {:profile/language :hoplite :profile/main demo.admin/app}}}").unwrap();
    let project = read(&root).unwrap();
    let web = project.resolve_profile(None).unwrap().unwrap();
    assert_eq!(
        (web.name.as_str(), web.language.as_str(), web.main.as_str()),
        ("web", "hoplite", "demo.core/app")
    );
    assert_eq!(web.options.to_string(), "{:port 8080}");
    let admin = project.resolve_profile(Some("admin")).unwrap().unwrap();
    assert_eq!(admin.main, "demo.admin/app");
    assert_eq!(admin.options, Form::Map(Vec::new()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_profile_language_and_unknown_default() {
    let root = temp("invalid-profiles");
    fs::create_dir_all(&root).unwrap();
    let prefix = "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} ";
    fs::write(
        root.join("project.edn"),
        format!("{prefix}:project/profiles {{:web {{}}}}}}"),
    )
    .unwrap();
    assert!(read(&root).unwrap_err().contains(":profile/language"));
    fs::write(root.join("project.edn"), format!("{prefix}:project/default-profile :missing :project/profiles {{:web {{:profile/language :hoplite}}}}}}")).unwrap();
    assert!(read(&root).unwrap_err().contains("is not declared"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn accepts_a_profile_only_multi_package_project_declaration() {
    let root = temp("package-profile");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/packages.edn"),
        "{demo.public {:include [[demo.public :complete]]}}",
    )
    .unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/package {:profile \"config/packages.edn\"}}",
    )
    .unwrap();
    let project = read(&root).unwrap();
    assert_eq!(project.package_name, None);
    assert_eq!(
        project.package_profile,
        Some(PathBuf::from("config/packages.edn"))
    );
    assert!(!project.package_workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expands_project_aliases_and_rejects_cycles() {
    let root = temp("aliases");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/aliases {:check-code [\"manage\" \"analyse\"] :all [\"check-code\" \":all\"]}}").unwrap();
    let project = read(&root).unwrap();
    assert_eq!(
        expand_aliases(&project, &["all".into(), "xt.lang".into()]).unwrap(),
        vec!["manage", "analyse", ":all", "xt.lang"]
    );
    fs::write(root.join("project.edn"), "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/aliases {:a [\"b\"] :b [\"a\"]}}").unwrap();
    assert!(expand_aliases(&read(&root).unwrap(), &["a".into()])
        .unwrap_err()
        .contains("cycle"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_a_distribution_declaration_and_rejects_an_unsafe_launcher() {
    let root = temp("distribution");
    fs::create_dir_all(&root).unwrap();
    let base = "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{}}";
    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned()
            + " :project/distribution {:launcher \"hara\" :entry demo.cli/main}}",
    )
    .unwrap();
    assert_eq!(
        read(&root).unwrap().distribution,
        Some(Distribution {
            launcher: "hara".into(),
            entry: "demo.cli/main".into(),
        })
    );

    fs::write(
        root.join("project.edn"),
        base.strip_suffix('}').unwrap().to_owned()
            + " :project/distribution {:launcher \"../hara\" :entry demo.cli/main}}",
    )
    .unwrap();
    assert!(read(&root)
        .unwrap_err()
        .contains(":launcher must contain lowercase letters"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selects_rust_runtime_profile_and_can_resolve_jvm_overlay() {
    let root = temp("runtime-profiles");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [\"test\"] :project/extension-paths [\"extensions\"] :project/capabilities #{} :project/dependencies {\"hara:hara/base\" {:version \"^1.0.0\"}} :project/profiles {:production {:profile/language :hara}} :project/runtime-profiles {:rust {:runtime/source-paths [\"src-rust\"] :runtime/test-paths [\"test-rust\"] :runtime/extension-paths [\"extensions-rust\"] :runtime/dependencies {:hara {\"hara:hara/crypto\" {:version \"^1.0.0\"}}}} :jvm {:runtime/source-paths [\"src-jvm\"] :runtime/native-source-paths [\"src-java\"] :runtime/target-path \"target/jvm/classes\" :runtime/dependencies {:maven {org.postgresql/postgresql {:version \"42.7.7\"}}}}}}",
    )
    .unwrap();
    let project = read(&root).unwrap();
    assert_eq!(project.active_runtime, "rust");
    assert_eq!(
        project.source_paths,
        vec![PathBuf::from("src"), PathBuf::from("src-rust")]
    );
    assert_eq!(
        project.test_paths,
        vec![PathBuf::from("test"), PathBuf::from("test-rust")]
    );
    assert_eq!(
        project.extension_paths,
        vec![
            PathBuf::from("extensions"),
            PathBuf::from("extensions-rust")
        ]
    );
    assert_eq!(project.dependencies["hara:hara/base"], "^1.0.0");
    assert_eq!(project.dependencies["hara:hara/crypto"], "^1.0.0");
    assert!(project.profiles.contains_key("production"));

    let jvm = project.resolve_runtime_profile("jvm").unwrap();
    assert_eq!(
        jvm.source_paths,
        vec![PathBuf::from("src"), PathBuf::from("src-jvm")]
    );
    assert_eq!(jvm.native_source_paths, vec![PathBuf::from("src-java")]);
    assert_eq!(jvm.target_path, Some(PathBuf::from("target/jvm/classes")));
    assert_eq!(
        jvm.maven_dependencies["org.postgresql/postgresql"],
        "42.7.7"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn validates_runtime_scoped_npm_wasm_imports() {
    let root = temp("npm-wasm-imports");
    fs::create_dir_all(&root).unwrap();
    let prefix = "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} ";
    fs::write(
        root.join("project.edn"),
        format!(
            "{prefix}:project/runtime-profiles {{:rust {{:runtime/dependencies {{:npm {{\"raw-math\" {{:version \"1.2.3\" :integrity \"sha512-AAAAAAAAAAAAAAAAAAAAAA==\"}}}}}} :runtime/imports {{Math {{:package \"raw-math\" :module \"dist/math.wasm\" :abi :core.v1}}}}}}}}}}"
        ),
    )
    .unwrap();
    let project = read(&root).unwrap();
    let rust = project.resolve_runtime_profile("rust").unwrap();
    assert_eq!(
        rust.npm_dependencies["raw-math"].version.to_string(),
        "1.2.3"
    );
    assert_eq!(
        rust.native_imports["Math"].module,
        PathBuf::from("dist/math.wasm")
    );

    for (declaration, message) in [
        (
            "{:version \"^1.2.3\" :integrity \"sha512-AAAAAAAAAAAAAAAAAAAAAA==\"}",
            "exact SemVer",
        ),
        ("{:version \"1.2.3\"}", "requires :integrity"),
        (
            "{:version \"1.2.3\" :integrity \"sha512-AAAAAAAAAAAAAAAAAAAAAA==\" :scripts {}}",
            "unsupported npm dependency field :scripts",
        ),
    ] {
        let mut source = prefix.to_owned();
        source.push_str(
            ":project/runtime-profiles {:rust {:runtime/dependencies {:npm {\"raw-math\" ",
        );
        source.push_str(declaration);
        source.push_str("}}}}}");
        fs::write(root.join("project.edn"), source).unwrap();
        assert!(read(&root).unwrap_err().contains(message));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_legacy_jvm_project_keys_and_conflicting_runtime_dependencies() {
    let root = temp("runtime-invalid");
    fs::create_dir_all(&root).unwrap();
    let prefix = "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} ";
    fs::write(
        root.join("project.edn"),
        format!("{prefix}:jvm/source-paths [\"src-java\"]}}"),
    )
    .unwrap();
    let legacy = read(&root).unwrap_err();
    assert!(legacy.contains(":project/runtime-profiles :jvm :runtime/native-source-paths"));

    fs::write(
        root.join("project.edn"),
        format!("{prefix}:project/dependencies {{\"hara:hara/crypto\" {{:version \"^1.0.0\"}}}} :project/runtime-profiles {{:rust {{:runtime/dependencies {{:hara {{\"hara:hara/crypto\" {{:version \"^2.0.0\"}}}}}}}}}}}}"),
    )
    .unwrap();
    let conflict = read(&root).unwrap_err();
    assert!(conflict.contains("conflicting Hara dependency requirements"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn runtime_namespace_alternatives_are_isolated_but_effective_duplicates_fail() {
    let root = temp("runtime-namespaces");
    fs::create_dir_all(root.join("src-rust/demo")).unwrap();
    fs::create_dir_all(root.join("src-jvm/demo")).unwrap();
    fs::create_dir_all(root.join("src/demo")).unwrap();
    fs::write(
        root.join("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/runtime-profiles {:rust {:runtime/source-paths [\"src-rust\"]} :jvm {:runtime/source-paths [\"src-jvm\"]}}}",
    )
    .unwrap();
    fs::write(
        root.join("src-rust/demo/adapter.hal"),
        "(ns demo.adapter) (def runtime :rust)",
    )
    .unwrap();
    fs::write(
        root.join("src-jvm/demo/adapter.hal"),
        "(ns demo.adapter) (def runtime :jvm)",
    )
    .unwrap();
    let project = read(&root).unwrap();
    let mut runtime = Runtime::new();
    register_sources(&project, &mut runtime).unwrap();

    fs::write(
        root.join("src/demo/adapter.hal"),
        "(ns demo.adapter) (def runtime :shared)",
    )
    .unwrap();
    let project = read(&root).unwrap();
    let mut runtime = Runtime::new();
    assert!(register_sources(&project, &mut runtime)
        .unwrap_err()
        .contains("duplicate namespace demo.adapter"));
    fs::remove_dir_all(root).unwrap();
}
