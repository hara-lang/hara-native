use super::super::compile;
use super::support::{analyzed, fixture_modules, plan};
use crate::vm::{compile_bytecode_bundle, ModuleSource};

#[test]
fn repeated_clean_compiles_are_byte_identical() {
    let first = analyzed(fixture_modules(), plan("app.main/start"));
    let second = analyzed(fixture_modules(), plan("app.main/start"));
    let first = compile::compile(&first).unwrap();
    let second = compile::compile(&second).unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(
        first
            .modules
            .iter()
            .map(|module| (
                &module.resource,
                &module.namespace_form,
                &module.dependencies
            ))
            .collect::<Vec<_>>(),
        second
            .modules
            .iter()
            .map(|module| (
                &module.resource,
                &module.namespace_form,
                &module.dependencies
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn records_module_and_var_level_size_reduction() {
    let modules = fixture_modules();
    let build = analyzed(modules.clone(), plan("app.main/start"));
    let pruned = compile::compile(&build).unwrap();
    let complete_sources = modules
        .iter()
        .map(|module| ModuleSource {
            resource: module.name.as_str(),
            source: module.source.as_str(),
        })
        .collect::<Vec<_>>();
    let complete = compile_bytecode_bundle(&complete_sources).unwrap();
    assert!(
        pruned.bytes.len() < complete.len(),
        "{} !< {}",
        pruned.bytes.len(),
        complete.len()
    );
}
