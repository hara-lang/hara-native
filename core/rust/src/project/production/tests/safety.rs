use super::super::source::SourceModule;
use super::super::{analyze_modules, BuildPlan};

fn plan(entrypoint: &str) -> BuildPlan {
    BuildPlan {
        project_id: "demo-app".into(),
        project_version: "0.1.0".into(),
        profile: "production".into(),
        language: "hara".into(),
        main: "app.main".into(),
        entrypoints: vec![entrypoint.into()],
        default_entrypoint: entrypoint.into(),
        keep_vars: Vec::new(),
        keep_namespaces: Vec::new(),
        output_bundle: "target/demo-app-production.hbx".into(),
        output_report: "target/demo-app-production.shake.edn".into(),
    }
}

#[test]
fn reports_unbounded_dynamic_access_at_its_source_unit() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(defn start [target] (resolve target))\n",
    )];
    let analysis = analyze_modules(&plan("app.main/start"), modules.clone()).unwrap();
    let diagnostic = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "production/unbounded-dynamic-var")
        .expect("unbounded resolve must fail production analysis");
    assert_eq!(diagnostic.location.path, "fixture:app.main");
    assert!(diagnostic.location.line >= 2);

    let mut bounded = plan("app.main/start");
    bounded.keep_vars = vec!["app.main/start".into()];
    let analysis = analyze_modules(&bounded, modules).unwrap();
    assert!(!analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "production/unbounded-dynamic-var"));
}

#[test]
fn literal_dynamic_targets_become_ordinary_edges() {
    let modules = vec![
        SourceModule::synthetic("app.handlers", "(ns app.handlers)\n(defn run [] 42)\n"),
        SourceModule::synthetic(
            "app.main",
            "(ns app.main (:require [app.handlers]))\n(defn start [] (resolve 'app.handlers/run))\n",
        ),
    ];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.runtime_closure.contains("app.handlers/run"));
    assert!(analysis.retained_vars.contains("app.handlers/run"));
}

#[test]
fn reports_dynamic_access_generated_by_a_referred_macro() {
    let modules = vec![
        SourceModule::synthetic(
            "app.macros",
            "(ns app.macros)\n(defmacro dynamic-form [target] (list 'resolve target))\n",
        ),
        SourceModule::synthetic(
            "app.main",
            "(ns app.main (:require [app.macros :refer-macros [dynamic-form]]))\n(defn start [target] (dynamic-form target))\n",
        ),
    ];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "production/unbounded-dynamic-var" && diagnostic.module == "app.main"
    }));
}

#[test]
fn ignores_dynamic_operation_names_shadowed_by_lexical_bindings() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(defn start [resolve require eval load-string]\n  (let [resolve (fn [value] value)]\n    [(resolve 1) (require 2) (eval 3) (load-string 4)\n     ((fn [resolve] (resolve 5)) resolve)]))\n",
    )];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.succeeded(), "{:?}", analysis.diagnostics);
    assert!(!analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.contains("dynamic")
            || diagnostic.code.contains("eval")
            || diagnostic.code.contains("generated")
    }));
}
