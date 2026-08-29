use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::extension::ExtensionExport;
use crate::kernel::Form;

use super::{
    BindingParameter, BindingResult, DirectWasmInspection, HaraValueType, Lifting, Lowering,
    MemoryContract, Ownership, WasmInterface, WasmValueType,
};

pub const MEMORY_BINDING_SCHEMA: &str = "hara.wasm-memory-binding/0-alpha";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBindingPlan {
    pub schema: String,
    pub namespace: String,
    pub module: String,
    pub memory: MemoryContract,
    pub functions: Vec<MemoryFunctionPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFunctionPlan {
    pub name: String,
    pub wasm_export: String,
    pub arguments: Vec<MemoryArgumentPlan>,
    pub returns: MemoryResultPlan,
    pub raw_arguments: Vec<WasmValueType>,
    pub raw_returns: WasmValueType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryArgumentPlan {
    pub name: String,
    pub hara_type: HaraValueType,
    pub lowering: Option<Lowering>,
    pub ownership: Option<Ownership>,
    pub raw_types: Vec<WasmValueType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryResultPlan {
    pub hara_type: HaraValueType,
    pub lifting: Option<Lifting>,
    pub ownership: Option<Ownership>,
    pub raw_type: WasmValueType,
}

impl WasmInterface {
    pub fn memory_plan(&self) -> Result<MemoryBindingPlan, String> {
        MemoryBindingPlan::compile(self)
    }
}

impl MemoryBindingPlan {
    pub fn compile(interface: &WasmInterface) -> Result<Self, String> {
        if interface.capabilities.is_empty()
            && interface
                .exports
                .iter()
                .all(|export| export.capabilities.is_empty())
        {
            // Memory bindings are pure module calls in this tranche.
        } else {
            return Err(
                "wasm-binding/capability-denied: memory.v1 cannot require host capabilities".into(),
            );
        }
        if interface
            .exports
            .iter()
            .any(|export| export.asynchronous || export.errors.is_some())
        {
            return Err(
                "wasm-binding/feature-unsupported: async and error mappings require HTA".into(),
            );
        }

        let memory = interface.memory.clone().ok_or_else(|| {
            "wasm-binding/memory-missing: non-scalar bindings require :memory".to_owned()
        })?;
        if memory.reallocate.is_some() {
            return Err(
                "wasm-binding/feature-unsupported: :reallocate is reserved for a later memory.v1 revision"
                    .into(),
            );
        }
        let mut raw_names = BTreeSet::new();
        let mut requires_allocate = false;
        let mut requires_release = false;
        let mut uses_memory = false;
        let mut functions = Vec::with_capacity(interface.exports.len());

        for export in &interface.exports {
            if !raw_names.insert(export.wasm_export.as_str()) {
                return Err(format!(
                    "wasm-binding/export-ambiguous: multiple Hara exports map to {}",
                    export.wasm_export
                ));
            }
            let mut raw_arguments = Vec::new();
            let mut arguments = Vec::with_capacity(export.arguments.len());
            for argument in &export.arguments {
                let compiled = compile_argument(argument, &export.name)?;
                uses_memory |= compiled.lowering.is_some();
                if compiled.lowering == Some(Lowering::PointerLength) {
                    requires_allocate = true;
                }
                raw_arguments.extend(compiled.raw_types.iter().copied());
                arguments.push(compiled);
            }
            let returns = compile_result(&export.returns, &export.name)?;
            uses_memory |= returns.lifting.is_some();
            if returns.ownership == Some(Ownership::Caller) {
                requires_release = true;
            }
            functions.push(MemoryFunctionPlan {
                name: export.name.clone(),
                wasm_export: export.wasm_export.clone(),
                arguments,
                returns: returns.clone(),
                raw_arguments,
                raw_returns: returns.raw_type,
            });
        }

        if !uses_memory {
            return Err(
                "wasm-binding/memory-unused: memory.v1 requires at least one lowered or lifted value"
                    .into(),
            );
        }
        if requires_allocate && memory.allocate.is_none() {
            return Err(
                "wasm-binding/allocator-missing: pointer/length inputs require :memory :allocate"
                    .into(),
            );
        }
        if requires_release && memory.release.is_none() {
            return Err(
                "wasm-binding/release-missing: caller-owned results require :memory :release"
                    .into(),
            );
        }

        Ok(Self {
            schema: MEMORY_BINDING_SCHEMA.into(),
            namespace: interface.namespace.clone(),
            module: interface.module.clone(),
            memory,
            functions,
        })
    }

    pub fn canonical_source(&self) -> String {
        plan_form(self).to_string()
    }

    pub fn digest(&self) -> String {
        let digest = Sha256::digest(self.canonical_source().as_bytes());
        format!("sha256:{digest:x}")
    }

    pub fn verify(&self, inspection: &DirectWasmInspection) -> Result<(), String> {
        if inspection.start.is_some() {
            return Err(
                "wasm-binding/start-denied: memory.v1 modules must not declare a start function"
                    .into(),
            );
        }
        let discovered = inspection
            .direct_exports()
            .map_err(|error| format!("wasm-binding/module-incompatible: {error}"))?
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        let memory_exported = inspection.memories.iter().any(|memory| {
            memory
                .export_names
                .iter()
                .any(|name| name == &self.memory.export)
        });
        if !memory_exported {
            return Err(format!(
                "wasm-binding/memory-missing: module does not export {}",
                self.memory.export
            ));
        }

        for function in &self.functions {
            verify_signature(
                &discovered,
                &function.wasm_export,
                &ExtensionExport {
                    arguments: function
                        .raw_arguments
                        .iter()
                        .map(|value| value.as_keyword().to_owned())
                        .collect(),
                    returns: function.raw_returns.as_keyword().to_owned(),
                    asynchronous: false,
                    raw_export: None,
                },
                &format!("{} -> {}", function.name, function.wasm_export),
            )?;
        }
        if let Some(name) = self.memory.allocate.as_deref() {
            verify_signature(
                &discovered,
                name,
                &signature(&[WasmValueType::I32], WasmValueType::I32),
                "memory allocator",
            )?;
        }
        if let Some(name) = self.memory.reallocate.as_deref() {
            verify_signature(
                &discovered,
                name,
                &signature(
                    &[WasmValueType::I32, WasmValueType::I32],
                    WasmValueType::I32,
                ),
                "memory reallocator",
            )?;
        }
        if let Some(name) = self.memory.release.as_deref() {
            verify_signature(
                &discovered,
                name,
                &signature(&[WasmValueType::I32], WasmValueType::Void),
                "memory release",
            )?;
        }
        Ok(())
    }
}

fn compile_argument(
    argument: &BindingParameter,
    export: &str,
) -> Result<MemoryArgumentPlan, String> {
    if let Some(expected) = argument.hara_type.direct_wasm_type() {
        if expected != argument.wasm_type
            || argument.lowering.is_some()
            || argument.ownership.is_some()
        {
            return Err(format!(
                "wasm-binding/signature-mismatch: scalar argument {} in {export} must map directly to :{}",
                argument.name,
                expected.as_keyword()
            ));
        }
        return Ok(MemoryArgumentPlan {
            name: argument.name.clone(),
            hara_type: argument.hara_type.clone(),
            lowering: None,
            ownership: None,
            raw_types: vec![expected],
        });
    }

    let memory_value = matches!(
        argument.hara_type,
        HaraValueType::String | HaraValueType::Bytes
    );
    if !memory_value
        || argument.lowering != Some(Lowering::PointerLength)
        || argument.wasm_type != WasmValueType::I32
    {
        return Err(format!(
            "wasm-binding/feature-unsupported: argument {} in {export} must be :string or :bytes lowered as [:pointer :length] from :i32",
            argument.name
        ));
    }
    match argument.ownership {
        Some(Ownership::Borrowed | Ownership::Transferred) => {}
        _ => {
            return Err(format!(
                "wasm-binding/ownership-invalid: argument {} in {export} must be :borrowed or :transferred",
                argument.name
            ))
        }
    }
    Ok(MemoryArgumentPlan {
        name: argument.name.clone(),
        hara_type: argument.hara_type.clone(),
        lowering: argument.lowering,
        ownership: argument.ownership,
        raw_types: vec![WasmValueType::I32, WasmValueType::I32],
    })
}

fn compile_result(result: &BindingResult, export: &str) -> Result<MemoryResultPlan, String> {
    if let Some(expected) = result.hara_type.direct_wasm_type() {
        if expected != result.wasm_type || result.lifting.is_some() || result.ownership.is_some() {
            return Err(format!(
                "wasm-binding/signature-mismatch: scalar result in {export} must map directly to :{}",
                expected.as_keyword()
            ));
        }
        return Ok(MemoryResultPlan {
            hara_type: result.hara_type.clone(),
            lifting: None,
            ownership: None,
            raw_type: expected,
        });
    }

    let memory_value = matches!(
        result.hara_type,
        HaraValueType::String | HaraValueType::Bytes
    );
    if !memory_value
        || result.lifting != Some(Lifting::PackedI64)
        || result.wasm_type != WasmValueType::I64
    {
        return Err(format!(
            "wasm-binding/feature-unsupported: result in {export} must be :string or :bytes lifted from :packed-i64"
        ));
    }
    match result.ownership {
        Some(Ownership::Caller | Ownership::Callee) => {}
        _ => {
            return Err(format!(
                "wasm-binding/ownership-invalid: result in {export} must be :caller or :callee"
            ))
        }
    }
    Ok(MemoryResultPlan {
        hara_type: result.hara_type.clone(),
        lifting: result.lifting,
        ownership: result.ownership,
        raw_type: WasmValueType::I64,
    })
}

fn verify_signature(
    discovered: &BTreeMap<String, ExtensionExport>,
    raw_name: &str,
    expected: &ExtensionExport,
    label: &str,
) -> Result<(), String> {
    let found = discovered
        .get(raw_name)
        .ok_or_else(|| format!("wasm-binding/export-missing: {label} requires {raw_name}"))?;
    if found != expected {
        return Err(format!(
            "wasm-binding/signature-mismatch: {label} expected {expected:?}, found {found:?}"
        ));
    }
    Ok(())
}

fn signature(arguments: &[WasmValueType], returns: WasmValueType) -> ExtensionExport {
    ExtensionExport {
        arguments: arguments
            .iter()
            .map(|argument| argument.as_keyword().to_owned())
            .collect(),
        returns: returns.as_keyword().to_owned(),
        asynchronous: false,
        raw_export: None,
    }
}

fn plan_form(plan: &MemoryBindingPlan) -> Form {
    Form::Map(vec![
        (keyword("schema"), string(&plan.schema)),
        (keyword("namespace"), symbol(&plan.namespace)),
        (keyword("module"), string(&plan.module)),
        (keyword("target"), keyword("memory.v1")),
        (keyword("memory"), memory_form(&plan.memory)),
        (
            keyword("functions"),
            Form::Vector(plan.functions.iter().map(function_form).collect()),
        ),
    ])
}

fn memory_form(memory: &MemoryContract) -> Form {
    let mut fields = vec![(keyword("export"), string(&memory.export))];
    push_string(&mut fields, "allocate", memory.allocate.as_deref());
    push_string(&mut fields, "reallocate", memory.reallocate.as_deref());
    push_string(&mut fields, "release", memory.release.as_deref());
    Form::Map(fields)
}

fn function_form(function: &MemoryFunctionPlan) -> Form {
    Form::Map(vec![
        (keyword("hara/name"), symbol(&function.name)),
        (keyword("wasm/export"), string(&function.wasm_export)),
        (
            keyword("arguments"),
            Form::Vector(function.arguments.iter().map(argument_form).collect()),
        ),
        (keyword("returns"), result_form(&function.returns)),
        (
            keyword("wasm/arguments"),
            Form::Vector(
                function
                    .raw_arguments
                    .iter()
                    .map(|value| keyword(value.as_keyword()))
                    .collect(),
            ),
        ),
        (
            keyword("wasm/returns"),
            keyword(function.raw_returns.as_keyword()),
        ),
    ])
}

fn argument_form(argument: &MemoryArgumentPlan) -> Form {
    let mut fields = vec![
        (keyword("name"), symbol(&argument.name)),
        (keyword("hara/type"), hara_type_form(&argument.hara_type)),
        (
            keyword("wasm/types"),
            Form::Vector(
                argument
                    .raw_types
                    .iter()
                    .map(|value| keyword(value.as_keyword()))
                    .collect(),
            ),
        ),
    ];
    if let Some(lowering) = argument.lowering {
        fields.push((keyword("lower"), lowering_form(lowering)));
    }
    if let Some(ownership) = argument.ownership {
        fields.push((keyword("ownership"), keyword(ownership_name(ownership))));
    }
    Form::Map(fields)
}

fn result_form(result: &MemoryResultPlan) -> Form {
    let mut fields = vec![
        (keyword("hara/type"), hara_type_form(&result.hara_type)),
        (keyword("wasm/type"), keyword(result.raw_type.as_keyword())),
    ];
    if let Some(lifting) = result.lifting {
        fields.push((keyword("lift"), lifting_form(lifting)));
    }
    if let Some(ownership) = result.ownership {
        fields.push((keyword("ownership"), keyword(ownership_name(ownership))));
    }
    Form::Map(fields)
}

fn hara_type_form(value: &HaraValueType) -> Form {
    match value {
        HaraValueType::I32 => keyword("i32"),
        HaraValueType::I64 => keyword("i64"),
        HaraValueType::F32 => keyword("f32"),
        HaraValueType::F64 => keyword("f64"),
        HaraValueType::Boolean => keyword("boolean"),
        HaraValueType::String => keyword("string"),
        HaraValueType::Bytes => keyword("bytes"),
        HaraValueType::Record(name) => named_type("record", name),
        HaraValueType::Variant(name) => named_type("variant", name),
        HaraValueType::Handle(name) => named_type("handle", name),
        HaraValueType::Callback(name) => named_type("callback", name),
        HaraValueType::Void => keyword("void"),
    }
}

fn lowering_form(value: Lowering) -> Form {
    match value {
        Lowering::Direct => keyword("direct"),
        Lowering::PointerLength => Form::Vector(vec![keyword("pointer"), keyword("length")]),
    }
}

fn lifting_form(value: Lifting) -> Form {
    match value {
        Lifting::Direct => keyword("direct"),
        Lifting::PointerLength => Form::Vector(vec![keyword("pointer"), keyword("length")]),
        Lifting::PackedI64 => keyword("packed-i64"),
    }
}

fn ownership_name(value: Ownership) -> &'static str {
    match value {
        Ownership::Borrowed => "borrowed",
        Ownership::Caller => "caller",
        Ownership::Callee => "callee",
        Ownership::Transferred => "transferred",
    }
}

fn named_type(kind: &str, name: &str) -> Form {
    Form::Vector(vec![keyword(kind), symbol(name)])
}

fn push_string(fields: &mut Vec<(Form, Form)>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        fields.push((keyword(name), string(value)));
    }
}

fn keyword(value: &str) -> Form {
    Form::Keyword(value.into())
}

fn symbol(value: &str) -> Form {
    Form::Symbol(value.into())
}

fn string(value: &str) -> Form {
    Form::String(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direct_wasm::{DirectWasmFunctionExport, DirectWasmMemory};

    const INTERFACE: &str = r#"
      (wasm/interface
       {:schema "hara.wasm-interface/0-alpha"
        :namespace codec.echo
        :module "echo.wasm"
        :memory {:export "memory" :allocate "alloc" :release "free"}
        :exports
        {echo {:wasm/export "echo_bytes"
               :arguments [{:name input
                            :hara/type :bytes
                            :wasm/type :i32
                            :lower [:pointer :length]
                            :ownership :borrowed}]
               :returns {:hara/type :bytes
                         :wasm/type :i64
                         :lift :packed-i64
                         :ownership :caller}}}})"#;

    #[test]
    fn compiles_a_closed_memory_plan() {
        let interface = WasmInterface::parse(INTERFACE, "fixture").unwrap();
        let plan = interface.memory_plan().unwrap();
        assert_eq!(
            plan.functions[0].raw_arguments,
            [WasmValueType::I32, WasmValueType::I32]
        );
        assert_eq!(plan.functions[0].raw_returns, WasmValueType::I64);
        assert!(plan.canonical_source().contains(":target :memory.v1"));
        assert!(plan.canonical_source().contains(":ownership :caller"));
        assert!(plan.digest().starts_with("sha256:"));
        plan.verify(&inspection()).unwrap();
    }

    #[test]
    fn rejects_signature_drift_and_missing_lifecycle_helpers() {
        let interface = WasmInterface::parse(INTERFACE, "fixture").unwrap();
        let plan = interface.memory_plan().unwrap();
        let mut drifted = inspection();
        drifted.exports[0].signature.returns = "i32".into();
        assert!(plan
            .verify(&drifted)
            .unwrap_err()
            .starts_with("wasm-binding/signature-mismatch"));

        let missing_release = INTERFACE.replace(" :release \"free\"", "");
        assert!(WasmInterface::parse(&missing_release, "fixture")
            .unwrap()
            .memory_plan()
            .unwrap_err()
            .starts_with("wasm-binding/release-missing"));

        let mut started = inspection();
        started.start = Some(0);
        assert!(plan
            .verify(&started)
            .unwrap_err()
            .starts_with("wasm-binding/start-denied"));
    }

    #[test]
    fn rejects_directionally_invalid_ownership() {
        let invalid_input = INTERFACE.replace(":ownership :borrowed", ":ownership :caller");
        assert!(WasmInterface::parse(&invalid_input, "fixture")
            .unwrap()
            .memory_plan()
            .unwrap_err()
            .starts_with("wasm-binding/ownership-invalid"));

        let invalid_result = INTERFACE.replace(":ownership :caller", ":ownership :borrowed");
        assert!(WasmInterface::parse(&invalid_result, "fixture")
            .unwrap()
            .memory_plan()
            .unwrap_err()
            .starts_with("wasm-binding/ownership-invalid"));
    }

    #[test]
    fn rejects_reallocate_until_a_revision_defines_its_lifecycle() {
        let with_reallocate = INTERFACE.replace(
            ":allocate \"alloc\"",
            ":allocate \"alloc\" :reallocate \"realloc\"",
        );
        assert!(WasmInterface::parse(&with_reallocate, "fixture")
            .unwrap()
            .memory_plan()
            .unwrap_err()
            .starts_with("wasm-binding/feature-unsupported"));
    }

    fn inspection() -> DirectWasmInspection {
        DirectWasmInspection {
            imports: Vec::new(),
            memories: vec![DirectWasmMemory {
                imported: false,
                minimum_pages: 1,
                maximum_pages: Some(16),
                shared: false,
                export_names: vec!["memory".into()],
            }],
            exports: vec![
                function("echo_bytes", &["i32", "i32"], "i64"),
                function("alloc", &["i32"], "i32"),
                function("free", &["i32"], "void"),
            ],
            start: None,
        }
    }

    fn function(name: &str, arguments: &[&str], returns: &str) -> DirectWasmFunctionExport {
        DirectWasmFunctionExport {
            name: name.into(),
            imported: false,
            signature: ExtensionExport {
                arguments: arguments.iter().map(|value| (*value).into()).collect(),
                returns: returns.into(),
                asynchronous: false,
                raw_export: None,
            },
        }
    }
}
