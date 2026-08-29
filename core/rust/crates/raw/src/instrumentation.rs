//! Minimal instrumentation data surface required by the shared evaluator.
//!
//! Raw Wasm does not host the trusted instrumentation hub, but the evaluator
//! still publishes portable observation values at its internal safepoints.

#[path = "../../src/instrumentation/model.rs"]
mod model;

pub use model::{EventKind, EventLocation, EventPhase, ProjectionLimits};

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortableProjection {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

impl PortableProjection {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            fields: BTreeMap::new(),
        }
    }

    pub fn with_field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(name.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerEvent {
    pub phase: EventPhase,
    pub event: EventKind,
    pub data: BTreeMap<String, String>,
}

impl ProducerEvent {
    pub fn live(event: EventKind) -> Self {
        Self {
            phase: EventPhase::Live,
            event,
            data: BTreeMap::new(),
        }
    }

    pub fn with_data(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(name.into(), value.into());
        self
    }
}
