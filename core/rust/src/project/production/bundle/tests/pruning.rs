use super::super::super::source::SourceModule;
use super::super::compile;
use super::support::{analyzed, fixture_modules, plan};
use crate::core::Value;
use crate::vm::decode_program;

#[test]
fn omits_unreachable_modules_and_vars_from_hbx() {
    let build = analyzed(fixture_modules(), plan("app.main/start"));
    let compiled = compile::compile(&build).unwrap();
    let resources = compiled
        .modules
        .iter()
        .map(|module| module.resource.as_str())
        .collect::<Vec<_>>();
    assert_eq!(resources, ["app.lib", "app.main"]);
    let lib = compiled
        .modules
        .iter()
        .find(|module| module.resource == "app.lib")
        .unwrap();
    let program = decode_program(&lib.artifact).unwrap();
    assert!(!program
        .constants
        .iter()
        .any(|value| matches!(value, Value::String(name) if name == "app.lib/unused-lib")));
    assert!(!compiled
        .bytes
        .windows("app.unused".len())
        .any(|window| window == b"app.unused"));
}

#[test]
fn final_definition_replaces_earlier_declare_provider() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(declare odd?)\n(defn even? [n] (or (= n 0) (odd? (- n 1))))\n(defn odd? [n] (and (not= n 0) (even? (- n 1))))\n(defn start [] (odd? 3))\n",
    )];
    let build = analyzed(modules, plan("app.main/start"));
    let retained = build
        .analysis
        .units
        .iter()
        .filter(|unit| {
            unit.provides.contains("app.main/odd?")
                && build.analysis.retained_unit_ids.contains(&unit.id)
        })
        .map(|unit| unit.index)
        .collect::<Vec<_>>();
    assert_eq!(retained, [2000]);
}
