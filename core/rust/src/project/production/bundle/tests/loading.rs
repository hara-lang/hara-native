use super::super::super::source::SourceModule;
use super::super::{compile, load};
use super::support::{analyzed, fixture_modules, plan};
use crate::core::Value;

#[test]
fn loads_with_runtime_core_and_invokes_every_entrypoint() {
    let build = analyzed(fixture_modules(), plan("app.main/start"));
    let compiled = compile::compile(&build).unwrap();
    let runtime = load::validate_bundle(&compiled.bytes, &build.plan.entrypoints).unwrap();
    assert!(matches!(
        load::invoke_zero_arity(&runtime, "app.main/start").unwrap(),
        Value::Number(42)
    ));
}

#[test]
fn rejects_missing_entrypoints_after_loading() {
    let build = analyzed(fixture_modules(), plan("app.main/start"));
    let compiled = compile::compile(&build).unwrap();
    let entrypoints = vec!["app.main/missing".into()];
    let error = match load::validate_bundle(&compiled.bytes, &entrypoints) {
        Ok(_) => panic!("missing production entrypoint unexpectedly loaded"),
        Err(error) => error,
    };
    assert!(error.contains("entrypoint is missing"));
}

#[test]
fn rejects_non_function_entrypoints_after_loading() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(def start 42)\n",
    )];
    let build = analyzed(modules, plan("app.main/start"));
    assert!(build.analysis.succeeded(), "{:?}", build.analysis.diagnostics);
    let compiled = compile::compile(&build).unwrap();
    let error = match load::validate_bundle(&compiled.bytes, &build.plan.entrypoints) {
        Ok(_) => panic!("non-callable production entrypoint unexpectedly loaded"),
        Err(error) => error,
    };
    assert!(error.contains("not invokable"));
}

#[test]
fn invokes_mutually_recursive_entrypoint_after_pruning_declare() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(declare odd?)\n(defn even? [n] (if (= n 0) true (odd? (- n 1))))\n(defn odd? [n] (if (= n 0) false (even? (- n 1))))\n(defn start [] (odd? 3))\n",
    )];
    let build = analyzed(modules, plan("app.main/start"));
    assert!(build.analysis.succeeded(), "{:?}", build.analysis.diagnostics);
    let compiled = compile::compile(&build).unwrap();
    let runtime = load::validate_bundle(&compiled.bytes, &build.plan.entrypoints).unwrap();
    assert!(matches!(
        load::invoke_zero_arity(&runtime, "app.main/start").unwrap(),
        Value::Bool(true)
    ));
}
