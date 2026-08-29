//! Production source-to-artifact observations for HALC.
//!
//! This module is available with the existing `halc-encoder` feature (and in
//! tests). It parses source, calls the canonical HALC encoder, decodes the
//! resulting artifact, and records bounded evidence for each supported stage.
//! No module form is evaluated.

mod evidence;

pub use evidence::{
    HalcSourceTraceLimits, DEFAULT_ARTIFACT_PREVIEW_BYTES, DEFAULT_MAX_FORMS,
    DEFAULT_MAX_SCHEMA_ENTRIES, DEFAULT_MAX_TEXT_BYTES,
};

use self::evidence::{
    boolean, diagnostic_evidence, encode_failure, envelope_evidence, event, failed_trace,
    form_evidence, integer, module_identity_evidence, module_projection_equal, next_event,
    normalize_id, payload_evidence, schema_evidence, sha256, string, structural_forms,
};
use super::halc::{decode_halc, encode_halc_module};
use super::halc_trace::{
    inspect_halc_artifact, HalcArtifactTrace, HalcTraceEvidence, HalcTraceStatus, HALC_TRACE_SCHEMA,
};
use super::parse_forms;

pub fn trace_halc_source(
    id: impl Into<String>,
    namespace: &str,
    resource: &str,
    source: &str,
) -> HalcArtifactTrace {
    trace_halc_source_with_limits(
        id,
        namespace,
        resource,
        source,
        HalcSourceTraceLimits::default(),
    )
}

pub fn trace_halc_source_with_limits(
    id: impl Into<String>,
    namespace: &str,
    resource: &str,
    source: &str,
    limits: HalcSourceTraceLimits,
) -> HalcArtifactTrace {
    let id = normalize_id(id);
    let source_hash = sha256(source.as_bytes());
    let parsed_forms = match parse_forms(source) {
        Ok(forms) => forms,
        Err(error) => {
            let message = error.to_string();
            let evidence = diagnostic_evidence("source/read", "source/parse", &message);
            let events = vec![event(
                1,
                "source/read",
                HalcTraceStatus::Error,
                evidence,
                Some(message.clone()),
            )];
            return failed_trace(id, events, message);
        }
    };

    let mut events = Vec::new();
    let mut source_evidence = form_evidence(&parsed_forms, limits);
    source_evidence.insert("source/bytes".into(), integer(source.len()));
    source_evidence.insert("source/hash".into(), string(&source_hash));
    next_event(
        &mut events,
        "source/read",
        HalcTraceStatus::Ok,
        source_evidence,
        None,
    );
    next_event(
        &mut events,
        "module/identity",
        HalcTraceStatus::Ok,
        module_identity_evidence(namespace, resource, &source_hash),
        None,
    );

    let artifact = match encode_halc_module(namespace, resource, source, parsed_forms.clone()) {
        Ok(artifact) => artifact,
        Err(error) => {
            let (stage, category) = encode_failure(&error);
            let evidence = diagnostic_evidence(stage, category, &error);
            next_event(
                &mut events,
                stage,
                HalcTraceStatus::Error,
                evidence,
                Some(error.clone()),
            );
            return failed_trace(id, events, error);
        }
    };

    let module = match decode_halc(&artifact) {
        Ok(module) => module,
        Err(error) => {
            let evidence =
                diagnostic_evidence("artifact/validate", "artifact/generated-invalid", &error);
            next_event(
                &mut events,
                "artifact/validate",
                HalcTraceStatus::Error,
                evidence,
                Some(error.clone()),
            );
            return failed_trace(id, events, error);
        }
    };

    let inspection = match inspect_halc_artifact(&artifact) {
        Ok(inspection) => inspection,
        Err(error) => {
            let evidence = diagnostic_evidence("artifact/validate", "artifact/inspection", &error);
            next_event(
                &mut events,
                "artifact/validate",
                HalcTraceStatus::Error,
                evidence,
                Some(error.clone()),
            );
            return failed_trace(id, events, error);
        }
    };

    let mut canonical_evidence = form_evidence(&module.forms, limits);
    canonical_evidence.insert(
        "forms/changed".into(),
        boolean(structural_forms(&parsed_forms) != structural_forms(&module.forms)),
    );
    next_event(
        &mut events,
        "forms/canonicalize",
        HalcTraceStatus::Ok,
        canonical_evidence.clone(),
        None,
    );

    let schemas = schema_evidence(&module, limits);
    next_event(
        &mut events,
        "schema/index",
        HalcTraceStatus::Ok,
        schemas.clone(),
        None,
    );

    let payload = payload_evidence(&inspection, &artifact);
    next_event(
        &mut events,
        "payload/encode",
        HalcTraceStatus::Ok,
        payload.clone(),
        None,
    );

    let envelope = envelope_evidence(&inspection, &artifact, limits);
    next_event(
        &mut events,
        "envelope/build",
        HalcTraceStatus::Ok,
        envelope.clone(),
        None,
    );

    let source_hash_matches = inspection.source_hash == source_hash;
    let mut validation = HalcTraceEvidence::new();
    validation.insert("artifact/valid".into(), boolean(true));
    validation.insert("source/hash-matches".into(), boolean(source_hash_matches));
    validation.insert(
        "payload/checksum".into(),
        string(&inspection.payload_checksum),
    );
    next_event(
        &mut events,
        "artifact/validate",
        HalcTraceStatus::Ok,
        validation.clone(),
        None,
    );

    let replay_artifact = match encode_halc_module(
        &module.namespace,
        &module.resource,
        source,
        module.forms.clone(),
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            let evidence = diagnostic_evidence("artifact/decode", "decode/reencode", &error);
            next_event(
                &mut events,
                "artifact/decode",
                HalcTraceStatus::Error,
                evidence,
                Some(error.clone()),
            );
            return failed_trace(id, events, error);
        }
    };
    let replay_module = match decode_halc(&replay_artifact) {
        Ok(module) => module,
        Err(error) => {
            let evidence = diagnostic_evidence("artifact/decode", "decode/replay-invalid", &error);
            next_event(
                &mut events,
                "artifact/decode",
                HalcTraceStatus::Error,
                evidence,
                Some(error.clone()),
            );
            return failed_trace(id, events, error);
        }
    };

    let artifact_byte_parity = artifact == replay_artifact;
    let module_parity = module_projection_equal(&module, &replay_module);
    let parity = source_hash_matches && artifact_byte_parity && module_parity;
    let mut decoded = HalcTraceEvidence::new();
    decoded.insert("decode/parity".into(), boolean(parity));
    decoded.insert(
        "decode/artifact-byte-parity".into(),
        boolean(artifact_byte_parity),
    );
    decoded.insert(
        "decode/module-projection-parity".into(),
        boolean(module_parity),
    );
    decoded.insert("form/count".into(), integer(module.forms.len()));
    decoded.insert(
        "schema/definition-count".into(),
        integer(module.schemas.definitions.len()),
    );
    decoded.insert(
        "schema/function-count".into(),
        integer(module.schemas.functions.len()),
    );
    if !parity {
        let message = "HALC encode/decode projection parity failed".to_owned();
        decoded.insert(
            "diagnostic/category".into(),
            string("decode/parity-mismatch"),
        );
        decoded.insert("diagnostic/message".into(), string(&message));
        next_event(
            &mut events,
            "artifact/decode",
            HalcTraceStatus::Error,
            decoded,
            Some(message.clone()),
        );
        return failed_trace(id, events, message);
    }
    next_event(
        &mut events,
        "artifact/decode",
        HalcTraceStatus::Ok,
        decoded.clone(),
        None,
    );

    let mut result = module_identity_evidence(namespace, resource, &source_hash);
    result.extend(canonical_evidence);
    result.extend(schemas);
    result.extend(payload);
    result.extend(envelope);
    result.extend(validation);
    result.extend(decoded);

    HalcArtifactTrace {
        schema: HALC_TRACE_SCHEMA,
        id,
        status: HalcTraceStatus::Ok,
        events,
        result: Some(result),
        error: None,
    }
}

#[cfg(test)]
mod tests;
