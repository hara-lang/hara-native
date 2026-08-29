use super::super::source::{Diagnostic, SourceLocation};
use super::{Effect, UnitKind};
use std::collections::BTreeSet;

/// Canonical native/runtime roots discovered while compiling one expanded
/// definition unit. The compatibility projections on [`UnitAnalysis`] remain
/// until the target generators consume this typed inventory directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NativeRootInventory {
    pub(crate) primitives: BTreeSet<String>,
    pub(crate) methods: BTreeSet<String>,
    pub(crate) dynamic_methods: BTreeSet<String>,
    pub(crate) types: BTreeSet<String>,
    pub(crate) protocols: BTreeSet<String>,
    pub(crate) protocol_methods: BTreeSet<String>,
    pub(crate) multimethods: BTreeSet<String>,
    pub(crate) host_calls: BTreeSet<String>,
    pub(crate) callbacks: BTreeSet<String>,
    pub(crate) runtime_shims: BTreeSet<String>,
}

impl NativeRootInventory {
    pub(crate) fn extend(&mut self, other: &Self) {
        self.primitives.extend(other.primitives.iter().cloned());
        self.methods.extend(other.methods.iter().cloned());
        self.dynamic_methods
            .extend(other.dynamic_methods.iter().cloned());
        self.types.extend(other.types.iter().cloned());
        self.protocols.extend(other.protocols.iter().cloned());
        self.protocol_methods
            .extend(other.protocol_methods.iter().cloned());
        self.multimethods.extend(other.multimethods.iter().cloned());
        self.host_calls.extend(other.host_calls.iter().cloned());
        self.callbacks.extend(other.callbacks.iter().cloned());
        self.runtime_shims
            .extend(other.runtime_shims.iter().cloned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitAnalysis {
    pub id: String,
    pub module: String,
    pub index: usize,
    /// Deterministic source for the macro-expanded top-level unit that was
    /// analyzed. Production emission compiles this exact source instead of
    /// reparsing or re-expanding the complete module.
    pub form_source: String,
    pub kind: UnitKind,
    pub effect: Effect,
    pub location: SourceLocation,
    pub provides: BTreeSet<String>,
    pub runtime_edges: BTreeSet<String>,
    pub compile_time_edges: BTreeSet<String>,
    pub namespace_edges: BTreeSet<String>,
    /// Typed root contract consumed by #553 runtime specialization.
    pub(crate) native_roots: NativeRootInventory,
    /// Compatibility projections retained for the existing 0-alpha report.
    pub native_primitives: BTreeSet<String>,
    pub native_types: BTreeSet<String>,
    pub native_protocols: BTreeSet<String>,
    pub diagnostics: Vec<Diagnostic>,
}
