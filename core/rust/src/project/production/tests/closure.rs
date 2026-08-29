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
fn keeps_canonical_cross_namespace_edges_and_removes_unused_vars() {
    let modules = vec![
        SourceModule::synthetic(
            "app.lib",
            "(ns app.lib)\n(defn helper [] 42)\n(defn unused-lib [] 0)\n",
        ),
        SourceModule::synthetic(
            "app.main",
            "(ns app.main (:require [app.lib :as lib]))\n(defn start [] (lib/helper))\n(defn unused [] 0)\n",
        ),
    ];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.succeeded(), "{:?}", analysis.diagnostics);
    assert!(analysis.retained_vars.contains("app.main/start"));
    assert!(analysis.retained_vars.contains("app.lib/helper"));
    assert!(analysis.removed_vars.contains("app.main/unused"));
    assert!(analysis.removed_vars.contains("app.lib/unused-lib"));
    assert!(analysis.runtime_closure.contains("app.lib/helper"));
}

#[test]
fn preserves_mutual_recursion_as_one_runtime_closure() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(declare odd-value?)\n(defn even-value? [n] (or (= n 0) (odd-value? (- n 1))))\n(defn odd-value? [n] (and (not= n 0) (even-value? (- n 1))))\n(defn start [] (even-value? 4))\n",
    )];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.retained_vars.contains("app.main/even-value?"));
    assert!(analysis.retained_vars.contains("app.main/odd-value?"));
}

#[test]
fn separates_macro_and_runtime_closures() {
    let modules = vec![
        SourceModule::synthetic(
            "app.macros",
            "(ns app.macros)\n(defmacro identity-form [value] value)\n",
        ),
        SourceModule::synthetic(
            "app.main",
            "(ns app.main (:require [app.macros :refer [identity-form]]))\n(defn start [] (identity-form 42))\n",
        ),
    ];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis
        .compile_time_closure
        .contains("app.macros/identity-form"));
    assert!(!analysis
        .runtime_closure
        .contains("app.macros/identity-form"));
}

#[test]
fn retains_unknown_top_level_initializers_with_a_reason() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(defn start [] 1)\n(str \"top-level\")\n",
    )];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.reasons.iter().any(|reason| {
        reason.code == "unknown-top-level-effect" && reason.unit_id.starts_with("app.main:")
    }));
}

#[test]
fn canonicalizes_aliased_and_explicit_macro_refers_before_expansion() {
    let modules = vec![
        SourceModule::synthetic(
            "app.macros",
            "(ns app.macros)\n(defmacro aliased-form [value] value)\n(defmacro referred-form [value] value)\n",
        ),
        SourceModule::synthetic(
            "app.main",
            "(ns app.main (:require [app.macros :as macros :refer-macros [referred-form]]))\n(defn start [] [(macros/aliased-form 41) (referred-form 42)])\n",
        ),
    ];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert!(analysis.succeeded(), "{:?}", analysis.diagnostics);
    assert!(analysis
        .compile_time_closure
        .contains("app.macros/aliased-form"));
    assert!(analysis
        .compile_time_closure
        .contains("app.macros/referred-form"));
}
