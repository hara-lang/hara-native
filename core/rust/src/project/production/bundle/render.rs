use super::model::{ProductionBuild, RenderedModule};
use crate::kernel::{parse, Form, GeneratedNamespaceConfig};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn retained_modules(build: &ProductionBuild) -> Result<Vec<RenderedModule>, String> {
    if !build.analysis.succeeded() {
        return Err("cannot emit a production bundle from failed analysis".into());
    }

    let all_modules = build
        .analysis
        .modules
        .iter()
        .map(|module| module.name.clone())
        .collect::<BTreeSet<_>>();
    let mut units_by_module = BTreeMap::<String, Vec<_>>::new();
    for unit in &build.analysis.units {
        if build.analysis.runtime_unit_ids.contains(&unit.id) {
            units_by_module
                .entry(unit.module.clone())
                .or_default()
                .push(unit);
        }
    }
    for units in units_by_module.values_mut() {
        units.sort_by(|left, right| {
            (left.index, left.id.as_str()).cmp(&(right.index, right.id.as_str()))
        });
    }
    let emitted_modules = units_by_module.keys().cloned().collect::<BTreeSet<_>>();

    let mut analyzed_modules = build.analysis.modules.iter().collect::<Vec<_>>();
    analyzed_modules.sort_by(|left, right| left.name.cmp(&right.name));
    let mut rendered = Vec::new();
    for module in analyzed_modules {
        let Some(units) = units_by_module.get(&module.name) else {
            continue;
        };
        let namespace_form =
            prune_namespace_form(&module.namespace_form, &all_modules, &emitted_modules)?;
        let body = units
            .iter()
            .map(|unit| unit.form_source.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let source = format!("{namespace_form}\n{body}\n");
        let dependencies = namespace_dependencies(&namespace_form)?
            .into_iter()
            .filter(|dependency| emitted_modules.contains(dependency))
            .collect();
        rendered.push(RenderedModule {
            resource: module.name.clone(),
            namespace_form,
            body,
            source,
            dependencies,
        });
    }
    if rendered.is_empty() {
        return Err("production analysis retained no runtime definition units".into());
    }
    Ok(rendered)
}

fn prune_namespace_form(
    source: &str,
    all_modules: &BTreeSet<String>,
    emitted_modules: &BTreeSet<String>,
) -> Result<String, String> {
    let Form::List(mut values) = parse(source)? else {
        return Err("production module has invalid ns form".into());
    };
    if values.len() < 2 {
        return Err("production module has incomplete ns form".into());
    }
    match values.first_mut() {
        Some(Form::Symbol(head)) if head == "ns" => {}
        Some(Form::Symbol(head)) if head == "ns+" => *head = "ns".into(),
        _ => return Err("production module must start with ns or ns+".into()),
    }

    let mut output = values.drain(..2).collect::<Vec<_>>();
    for clause in values {
        if let Some(clause) = prune_dependency_clause(clause, all_modules, emitted_modules) {
            output.push(clause);
        }
    }
    Ok(Form::List(output).to_string())
}

fn prune_dependency_clause(
    clause: Form,
    all_modules: &BTreeSet<String>,
    emitted_modules: &BTreeSet<String>,
) -> Option<Form> {
    let Form::List(mut items) = clause else {
        return Some(clause);
    };
    let dependency_clause = matches!(
        items.first(),
        Some(Form::Keyword(name)) if name == "require" || name == "use"
    );
    if !dependency_clause {
        return Some(Form::List(items));
    }
    let head = items.remove(0);
    let mut retained = vec![head];
    retained.extend(items.into_iter().filter(|spec| {
        let Some(target) = dependency_target(spec) else {
            return true;
        };
        let target = crate::kernel::generated::normalize_namespace(target);
        !all_modules.contains(target)
            || emitted_modules.contains(target)
            || core_runtime_namespace(target)
    }));
    (retained.len() > 1).then(|| Form::List(retained))
}

fn dependency_target(spec: &Form) -> Option<&str> {
    match spec {
        Form::Symbol(target) => Some(target),
        Form::Vector(items) => match items.first() {
            Some(Form::Symbol(target)) => Some(target),
            _ => None,
        },
        Form::List(items) if matches!(items.first(), Some(Form::Symbol(head)) if head == "quote") => {
            match items.get(1) {
                Some(Form::Symbol(target)) => Some(target),
                _ => None,
            }
        }
        _ => None,
    }
}

fn core_runtime_namespace(namespace: &str) -> bool {
    namespace == "std.foundation"
        || namespace == "std.native"
        || namespace.starts_with("std.native.")
        || namespace.starts_with("std.protocol.")
}

fn namespace_dependencies(namespace_form: &str) -> Result<Vec<String>, String> {
    let Form::List(items) = parse(namespace_form)? else {
        return Err("production module has invalid ns form".into());
    };
    let config = GeneratedNamespaceConfig::configure_with(&items[2..], |_| true)?;
    let mut dependencies = config.required_namespaces().to_vec();
    dependencies.extend(config.used_namespaces().iter().cloned());
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}
