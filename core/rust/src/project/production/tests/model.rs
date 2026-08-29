use super::super::source::SourceModule;
use super::super::unit;
use super::super::{analyze_modules, report, BuildPlan};
use crate::kernel::parse;
use std::collections::BTreeSet;

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
fn records_struct_protocol_and_multimethod_providers() {
    let forms = crate::kernel::parse_forms(
        "(defstruct Point [x y]) (defprotocol Shape (area [value])) (defmulti render type)",
    )
    .unwrap();
    let provided = forms
        .iter()
        .flat_map(|form| unit::raw_provided_vars(form, "app.model"))
        .collect::<BTreeSet<_>>();
    let expected = [
        "app.model/->Point",
        "app.model/Point",
        "app.model/Shape",
        "app.model/area",
        "app.model/map->Point",
        "app.model/render",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    assert_eq!(provided, expected);
}

#[test]
fn emits_byte_identical_analysis_reports() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(defn start [] 42)\n(defn unused [] 0)\n",
    )];
    let build = plan("app.main/start");
    let first = analyze_modules(&build, modules.clone()).unwrap();
    let second = analyze_modules(&build, modules).unwrap();
    let first = report::report_source(&build, &first, None);
    let second = report::report_source(&build, &second, None);
    assert_eq!(first, second);
    let parsed = parse(&first).unwrap();
    assert!(parsed
        .to_string()
        .contains("hara.production-analysis/0-alpha"));
}

#[test]
fn missing_entrypoints_are_deterministic_analysis_diagnostics() {
    let modules = vec![SourceModule::synthetic(
        "app.main",
        "(ns app.main)\n(defn other [] 42)\n",
    )];
    let analysis = analyze_modules(&plan("app.main/start"), modules).unwrap();
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "production/missing-entrypoint")
            .count(),
        1
    );
}
