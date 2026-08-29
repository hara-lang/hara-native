use super::super::halc::{HalcModule, HalcOrigin};
use super::super::halc_trace::{
    HalcArtifactInspection, HalcArtifactTrace, HalcTraceEvent, HalcTraceEvidence, HalcTraceStatus,
    HalcTraceValue, HALC_TRACE_SCHEMA,
};
use super::super::{Form, SchemaType};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const DEFAULT_MAX_FORMS: usize = 32;
pub const DEFAULT_MAX_SCHEMA_ENTRIES: usize = 64;
pub const DEFAULT_MAX_TEXT_BYTES: usize = 512;
pub const DEFAULT_ARTIFACT_PREVIEW_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalcSourceTraceLimits {
    pub max_forms: usize,
    pub max_schema_entries: usize,
    pub max_text_bytes: usize,
    pub max_artifact_preview_bytes: usize,
}

impl Default for HalcSourceTraceLimits {
    fn default() -> Self {
        Self {
            max_forms: DEFAULT_MAX_FORMS,
            max_schema_entries: DEFAULT_MAX_SCHEMA_ENTRIES,
            max_text_bytes: DEFAULT_MAX_TEXT_BYTES,
            max_artifact_preview_bytes: DEFAULT_ARTIFACT_PREVIEW_BYTES,
        }
    }
}

pub(super) fn string(value: impl Into<String>) -> HalcTraceValue {
    HalcTraceValue::String(value.into())
}

pub(super) fn integer(value: usize) -> HalcTraceValue {
    HalcTraceValue::Integer(value as u64)
}

pub(super) fn boolean(value: bool) -> HalcTraceValue {
    HalcTraceValue::Boolean(value)
}

fn strings(values: Vec<String>) -> HalcTraceValue {
    HalcTraceValue::Strings(values)
}

pub(super) fn event(
    id: u64,
    stage: &'static str,
    status: HalcTraceStatus,
    evidence: HalcTraceEvidence,
    error: Option<String>,
) -> HalcTraceEvent {
    HalcTraceEvent {
        id,
        sequence: id,
        stage,
        status,
        evidence,
        error,
    }
}

pub(super) fn next_event(
    events: &mut Vec<HalcTraceEvent>,
    stage: &'static str,
    status: HalcTraceStatus,
    evidence: HalcTraceEvidence,
    error: Option<String>,
) {
    let id = events.len() as u64 + 1;
    events.push(event(id, stage, status, evidence, error));
}

pub(super) fn normalize_id(id: impl Into<String>) -> String {
    let id = id.into();
    if id.is_empty() {
        "halc-source".to_owned()
    } else {
        id
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn bounded_text(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_owned(), false);
    }
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

fn bounded_strings(
    values: Vec<String>,
    max_items: usize,
    max_text_bytes: usize,
) -> (Vec<String>, bool) {
    let mut truncated = values.len() > max_items;
    let mut output = Vec::with_capacity(values.len().min(max_items));
    for value in values.into_iter().take(max_items) {
        let (value, text_truncated) = bounded_text(&value, max_text_bytes);
        truncated |= text_truncated;
        output.push(value);
    }
    (output, truncated)
}

pub(super) fn structural_forms(forms: &[Form]) -> Vec<String> {
    forms
        .iter()
        .map(|form| format!("{form:?}"))
        .collect::<Vec<_>>()
}

pub(super) fn form_evidence(forms: &[Form], limits: HalcSourceTraceLimits) -> HalcTraceEvidence {
    let readable = forms.iter().map(ToString::to_string).collect::<Vec<_>>();
    let structural = structural_forms(forms);
    let (readable, readable_truncated) =
        bounded_strings(readable, limits.max_forms, limits.max_text_bytes);
    let (structural, structural_truncated) =
        bounded_strings(structural, limits.max_forms, limits.max_text_bytes);
    let mut evidence = HalcTraceEvidence::new();
    evidence.insert("form/count".into(), integer(forms.len()));
    evidence.insert("forms/readable".into(), strings(readable));
    evidence.insert("forms/structural".into(), strings(structural));
    evidence.insert(
        "forms/truncated".into(),
        boolean(readable_truncated || structural_truncated),
    );
    evidence
}

fn sorted_names<T>(values: &HashMap<String, T>) -> Vec<String> {
    let mut names = values.keys().cloned().collect::<Vec<_>>();
    names.sort();
    names
}

fn type_values(values: &HashMap<String, SchemaType>) -> Vec<String> {
    let mut output = values
        .iter()
        .map(|(name, schema)| format!("{name}={schema:?}"))
        .collect::<Vec<_>>();
    output.sort();
    output
}

pub(super) fn schema_evidence(
    module: &HalcModule,
    limits: HalcSourceTraceLimits,
) -> HalcTraceEvidence {
    let definitions = sorted_names(&module.schemas.definitions);
    let functions = sorted_names(&module.schemas.functions);
    let definition_types = type_values(&module.schemas.definition_types);
    let function_types = type_values(&module.schemas.function_types);
    let mut resolved_functions = functions
        .iter()
        .filter_map(|name| {
            module
                .schemas
                .resolved_function_type(name)
                .map(|schema| format!("{name}={schema:?}"))
        })
        .collect::<Vec<_>>();
    resolved_functions.sort();

    let (definitions, definitions_truncated) = bounded_strings(
        definitions,
        limits.max_schema_entries,
        limits.max_text_bytes,
    );
    let (functions, functions_truncated) =
        bounded_strings(functions, limits.max_schema_entries, limits.max_text_bytes);
    let (definition_types, definition_types_truncated) = bounded_strings(
        definition_types,
        limits.max_schema_entries,
        limits.max_text_bytes,
    );
    let (function_types, function_types_truncated) = bounded_strings(
        function_types,
        limits.max_schema_entries,
        limits.max_text_bytes,
    );
    let (resolved_functions, resolved_functions_truncated) = bounded_strings(
        resolved_functions,
        limits.max_schema_entries,
        limits.max_text_bytes,
    );

    let mut evidence = HalcTraceEvidence::new();
    evidence.insert("schema/definitions".into(), strings(definitions));
    evidence.insert("schema/functions".into(), strings(functions));
    evidence.insert("schema/definition-types".into(), strings(definition_types));
    evidence.insert("schema/function-types".into(), strings(function_types));
    evidence.insert(
        "schema/resolved-functions".into(),
        strings(resolved_functions),
    );
    evidence.insert(
        "schema/truncated".into(),
        boolean(
            definitions_truncated
                || functions_truncated
                || definition_types_truncated
                || function_types_truncated
                || resolved_functions_truncated,
        ),
    );
    evidence
}

fn origin_name(origin: HalcOrigin) -> &'static str {
    match origin {
        HalcOrigin::Halc => "halc",
        HalcOrigin::LegacyHir => "legacy-hir",
    }
}

pub(super) fn module_identity_evidence(
    namespace: &str,
    resource: &str,
    source_hash: &str,
) -> HalcTraceEvidence {
    let mut evidence = HalcTraceEvidence::new();
    evidence.insert("module/namespace".into(), string(namespace));
    evidence.insert("module/resource".into(), string(resource));
    evidence.insert("source/hash".into(), string(source_hash));
    evidence
}

pub(super) fn payload_evidence(
    inspection: &HalcArtifactInspection,
    artifact: &[u8],
) -> HalcTraceEvidence {
    let mut evidence = HalcTraceEvidence::new();
    evidence.insert(
        "payload/bytes".into(),
        HalcTraceValue::Integer(inspection.payload_length as u64),
    );
    evidence.insert(
        "payload/checksum".into(),
        string(&inspection.payload_checksum),
    );
    evidence.insert("artifact/bytes".into(), integer(artifact.len()));
    evidence.insert("artifact/digest".into(), string(sha256(artifact)));
    evidence
}

pub(super) fn envelope_evidence(
    inspection: &HalcArtifactInspection,
    artifact: &[u8],
    limits: HalcSourceTraceLimits,
) -> HalcTraceEvidence {
    let preview_bytes = artifact.len().min(limits.max_artifact_preview_bytes);
    let mut evidence = payload_evidence(inspection, artifact);
    evidence.insert(
        "artifact/origin".into(),
        string(origin_name(inspection.origin)),
    );
    evidence.insert(
        "format/version".into(),
        HalcTraceValue::Integer(inspection.format_version as u64),
    );
    evidence.insert(
        "format/flags".into(),
        HalcTraceValue::Integer(inspection.flags as u64),
    );
    evidence.insert(
        "artifact/preview-hex".into(),
        string(hex(&artifact[..preview_bytes])),
    );
    evidence.insert("artifact/preview-bytes".into(), integer(preview_bytes));
    evidence.insert(
        "artifact/preview-truncated".into(),
        boolean(preview_bytes < artifact.len()),
    );
    evidence
}

pub(super) fn diagnostic_evidence(
    stage: &'static str,
    category: &'static str,
    message: &str,
) -> HalcTraceEvidence {
    let mut evidence = HalcTraceEvidence::new();
    evidence.insert("diagnostic/stage".into(), string(stage));
    evidence.insert("diagnostic/category".into(), string(category));
    evidence.insert("diagnostic/message".into(), string(message));
    evidence
}

pub(super) fn failed_trace(
    id: String,
    events: Vec<HalcTraceEvent>,
    error: impl Into<String>,
) -> HalcArtifactTrace {
    let error = error.into();
    HalcArtifactTrace {
        schema: HALC_TRACE_SCHEMA,
        id,
        status: HalcTraceStatus::Error,
        events,
        result: None,
        error: Some(error),
    }
}

pub(super) fn encode_failure(error: &str) -> (&'static str, &'static str) {
    let lower = error.to_ascii_lowercase();
    if lower.contains("schema") || lower.contains("reference") {
        ("schema/index", "schema/invalid")
    } else if lower.contains("canonical") {
        ("forms/canonicalize", "forms/canonicalization")
    } else {
        ("payload/encode", "artifact/encode")
    }
}

pub(super) fn module_projection_equal(left: &HalcModule, right: &HalcModule) -> bool {
    left.namespace == right.namespace
        && left.resource == right.resource
        && left.source_hash == right.source_hash
        && structural_forms(&left.forms) == structural_forms(&right.forms)
        && left.schemas == right.schemas
        && left.origin == right.origin
}
