use crate::{core, Runtime};
use std::collections::BTreeSet;
use std::sync::OnceLock;

fn foundation_runtime() -> Runtime {
    let registry = crate::spec_registry::root()
        .expect("native behavioral conformance requires hara-specs-registry");
    let source_root = registry
        .parent()
        .map(|root| root.join("hara"))
        .filter(|root| root.join("project.edn").is_file())
        .expect("native behavioral conformance requires sibling technology/hara source");
    let project = crate::project::read(&source_root)
        .expect("native behavioral conformance Hara project must be valid");
    let catalog = crate::project::source_catalog(&project)
        .expect("native behavioral conformance source catalog must be valid");
    let mut runtime = Runtime::new();
    runtime.install_native_file_provider(source_root.to_string_lossy().as_ref());
    runtime.register_source_catalog(&catalog);
    runtime
        .bootstrap_source_foundation()
        .expect("native behavioral conformance must bootstrap source Foundation");
    runtime
}

fn corpus() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE
        .get_or_init(|| {
            std::fs::read_to_string(crate::spec_registry::require(
                "01-lang/001-language/draft/conformance/fixtures/native_behavioral.hal",
            ))
            .expect("native behavioral corpus must be readable")
        })
        .as_str()
}

fn corpus_methods() -> BTreeSet<String> {
    let corpus = corpus();
    let mut runtime = foundation_runtime();
    let value = runtime
        .eval_native_value(&format!("{corpus}\n(native-method-keys)"))
        .expect("native corpus keys must evaluate");
    let core::Value::Vector(values) = value else {
        panic!("native-method-keys must return a vector");
    };
    let methods = values
        .iter()
        .map(core::Value::display)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        values.len(),
        methods.len(),
        "duplicate native corpus method"
    );
    assert!(!methods.is_empty(), "native corpus must not be empty");
    methods
}

fn live_methods() -> BTreeSet<String> {
    core::native_declarations()
        .iter()
        .flat_map(|declaration| {
            declaration
                .methods
                .iter()
                .map(move |method| format!("{}/{}", declaration.name, method))
        })
        .collect()
}

fn closure_pass(methods: &BTreeSet<String>) -> bool {
    let corpus = corpus();
    let literal = methods
        .iter()
        .map(|method| format!("'{method}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut runtime = foundation_runtime();
    runtime
        .eval_text(&format!(
            "{corpus}\n(get (native-closure-report [{literal}]) :pass)"
        ))
        .expect("native closure report must evaluate")
        == "true"
}

fn calibration_value(name: &str, field: &str) -> core::Value {
    let corpus = corpus();
    let mut runtime = foundation_runtime();
    runtime
        .eval_native_value(&format!(
            "{corpus}\n(get (get native-calibration-snippets :{name}) :{field})"
        ))
        .expect("native calibration snippet must evaluate")
}

fn calibration_source(name: &str) -> String {
    match calibration_value(name, "source") {
        core::Value::String(source) => source,
        value => panic!(
            "native calibration snippet must be a string: {}",
            value.display()
        ),
    }
}

fn calibration_expected(name: &str) -> String {
    calibration_value(name, "expected").display()
}

#[test]
fn specs_owned_native_corpus_closes_over_live_inventory_and_rejects_drift() {
    let corpus = corpus();
    let mut runtime = foundation_runtime();
    assert_eq!(
        "true",
        runtime
            .eval_text(&format!("{corpus}\n(native-corpus-valid?)"))
            .expect("native corpus validation must evaluate")
    );
    eprintln!(
        "native behavioral classifications {}",
        runtime
            .eval_text(&format!("{corpus}\n(native-classification-summary)"))
            .expect("native classification summary must evaluate")
    );

    let classified = corpus_methods();
    let live = live_methods();
    let missing = live.difference(&classified).cloned().collect::<Vec<_>>();
    let stale = classified.difference(&live).cloned().collect::<Vec<_>>();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "native corpus drift: missing={missing:?}; stale={stale:?}"
    );
    assert!(closure_pass(&live));

    let first = classified
        .iter()
        .next()
        .expect("classified native method")
        .clone();
    let mut removed = classified.clone();
    removed.remove(&first);
    assert!(!closure_pass(&removed));

    let mut added = classified.clone();
    added.insert("Unclassified/addition".to_owned());
    assert!(!closure_pass(&added));

    let mut renamed = classified;
    renamed.remove(&first);
    renamed.insert(format!("{first}-renamed"));
    assert!(!closure_pass(&renamed));
}

#[test]
fn evaluator_runs_every_specs_owned_classification_boundary_and_profile() {
    let corpus = corpus();
    let methods = corpus_methods();
    let mut runtime = foundation_runtime();
    let results = runtime
        .eval_text(&format!("{corpus}\n(native-method-results)"))
        .expect("shared native behavioral corpus must evaluate");
    assert!(!results.contains(":pass false"), "{results}");
    assert_eq!(methods.len(), results.matches(":pass true").count());

    let mut boundary_runtime = foundation_runtime();
    let boundaries = boundary_runtime
        .eval_text(&format!("{corpus}\n(native-boundary-results)"))
        .expect("portable native boundary results must evaluate");
    assert_eq!(
        "true",
        runtime
            .eval_text(&format!(
                "{corpus}\n(every? (fn [case] (= true (get case :pass))) (native-boundary-results))"
            ))
            .expect("portable native boundary pass report must evaluate"),
        "{boundaries}"
    );
    assert_eq!(
        "true",
        runtime
            .eval_text(&format!(
                "{corpus}\n(let [report (native-profile-report)] (and (= 0 (get report :failed)) (= (+ (get report :passed) (get report :failed) (get report :skipped)) (+ (get report :portable) (get report :capability-specific) (get report :inventory-only)))))"
            ))
            .expect("native profile accounting must evaluate")
    );
}

#[test]
fn evaluator_and_bytecode_use_the_specs_owned_calibration_probe() {
    let probe = calibration_source("evaluator-compiler");
    let expected = calibration_expected("evaluator-compiler");
    let mut runtime = foundation_runtime();
    let interpreted = runtime
        .eval_text(&probe)
        .expect("evaluator native probe must run");
    let compiled = runtime
        .eval_bytecode_native(&probe)
        .expect("bytecode native probe must run");
    assert_eq!(expected, interpreted);
    assert_eq!(expected, compiled);
}

#[test]
fn rust_runs_the_specs_owned_identity_fast_path_calibration() {
    let source = calibration_source("base-identity-fast-paths");
    let expected = calibration_expected("base-identity-fast-paths");
    let mut runtime = foundation_runtime();
    assert_eq!(
        expected,
        runtime
            .eval_text(&source)
            .expect("identity fast-path calibration must run")
    );
}
