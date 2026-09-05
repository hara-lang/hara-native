//! Restricted `.hal` interface contracts for portable Wasm extension bindings.
//!
//! Sources are parsed as data with the Hara reader. This module never evaluates
//! an interface, instantiates a module, or acquires host authority.

#[cfg(not(target_arch = "wasm32"))]
mod adapter;
mod canonical;
mod direct;
mod memory;
#[cfg(not(target_arch = "wasm32"))]
mod package;
mod parser;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;
mod syntax;
pub mod wit;
mod wit_format;
mod wit_parser;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::extension::ExtensionExport;

pub use crate::direct_wasm::{
    DirectWasmFunctionExport, DirectWasmImport, DirectWasmImportKind, DirectWasmInspection,
    DirectWasmMemory,
};
#[cfg(not(target_arch = "wasm32"))]
pub use adapter::{
    generate_adapter, generate_hta_adapter, verify_hta_scalar, AdapterArtifact,
    ADAPTER_MANIFEST_SCHEMA,
};
pub use direct::{
    direct_inspection_source, direct_interface_skeleton, inspect_direct,
    DIRECT_WASM_INSPECTION_SCHEMA,
};
pub use memory::{
    MemoryArgumentPlan, MemoryBindingPlan, MemoryFunctionPlan, MemoryResultPlan,
    MEMORY_BINDING_SCHEMA,
};
#[cfg(not(target_arch = "wasm32"))]
pub use package::{
    bind_package, inspect_module, write_interface_skeleton, BindingTarget, BoundPackage,
    InspectionArtifact, DIRECT_WASM_BINDING_SCHEMA, DIRECT_WASM_BUILD_PRODUCT_SCHEMA,
    DIRECT_WASM_CONFORMANCE_SCHEMA,
};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::WasmtimeMemoryExecutor;
pub use wit::{
    import_wit, project_wit, WitDiagnostic, WitDiagnosticSeverity, WitImportArtifact,
    WitImportOptions, WitProjectionArtifact, WitProjectionOptions, WitRoute, WIT_IR_SCHEMA,
    WIT_MANIFEST_SCHEMA,
};

/// Verifies that a Component manifest names a package and world actually
/// declared by its integrity-pinned WIT source. This remains a data-only
/// parse and never acquires host capabilities.
pub fn validate_component_wit_world(
    source: &str,
    package: &str,
    world: &str,
) -> Result<(), String> {
    let document = wit_parser::parse(source)
        .map_err(|error| format!("wasm-wit/malformed component manifest: {error}"))?;
    if document.package.as_deref() != Some(package) {
        return Err("wasm-wit/package-mismatch: manifest package is not declared by WIT".into());
    }
    if !document.worlds.contains_key(world) {
        return Err("wasm-wit/world-missing: manifest world is not declared by WIT".into());
    }
    Ok(())
}

/// Verifies the synchronous callable surface declared by a Component manifest
/// against the selected standard WIT world.  The manifest may give a Hara
/// function a friendlier public name, but its raw Component export must be a
/// direct world function or an `interface::function` export from that world.
///
/// This is deliberately data-only: it validates the integrity-pinned WIT
/// source before the host links imports or instantiates the Component.
pub fn validate_component_wit_contract(
    source: &str,
    package: &str,
    world: &str,
    exports: &[(String, ExtensionExport)],
) -> Result<(), String> {
    validate_component_wit_contract_with_dependencies(source, package, world, exports, &[])
}

/// Validates an extension WIT contract together with its source-pinned WIT
/// dependencies. `:value` is admitted only for the standard imported
/// `hara:values/values@0.1.0.{value}` contract.
pub fn validate_component_wit_contract_with_dependencies(
    source: &str,
    package: &str,
    world: &str,
    exports: &[(String, ExtensionExport)],
    dependencies: &[(String, String)],
) -> Result<(), String> {
    let document = wit_parser::parse(source)
        .map_err(|error| format!("wasm-wit/malformed component manifest: {error}"))?;
    if document.package.as_deref() != Some(package) {
        return Err("wasm-wit/package-mismatch: manifest package is not declared by WIT".into());
    }
    let selected = document.worlds.get(world).ok_or_else(|| {
        "wasm-wit/world-missing: manifest world is not declared by WIT".to_owned()
    })?;
    let hara_values_available = hara_values_dependency_is_valid(dependencies)?;
    let mut declared = BTreeMap::new();
    if selected.functions.is_empty() {
        for interface_name in &selected.exports {
            let interface = document.interfaces.get(interface_name).ok_or_else(|| {
                format!(
                    "wasm-wit/export-mismatch: world {world} exports unknown interface {interface_name}"
                )
            })?;
            for function in &interface.functions {
                let raw_name =
                    component_interface_export_name(package, interface_name, &function.name)?;
                if declared
                    .insert(raw_name.clone(), (function, interface))
                    .is_some()
                {
                    return Err(format!(
                        "wasm-wit/export-mismatch: world {world} has duplicate Component export {raw_name}"
                    ));
                }
            }
        }
    } else {
        let interface = document.interfaces.get(world).ok_or_else(|| {
            format!("wasm-wit/export-mismatch: world {world} has no callable interface")
        })?;
        for function in &selected.functions {
            if declared
                .insert(function.name.clone(), (function, interface))
                .is_some()
            {
                return Err(format!(
                    "wasm-wit/export-mismatch: world {world} has duplicate Component export {}",
                    function.name
                ));
            }
        }
    }
    if declared.len() != exports.len() {
        return Err(format!(
            "wasm-wit/export-mismatch: WIT world {world} declares {:?}, manifest declares {:?}",
            declared.keys().collect::<Vec<_>>(),
            exports
                .iter()
                .map(|(name, specification)| specification.raw_name(name))
                .collect::<Vec<_>>()
        ));
    }
    for (public_name, specification) in exports {
        let raw_name = specification.raw_name(public_name);
        let (function, interface) = declared.get(raw_name).ok_or_else(|| {
            format!(
                "wasm-wit/export-mismatch: manifest export {public_name} maps to undeclared WIT export {raw_name}"
            )
        })?;
        if specification.asynchronous || function.async_ {
            return Err(format!(
                "wasm-wit/async-unsupported: Component export {raw_name} is asynchronous"
            ));
        }
        if function.arguments.len() != specification.arguments.len() {
            return Err(format!(
                "wasm-wit/export-mismatch: {raw_name} has {} WIT arguments but {} manifest arguments",
                function.arguments.len(),
                specification.arguments.len()
            ));
        }
        for ((_, wit_type), manifest_type) in
            function.arguments.iter().zip(&specification.arguments)
        {
            if !component_wire_type_matches(
                manifest_type,
                wit_type,
                interface,
                hara_values_available,
            ) {
                return Err(format!(
                    "wasm-wit/export-mismatch: {raw_name} argument WIT type {} does not match manifest :{}",
                    wit_parser::type_label(wit_type),
                    manifest_type
                ));
            }
        }
        match (function.result.as_ref(), specification.returns.as_str()) {
            (None, "void") => {}
            (Some(wit_type), manifest_type)
                if component_wire_type_matches(
                    manifest_type,
                    wit_type,
                    interface,
                    hara_values_available,
                ) => {}
            (None, manifest_type) => {
                return Err(format!(
                    "wasm-wit/export-mismatch: {raw_name} has no WIT result but manifest declares :{manifest_type}"
                ))
            }
            (Some(wit_type), "void") => {
                return Err(format!(
                    "wasm-wit/export-mismatch: {raw_name} returns WIT type {} but manifest declares :void",
                    wit_parser::type_label(wit_type)
                ))
            }
            (Some(wit_type), manifest_type) => {
                return Err(format!(
                    "wasm-wit/export-mismatch: {raw_name} result WIT type {} does not match manifest :{manifest_type}",
                    wit_parser::type_label(wit_type)
                ))
            }
        }
    }
    Ok(())
}

fn component_interface_export_name(
    package: &str,
    interface: &str,
    function: &str,
) -> Result<String, String> {
    let (coordinate, version) = package.rsplit_once('@').ok_or_else(|| {
        "wasm-wit/package-mismatch: package must include a Component version".to_owned()
    })?;
    Ok(format!("{coordinate}/{interface}@{version}#{function}"))
}

fn component_wire_type_matches(
    manifest_type: &str,
    wit_type: &wit_parser::Type,
    interface: &wit_parser::Interface,
    hara_values_available: bool,
) -> bool {
    match (manifest_type, wit_type) {
        ("value", wit_parser::Type::Atom(local_name)) => {
            hara_values_available
                && interface
                    .imported_types
                    .get(local_name)
                    .is_some_and(|imported| {
                        imported.package == "hara:values@0.1.0"
                            && imported.interface == "values"
                            && imported.type_name == "value"
                    })
        }
        ("bool", wit_parser::Type::Atom(value)) => value == "bool",
        ("i32", wit_parser::Type::Atom(value)) => {
            matches!(value.as_str(), "s8" | "u8" | "s16" | "u16" | "s32" | "u32")
        }
        ("i64", wit_parser::Type::Atom(value)) => value == "s64",
        ("f32", wit_parser::Type::Atom(value)) => value == "f32",
        ("f64", wit_parser::Type::Atom(value)) => value == "f64",
        ("char", wit_parser::Type::Atom(value)) => value == "char",
        ("string", wit_parser::Type::Atom(value)) => value == "string",
        ("bytes", wit_parser::Type::List(value)) => {
            matches!(value.as_ref(), wit_parser::Type::Atom(element) if element == "u8")
        }
        _ => false,
    }
}

fn hara_values_dependency_is_valid(dependencies: &[(String, String)]) -> Result<bool, String> {
    let Some((_, source)) = dependencies
        .iter()
        .find(|(package, _)| package == "hara:values@0.1.0")
    else {
        return Ok(false);
    };
    let document = wit_parser::parse(source)
        .map_err(|error| format!("wasm-wit/malformed hara:values dependency: {error}"))?;
    if document.package.as_deref() != Some("hara:values@0.1.0") {
        return Err("wasm-wit/hara-values-package-mismatch: dependency package is invalid".into());
    }
    let interface = document.interfaces.get("values").ok_or_else(|| {
        "wasm-wit/hara-values-interface-missing: dependency has no values interface".to_owned()
    })?;
    let Some(wit_parser::TypeDecl::Record(fields)) = interface.types.get("value") else {
        return Err("wasm-wit/hara-values-contract-mismatch: value must be a record".into());
    };
    let valid_value_record = matches!(
        fields.as_slice(),
        [
            (root, wit_parser::Type::Atom(root_type)),
            (nodes, wit_parser::Type::List(nodes_type)),
        ] if root == "root"
            && root_type == "u32"
            && nodes == "nodes"
            && matches!(nodes_type.as_ref(), wit_parser::Type::Atom(node_type) if node_type == "node")
    );
    if !valid_value_record {
        return Err(
            "wasm-wit/hara-values-contract-mismatch: value must contain root and nodes".into(),
        );
    }
    let Some(wit_parser::TypeDecl::Variant(cases)) = interface.types.get("node") else {
        return Err("wasm-wit/hara-values-contract-mismatch: node must be a variant".into());
    };
    let expected = [
        "nil",
        "boolean",
        "integer",
        "float",
        "character",
        "big-integer",
        "regex",
        "text",
        "byte-vector",
        "keyword",
        "symbol",
        "linear-list",
        "vector",
        "cons",
        "deque",
        "queue",
        "set",
        "ordered-set",
        "sorted-set",
        "dictionary",
        "ordered-map",
        "sorted-map",
        "priority-map",
        "trie",
        "tagged",
        "map-entry",
        "pointer",
        "structure",
        "exception",
    ];
    if cases
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        != expected
    {
        return Err("wasm-wit/hara-values-contract-mismatch: node cases differ".into());
    }
    let scalar_cases = [
        ("boolean", "bool"),
        ("integer", "s64"),
        ("float", "f64"),
        ("character", "char"),
        ("big-integer", "string"),
        ("regex", "string"),
        ("text", "string"),
        ("keyword", "string"),
    ];
    if !scalar_cases.iter().all(|(name, ty)| {
        variant_payload(cases, name).is_some_and(
            |payload| matches!(payload, wit_parser::Type::Atom(actual) if actual == ty),
        )
    }) || !variant_payload(cases, "byte-vector").is_some_and(is_u8_list)
        || ![
            "symbol",
            "tagged",
            "map-entry",
            "pointer",
            "structure",
            "exception",
        ]
        .iter()
        .all(|name| {
            variant_payload(cases, name)
                .is_some_and(|payload| matches!(payload, wit_parser::Type::Atom(_)))
        })
        || ![
            "linear-list",
            "vector",
            "cons",
            "deque",
            "queue",
            "set",
            "ordered-set",
            "sorted-set",
        ]
        .iter()
        .all(|name| {
            variant_payload(cases, name).is_some_and(
                |payload| matches!(payload, wit_parser::Type::Atom(actual) if actual == "sequence"),
            )
        })
        || ![
            "dictionary",
            "ordered-map",
            "sorted-map",
            "priority-map",
            "trie",
        ]
        .iter()
        .all(|name| {
            variant_payload(cases, name).is_some_and(
                |payload| matches!(payload, wit_parser::Type::Atom(actual) if actual == "mapping"),
            )
        })
    {
        return Err("wasm-wit/hara-values-contract-mismatch: node payloads differ".into());
    }
    if !record_fields_match(interface, "node-entry", &["key", "value"])
        || !record_fields_match(interface, "sequence", &["values", "metadata"])
        || !record_fields_match(interface, "mapping", &["entries", "metadata"])
        || !record_fields_match(interface, "symbol-value", &["text", "metadata"])
        || !record_fields_match(interface, "tagged-value", &["tag", "form"])
        || !record_fields_match(interface, "map-entry-value", &["key", "value", "metadata"])
        || !record_fields_match(
            interface,
            "pointer-value",
            &["context", "fields", "metadata"],
        )
        || !record_fields_match(
            interface,
            "struct-value",
            &["name", "fields", "values", "metadata"],
        )
        || !record_fields_match(
            interface,
            "exception-site",
            &["namespace", "source-resource", "line", "column"],
        )
        || !record_fields_match(interface, "exception-provenance", &["created-at", "throws"])
        || !record_fields_match(
            interface,
            "exception-value",
            &["message", "data", "cause", "provenance"],
        )
    {
        return Err("wasm-wit/hara-values-contract-mismatch: node records differ".into());
    }
    Ok(true)
}

fn variant_payload<'a>(
    cases: &'a [(String, Option<wit_parser::Type>)],
    name: &str,
) -> Option<&'a wit_parser::Type> {
    cases
        .iter()
        .find(|(candidate, _)| candidate == name)
        .and_then(|(_, payload)| payload.as_ref())
}

fn is_u8_list(ty: &wit_parser::Type) -> bool {
    matches!(ty, wit_parser::Type::List(element) if matches!(element.as_ref(), wit_parser::Type::Atom(name) if name == "u8"))
}

fn record_fields_match(interface: &wit_parser::Interface, record: &str, names: &[&str]) -> bool {
    matches!(
        interface.types.get(record),
        Some(wit_parser::TypeDecl::Record(fields))
            if fields.iter().map(|(name, _)| name.as_str()).eq(names.iter().copied())
    )
}

pub const WASM_INTERFACE_SCHEMA: &str = "hara.wasm-interface/0-alpha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WasmValueType {
    I32,
    I64,
    F32,
    F64,
    Void,
}

impl WasmValueType {
    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Void => "void",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum HaraValueType {
    I32,
    I64,
    F32,
    F64,
    Boolean,
    String,
    Bytes,
    Record(String),
    Variant(String),
    Handle(String),
    Callback(String),
    Void,
}

impl HaraValueType {
    fn direct_wasm_type(&self) -> Option<WasmValueType> {
        match self {
            Self::I32 | Self::Boolean => Some(WasmValueType::I32),
            Self::I64 => Some(WasmValueType::I64),
            Self::F32 => Some(WasmValueType::F32),
            Self::F64 => Some(WasmValueType::F64),
            Self::Void => Some(WasmValueType::Void),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Borrowed,
    Caller,
    Callee,
    Transferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lowering {
    Direct,
    PointerLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifting {
    Direct,
    PointerLength,
    PackedI64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContract {
    pub export: String,
    pub allocate: Option<String>,
    pub reallocate: Option<String>,
    pub release: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingParameter {
    pub name: String,
    pub hara_type: HaraValueType,
    pub wasm_type: WasmValueType,
    pub lowering: Option<Lowering>,
    pub ownership: Option<Ownership>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingResult {
    pub hara_type: HaraValueType,
    pub wasm_type: WasmValueType,
    pub lifting: Option<Lifting>,
    pub ownership: Option<Ownership>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorContract {
    pub convention: String,
    pub codes: BTreeMap<i64, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestPolicy {
    pub timeout_ms: Option<u64>,
    pub max_in_flight: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationPolicy {
    Cooperative,
    Abort,
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncPolicy {
    pub operation: String,
    pub request: RequestPolicy,
    pub cancellation: CancellationPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCallContract {
    pub methods: BTreeSet<String>,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackParameter {
    pub name: String,
    pub hara_type: HaraValueType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackContract {
    pub arguments: Vec<CallbackParameter>,
    pub returns: HaraValueType,
    pub reentrant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleContract {
    pub tag: String,
    pub release: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingFunction {
    pub name: String,
    pub wasm_export: String,
    pub arguments: Vec<BindingParameter>,
    pub returns: BindingResult,
    pub asynchronous: bool,
    pub operation: Option<String>,
    pub request: Option<RequestPolicy>,
    pub cancellation: Option<CancellationPolicy>,
    pub errors: Option<ErrorContract>,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmInterface {
    pub schema: String,
    pub namespace: String,
    pub module: String,
    pub memory: Option<MemoryContract>,
    pub exports: Vec<BindingFunction>,
    pub capabilities: BTreeSet<String>,
    pub host_calls: BTreeMap<String, HostCallContract>,
    pub callbacks: BTreeMap<String, CallbackContract>,
    pub handles: BTreeMap<String, HandleContract>,
    pub resources: BTreeMap<String, HandleContract>,
}

impl WasmInterface {
    pub fn parse(source: &str, origin: &str) -> Result<Self, String> {
        parser::parse_interface(source, origin)
    }

    pub fn canonical_source(&self) -> String {
        canonical::source(self)
    }

    pub fn digest(&self) -> String {
        let digest = Sha256::digest(self.canonical_source().as_bytes());
        format!("sha256:{digest:x}")
    }

    pub fn hta_required(&self) -> bool {
        !self.host_calls.is_empty()
            || !self.callbacks.is_empty()
            || !self.handles.is_empty()
            || !self.resources.is_empty()
            || self.exports.iter().any(|export| {
                export.asynchronous
                    || export.operation.is_some()
                    || export.request.is_some()
                    || export.cancellation.is_some()
            })
    }

    pub fn direct_exports(&self) -> Vec<(String, ExtensionExport)> {
        self.exports
            .iter()
            .map(|export| {
                (
                    export.wasm_export.clone(),
                    ExtensionExport {
                        arguments: export
                            .arguments
                            .iter()
                            .map(|argument| argument.wasm_type.as_keyword().to_owned())
                            .collect(),
                        returns: export.returns.wasm_type.as_keyword().to_owned(),
                        asynchronous: false,
                        raw_export: None,
                    },
                )
            })
            .collect()
    }
}
