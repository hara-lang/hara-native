use super::super::plan::BuildPlan;
use super::index::{namespace_index, project_diagnostic, provider_index};
use super::model::{Analysis, ModuleAnalysis};
use super::reachability;
use super::{NativeRootInventory, UnitAnalysis};
use std::collections::BTreeSet;

pub fn finish_analysis(
    plan: &BuildPlan,
    modules: Vec<ModuleAnalysis>,
    units: Vec<UnitAnalysis>,
    input_bytes: usize,
    input_digest: String,
) -> Analysis {
    let providers = provider_index(&units);
    let namespace_units = namespace_index(&units);
    let reachability = reachability::compute(plan, &units, &providers, &namespace_units);
    let all_unit_ids = units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<BTreeSet<_>>();
    let removed_unit_ids = all_unit_ids
        .difference(&reachability.retained_unit_ids)
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut retained_vars = BTreeSet::new();
    let mut removed_vars = BTreeSet::new();
    let mut retained_namespaces = BTreeSet::new();
    let mut native_roots = NativeRootInventory::default();
    let mut native_primitives = BTreeSet::new();
    let mut native_types = BTreeSet::new();
    let mut native_protocols = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for unit in &units {
        if reachability.retained_unit_ids.contains(&unit.id) {
            retained_vars.extend(unit.provides.iter().cloned());
            retained_namespaces.insert(unit.module.clone());
            native_roots.extend(&unit.native_roots);
            native_primitives.extend(unit.native_primitives.iter().cloned());
            native_types.extend(unit.native_types.iter().cloned());
            native_protocols.extend(unit.native_protocols.iter().cloned());
            diagnostics.extend(unit.diagnostics.iter().cloned());
        } else {
            removed_vars.extend(unit.provides.iter().cloned());
        }
    }
    removed_vars = removed_vars
        .difference(&retained_vars)
        .cloned()
        .collect::<BTreeSet<_>>();
    let all_namespaces = modules
        .iter()
        .map(|module| module.name.clone())
        .collect::<BTreeSet<_>>();
    let removed_namespaces = all_namespaces
        .difference(&retained_namespaces)
        .cloned()
        .collect::<BTreeSet<_>>();

    for entrypoint in &plan.entrypoints {
        if !providers.contains_key(entrypoint) {
            diagnostics.push(project_diagnostic(
                "production/missing-entrypoint",
                "entrypoint",
                entrypoint,
                format!("production entrypoint has no analyzed provider: {entrypoint}"),
            ));
        }
    }
    for keep_var in &plan.keep_vars {
        if !providers.contains_key(keep_var) {
            diagnostics.push(project_diagnostic(
                "production/missing-keep-var",
                "keep-var",
                keep_var,
                format!("kept Var has no analyzed provider: {keep_var}"),
            ));
        }
    }
    for namespace in &plan.keep_namespaces {
        if !namespace_units.contains_key(namespace) {
            diagnostics.push(project_diagnostic(
                "production/missing-keep-namespace",
                "keep-namespace",
                namespace,
                format!("kept namespace was not analyzed: {namespace}"),
            ));
        }
    }

    diagnostics.sort_by(|left, right| {
        (
            left.location.path.as_str(),
            left.location.line,
            left.location.column,
            left.code.as_str(),
            left.message.as_str(),
        )
            .cmp(&(
                right.location.path.as_str(),
                right.location.line,
                right.location.column,
                right.code.as_str(),
                right.message.as_str(),
            ))
    });
    diagnostics.dedup();
    let mut reasons = reachability.reasons;
    reasons.sort_by(|left, right| {
        (
            left.unit_id.as_str(),
            left.code.as_str(),
            left.subject.as_deref().unwrap_or(""),
        )
            .cmp(&(
                right.unit_id.as_str(),
                right.code.as_str(),
                right.subject.as_deref().unwrap_or(""),
            ))
    });
    reasons.dedup();

    Analysis {
        modules,
        units,
        runtime_roots: reachability.runtime_roots,
        runtime_closure: reachability.runtime_closure,
        runtime_unit_ids: reachability.runtime_unit_ids,
        compile_time_roots: reachability.compile_time_roots,
        compile_time_closure: reachability.compile_time_closure,
        compile_time_unit_ids: reachability.compile_time_unit_ids,
        retained_unit_ids: reachability.retained_unit_ids,
        removed_unit_ids,
        retained_vars,
        removed_vars,
        retained_namespaces,
        removed_namespaces,
        reasons,
        diagnostics,
        native_roots,
        native_primitives,
        native_types,
        native_protocols,
        input_bytes,
        input_digest,
    }
}
