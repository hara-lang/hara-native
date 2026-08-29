use super::super::{graph::Analysis, plan::BuildPlan};
use crate::vm::BytecodeBundleModule;

#[derive(Debug, Clone)]
pub(in crate::task::production) struct ProductionBuild {
    pub(in crate::task::production) plan: BuildPlan,
    pub(in crate::task::production) analysis: Analysis,
}

#[derive(Clone)]
pub(in crate::task::production) struct CompiledBundle {
    pub(in crate::task::production) bytes: Vec<u8>,
    pub(in crate::task::production) modules: Vec<BytecodeBundleModule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderedModule {
    pub(super) resource: String,
    pub(super) namespace_form: String,
    pub(super) body: String,
    pub(super) source: String,
    pub(super) dependencies: Vec<String>,
}
