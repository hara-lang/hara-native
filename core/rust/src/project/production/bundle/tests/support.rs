use super::super::super::source::SourceModule;
use super::super::super::{analyze_modules, BuildPlan};
use super::super::model::ProductionBuild;

pub fn plan(entrypoint: &str) -> BuildPlan {
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

pub fn fixture_modules() -> Vec<SourceModule> {
    vec![
        SourceModule::synthetic(
            "app.main",
            "(ns app.main (:require [app.lib :as lib]))\n(defn unused-main [] 7)\n(defn start [] (lib/used))\n",
        ),
        SourceModule::synthetic(
            "app.lib",
            "(ns app.lib)\n(defn used [] 42)\n(defn unused-lib [] 0)\n",
        ),
        SourceModule::synthetic(
            "app.unused",
            "(ns app.unused)\n(defn never-called [] :unreachable)\n",
        ),
    ]
}

pub fn analyzed(modules: Vec<SourceModule>, plan: BuildPlan) -> ProductionBuild {
    let analysis = analyze_modules(&plan, modules).expect("analyze production fixture");
    ProductionBuild { plan, analysis }
}
