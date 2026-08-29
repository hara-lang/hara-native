#[path = "report/bundle.rs"]
mod bundle;
#[path = "report/form.rs"]
mod form;

use super::graph::{Analysis, ModuleAnalysis, RetentionReason};
use super::plan::BuildPlan;
use super::source::Diagnostic;
use super::unit::UnitAnalysis;
use crate::kernel::Form;
use form::{
    boolean, keyword, map, nil, number, source_form, string, string_vector, strings, symbol,
    symbol_vector, symbols, vector,
};

pub const ANALYSIS_FORMAT: &str = "hara.production-analysis/0-alpha";
pub const SHAKE_FORMAT: &str = "hara.production-shake/0-alpha";

pub(super) use bundle::BundleSummary;

pub fn report_source(
    plan: &BuildPlan,
    analysis: &Analysis,
    output: Option<&BundleSummary>,
) -> String {
    format!("{}\n", report_form(plan, analysis, output))
}

fn report_form(plan: &BuildPlan, analysis: &Analysis, output: Option<&BundleSummary>) -> Form {
    let dynamic_diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code.contains("dynamic")
                || diagnostic.code.contains("eval")
                || diagnostic.code.contains("generated-source")
                || diagnostic.code.contains("load-string")
        })
        .map(diagnostic_form);
    let status = if !analysis.succeeded() {
        "analysis-failed"
    } else if output.is_some() {
        "bundle-complete"
    } else {
        "analysis-complete"
    };
    map(vec![
        ("shake/format", string(SHAKE_FORMAT)),
        ("shake/status", keyword(status)),
        (
            "shake/project",
            map(vec![
                ("project/id", symbol(&plan.project_id)),
                ("project/version", string(&plan.project_version)),
            ]),
        ),
        ("shake/profile", keyword(&plan.profile)),
        ("shake/language", keyword(&plan.language)),
        ("shake/main", symbol(&plan.main)),
        (
            "shake/entrypoints",
            symbol_vector(plan.entrypoints.iter().cloned()),
        ),
        ("shake/default-entrypoint", symbol(&plan.default_entrypoint)),
        (
            "shake/keep-vars",
            symbol_vector(plan.keep_vars.iter().cloned()),
        ),
        (
            "shake/keep-namespaces",
            symbol_vector(plan.keep_namespaces.iter().cloned()),
        ),
        (
            "shake/retained",
            map(vec![
                ("vars", symbols(&analysis.retained_vars)),
                ("namespaces", symbols(&analysis.retained_namespaces)),
            ]),
        ),
        (
            "shake/removed",
            map(vec![
                ("vars", symbols(&analysis.removed_vars)),
                ("namespaces", symbols(&analysis.removed_namespaces)),
            ]),
        ),
        (
            "shake/retention-reasons",
            vector(analysis.reasons.iter().map(reason_form)),
        ),
        (
            "shake/dynamic-access-diagnostics",
            vector(dynamic_diagnostics),
        ),
        (
            "shake/native-roots",
            map(vec![
                ("primitives", strings(&analysis.native_primitives)),
                ("types", strings(&analysis.native_types)),
                ("protocols", strings(&analysis.native_protocols)),
            ]),
        ),
        (
            "shake/sizes",
            map(vec![
                ("input-bytes", number(analysis.input_bytes)),
                (
                    "output-bytes",
                    output.map_or_else(nil, |summary| number(summary.output_bytes)),
                ),
            ]),
        ),
        (
            "shake/digests",
            map(vec![
                ("algorithm", keyword("sha-256")),
                ("input", string(&analysis.input_digest)),
                (
                    "output",
                    output.map_or_else(nil, |summary| string(&summary.output_digest)),
                ),
            ]),
        ),
        (
            "shake/output",
            map(vec![
                ("bundle", string(&plan.output_bundle)),
                ("report", string(&plan.output_report)),
                (
                    "module-count",
                    output.map_or_else(nil, |summary| number(summary.module_count)),
                ),
            ]),
        ),
        ("shake/analysis", analysis_form(analysis)),
    ])
}

fn analysis_form(analysis: &Analysis) -> Form {
    let mut modules = analysis.modules.iter().collect::<Vec<_>>();
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    let mut units = analysis.units.iter().collect::<Vec<_>>();
    units.sort_by(|left, right| left.id.cmp(&right.id));
    map(vec![
        ("analysis/format", string(ANALYSIS_FORMAT)),
        (
            "analysis/modules",
            vector(modules.into_iter().map(module_form)),
        ),
        ("analysis/units", vector(units.into_iter().map(unit_form))),
        (
            "analysis/runtime",
            map(vec![
                ("roots", symbols(&analysis.runtime_roots)),
                ("closure", symbols(&analysis.runtime_closure)),
                (
                    "unit-ids",
                    string_vector(analysis.runtime_unit_ids.iter().cloned()),
                ),
            ]),
        ),
        (
            "analysis/compile-time",
            map(vec![
                ("roots", symbols(&analysis.compile_time_roots)),
                ("closure", symbols(&analysis.compile_time_closure)),
                (
                    "unit-ids",
                    string_vector(analysis.compile_time_unit_ids.iter().cloned()),
                ),
            ]),
        ),
        (
            "analysis/retained-unit-ids",
            string_vector(analysis.retained_unit_ids.iter().cloned()),
        ),
        (
            "analysis/removed-unit-ids",
            string_vector(analysis.removed_unit_ids.iter().cloned()),
        ),
        (
            "analysis/diagnostics",
            vector(analysis.diagnostics.iter().map(diagnostic_form)),
        ),
    ])
}

fn module_form(module: &ModuleAnalysis) -> Form {
    map(vec![
        ("module/name", symbol(&module.name)),
        ("module/path", string(&module.path)),
        ("module/digest", string(&module.digest)),
        ("module/input-bytes", number(module.input_bytes)),
        (
            "module/dependencies",
            symbol_vector(module.dependencies.iter().cloned()),
        ),
        (
            "module/unit-ids",
            string_vector(module.unit_ids.iter().cloned()),
        ),
        ("module/standard-library", boolean(module.standard_library)),
    ])
}

fn unit_form(unit: &UnitAnalysis) -> Form {
    map(vec![
        ("unit/id", string(&unit.id)),
        ("unit/module", symbol(&unit.module)),
        ("unit/index", number(unit.index)),
        ("unit/kind", keyword(unit.kind.keyword())),
        ("unit/effect", keyword(unit.effect.keyword())),
        ("unit/source", source_form(&unit.location)),
        ("unit/provides", symbols(&unit.provides)),
        ("unit/runtime-edges", symbols(&unit.runtime_edges)),
        ("unit/compile-time-edges", symbols(&unit.compile_time_edges)),
        ("unit/namespace-edges", symbols(&unit.namespace_edges)),
        ("unit/native-primitives", strings(&unit.native_primitives)),
        ("unit/native-types", strings(&unit.native_types)),
        ("unit/native-protocols", strings(&unit.native_protocols)),
        (
            "unit/diagnostics",
            vector(unit.diagnostics.iter().map(diagnostic_form)),
        ),
    ])
}

fn reason_form(reason: &RetentionReason) -> Form {
    map(vec![
        ("reason/unit", string(&reason.unit_id)),
        (
            "reason/subject",
            reason.subject.as_deref().map_or_else(nil, symbol),
        ),
        (
            "reason/code",
            keyword(&format!("production/{}", reason.code)),
        ),
        ("reason/detail", string(&reason.detail)),
    ])
}

fn diagnostic_form(diagnostic: &Diagnostic) -> Form {
    map(vec![
        ("diagnostic/code", keyword(&diagnostic.code)),
        ("diagnostic/operation", keyword(&diagnostic.operation)),
        ("diagnostic/module", symbol(&diagnostic.module)),
        ("diagnostic/source", source_form(&diagnostic.location)),
        ("diagnostic/message", string(&diagnostic.message)),
    ])
}
