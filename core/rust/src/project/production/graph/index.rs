use super::super::source::Diagnostic;
use super::model::{project_location, RetentionReason};
use super::UnitAnalysis;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) fn provider_index(units: &[UnitAnalysis]) -> BTreeMap<String, String> {
    let mut output = BTreeMap::new();
    for unit in units {
        for provided in &unit.provides {
            // Namespace evaluation is ordered. A later real definition must
            // replace an earlier `declare` placeholder or deliberate rebind.
            output.insert(provided.clone(), unit.id.clone());
        }
    }
    output
}

pub(super) fn namespace_index(units: &[UnitAnalysis]) -> BTreeMap<String, Vec<usize>> {
    let mut output = BTreeMap::<String, Vec<usize>>::new();
    for (index, unit) in units.iter().enumerate() {
        output.entry(unit.module.clone()).or_default().push(index);
    }
    output
}

pub(super) fn project_diagnostic(
    code: &str,
    operation: &str,
    subject: &str,
    message: String,
) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        operation: operation.into(),
        module: subject
            .split_once('/')
            .map_or(subject, |(namespace, _)| namespace)
            .into(),
        location: project_location(),
        message,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn retain(
    unit_id: &str,
    subject: Option<String>,
    code: &str,
    detail: &str,
    retained: &mut BTreeSet<String>,
    reasons: &mut Vec<RetentionReason>,
    queue: &mut VecDeque<String>,
) {
    reasons.push(RetentionReason {
        unit_id: unit_id.into(),
        subject,
        code: code.into(),
        detail: detail.into(),
    });
    if retained.insert(unit_id.into()) {
        queue.push_back(unit_id.into());
    }
}
