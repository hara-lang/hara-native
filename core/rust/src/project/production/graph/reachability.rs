use super::super::plan::BuildPlan;
use super::index::retain;
use super::model::RetentionReason;
use super::{Effect, UnitAnalysis, UnitKind};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) struct Reachability {
    pub runtime_roots: BTreeSet<String>,
    pub runtime_closure: BTreeSet<String>,
    pub runtime_unit_ids: BTreeSet<String>,
    pub compile_time_roots: BTreeSet<String>,
    pub compile_time_closure: BTreeSet<String>,
    pub compile_time_unit_ids: BTreeSet<String>,
    pub retained_unit_ids: BTreeSet<String>,
    pub reasons: Vec<RetentionReason>,
}

pub(super) fn compute(
    plan: &BuildPlan,
    units: &[UnitAnalysis],
    providers: &BTreeMap<String, String>,
    namespace_units: &BTreeMap<String, Vec<usize>>,
) -> Reachability {
    let unit_positions = units
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let runtime_roots = plan
        .entrypoints
        .iter()
        .chain(plan.keep_vars.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut reasons = Vec::new();
    let mut runtime_units = BTreeSet::new();
    let mut runtime_closure = runtime_roots.clone();
    let mut queue = VecDeque::new();

    for root in &runtime_roots {
        if let Some(unit_id) = providers.get(root) {
            retain(
                unit_id,
                Some(root.clone()),
                if plan.entrypoints.contains(root) {
                    "entrypoint"
                } else {
                    "keep-var"
                },
                "declared production root",
                &mut runtime_units,
                &mut reasons,
                &mut queue,
            );
        }
    }
    for namespace in &plan.keep_namespaces {
        if let Some(indices) = namespace_units.get(namespace) {
            for index in indices {
                retain(
                    &units[*index].id,
                    None,
                    "keep-namespace",
                    &format!("namespace {namespace} is explicitly retained"),
                    &mut runtime_units,
                    &mut reasons,
                    &mut queue,
                );
            }
        }
    }
    for unit in units {
        if unit.kind == UnitKind::Registration {
            retain(
                &unit.id,
                unit.provides.iter().next().cloned(),
                "registration",
                "registration forms are conservatively retained",
                &mut runtime_units,
                &mut reasons,
                &mut queue,
            );
        } else if unit.kind == UnitKind::Initializer && unit.effect != Effect::Pure {
            retain(
                &unit.id,
                None,
                "unknown-top-level-effect",
                "top-level initializer is not proven pure",
                &mut runtime_units,
                &mut reasons,
                &mut queue,
            );
        }
    }
    while let Some(unit_id) = queue.pop_front() {
        let Some(index) = unit_positions.get(&unit_id).copied() else {
            continue;
        };
        let unit = &units[index];
        for edge in &unit.runtime_edges {
            runtime_closure.insert(edge.clone());
            if let Some(target) = providers.get(edge) {
                retain(
                    target,
                    Some(edge.clone()),
                    "runtime-dependency",
                    &format!("referenced by {}", unit.id),
                    &mut runtime_units,
                    &mut reasons,
                    &mut queue,
                );
            }
        }
        for namespace in &unit.namespace_edges {
            if let Some(indices) = namespace_units.get(namespace) {
                for target in indices {
                    retain(
                        &units[*target].id,
                        None,
                        "dynamic-namespace",
                        &format!("namespace {namespace} is loaded by {}", unit.id),
                        &mut runtime_units,
                        &mut reasons,
                        &mut queue,
                    );
                }
            }
        }
    }

    let mut compile_time_roots = BTreeSet::new();
    for unit_id in &runtime_units {
        if let Some(index) = unit_positions.get(unit_id) {
            compile_time_roots.extend(units[*index].compile_time_edges.iter().cloned());
        }
    }
    let mut compile_time_closure = compile_time_roots.clone();
    let mut compile_units = BTreeSet::new();
    let mut compile_queue = VecDeque::new();
    for root in &compile_time_roots {
        if let Some(unit_id) = providers.get(root) {
            retain(
                unit_id,
                Some(root.clone()),
                "compile-time-macro",
                "macro is required while producing retained definitions",
                &mut compile_units,
                &mut reasons,
                &mut compile_queue,
            );
        }
    }
    while let Some(unit_id) = compile_queue.pop_front() {
        let Some(index) = unit_positions.get(&unit_id).copied() else {
            continue;
        };
        let unit = &units[index];
        for edge in unit
            .compile_time_edges
            .iter()
            .chain(unit.runtime_edges.iter())
        {
            compile_time_closure.insert(edge.clone());
            if let Some(target) = providers.get(edge) {
                retain(
                    target,
                    Some(edge.clone()),
                    "compile-time-dependency",
                    &format!("required by compile-time unit {}", unit.id),
                    &mut compile_units,
                    &mut reasons,
                    &mut compile_queue,
                );
            }
        }
    }

    let retained_unit_ids = runtime_units.union(&compile_units).cloned().collect();
    Reachability {
        runtime_roots,
        runtime_closure,
        runtime_unit_ids: runtime_units,
        compile_time_roots,
        compile_time_closure,
        compile_time_unit_ids: compile_units,
        retained_unit_ids,
        reasons,
    }
}
