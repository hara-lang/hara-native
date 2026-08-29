use std::collections::{BTreeMap, BTreeSet};

use crate::direct_wasm;
use crate::extension::ExtensionExport;
use crate::kernel::Form;

use super::{DirectWasmInspection, WasmInterface, WASM_INTERFACE_SCHEMA};

pub const DIRECT_WASM_INSPECTION_SCHEMA: &str = "hara.wasm-inspection/0-alpha";

pub fn inspect_direct(bytes: &[u8]) -> Result<DirectWasmInspection, String> {
    direct_wasm::inspect(bytes)
}

pub fn direct_inspection_source(inspection: &DirectWasmInspection) -> String {
    Form::Map(vec![
        (
            keyword_form("schema"),
            string_form(DIRECT_WASM_INSPECTION_SCHEMA),
        ),
        (
            keyword_form("imports"),
            Form::Vector(
                inspection
                    .imports
                    .iter()
                    .map(|import| {
                        let mut fields = vec![
                            (keyword_form("module"), string_form(&import.module)),
                            (keyword_form("name"), string_form(&import.name)),
                            (keyword_form("kind"), keyword_form(import.kind.as_keyword())),
                        ];
                        if let Some(signature) = import.signature.as_ref() {
                            fields.extend(signature_forms(signature));
                        }
                        Form::Map(fields)
                    })
                    .collect(),
            ),
        ),
        (
            keyword_form("memories"),
            Form::Vector(
                inspection
                    .memories
                    .iter()
                    .map(|memory| {
                        Form::Map(vec![
                            (keyword_form("imported"), Form::Bool(memory.imported)),
                            (
                                keyword_form("minimum-pages"),
                                Form::Number(i64::from(memory.minimum_pages)),
                            ),
                            (
                                keyword_form("maximum-pages"),
                                memory
                                    .maximum_pages
                                    .map(|value| Form::Number(i64::from(value)))
                                    .unwrap_or(Form::Nil),
                            ),
                            (keyword_form("shared"), Form::Bool(memory.shared)),
                            (
                                keyword_form("exports"),
                                Form::Vector(
                                    memory
                                        .export_names
                                        .iter()
                                        .map(|name| string_form(name))
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            keyword_form("start"),
            inspection
                .start
                .map(|value| Form::Number(i64::from(value)))
                .unwrap_or(Form::Nil),
        ),
        (
            keyword_form("exports"),
            Form::Vector(
                inspection
                    .exports
                    .iter()
                    .map(|export| {
                        let mut fields = vec![(keyword_form("name"), string_form(&export.name))];
                        fields.extend(signature_forms(&export.signature));
                        fields.push((keyword_form("imported"), Form::Bool(export.imported)));
                        Form::Map(fields)
                    })
                    .collect(),
            ),
        ),
    ])
    .to_string()
}

pub fn direct_interface_skeleton(
    namespace: &str,
    module: &str,
    inspection: &DirectWasmInspection,
) -> Result<String, String> {
    if !valid_namespace(namespace) {
        return Err("wasm-binding/malformed: namespace must be a qualified lower-case name".into());
    }
    if !valid_module_path(module) {
        return Err(
            "wasm-binding/malformed: module must be a safe relative .wasm package path".into(),
        );
    }
    if inspection.exports.is_empty() {
        return Err("wasm-binding/export-missing: module has no function exports".into());
    }

    let mut names = BTreeSet::new();
    let exports = inspection
        .exports
        .iter()
        .enumerate()
        .map(|(index, export)| {
            let public_name = unique_binding_name(&export.name, index, &mut names);
            let arguments = export
                .signature
                .arguments
                .iter()
                .enumerate()
                .map(|(argument, wasm_type)| {
                    Form::Map(vec![
                        (
                            keyword_form("name"),
                            symbol_form(&format!("arg-{argument}")),
                        ),
                        (keyword_form("hara/type"), keyword_form("unresolved")),
                        (keyword_form("wasm/type"), keyword_form(wasm_type)),
                    ])
                })
                .collect();
            let hara_result = if export.signature.returns == "void" {
                "void"
            } else {
                "unresolved"
            };
            (
                symbol_form(&public_name),
                Form::Map(vec![
                    (keyword_form("wasm/export"), string_form(&export.name)),
                    (keyword_form("arguments"), Form::Vector(arguments)),
                    (
                        keyword_form("returns"),
                        Form::Map(vec![
                            (keyword_form("hara/type"), keyword_form(hara_result)),
                            (
                                keyword_form("wasm/type"),
                                keyword_form(&export.signature.returns),
                            ),
                        ]),
                    ),
                ]),
            )
        })
        .collect();

    Ok(Form::List(vec![
        symbol_form("wasm/interface"),
        Form::Map(vec![
            (keyword_form("schema"), string_form(WASM_INTERFACE_SCHEMA)),
            (keyword_form("namespace"), symbol_form(namespace)),
            (keyword_form("module"), string_form(module)),
            (keyword_form("exports"), Form::Map(exports)),
        ]),
    ])
    .to_string())
}

impl WasmInterface {
    pub fn verify_direct(&self, inspection: &DirectWasmInspection) -> Result<(), String> {
        if self.memory.is_some() {
            return Err(
                "wasm-binding/feature-unsupported: :memory requires the memory binding tranche"
                    .into(),
            );
        }
        if !self.capabilities.is_empty()
            || self
                .exports
                .iter()
                .any(|export| !export.capabilities.is_empty())
        {
            return Err(
                "wasm-binding/capability-denied: direct core.v1 bindings cannot require capabilities"
                    .into(),
            );
        }
        if self.exports.iter().any(|export| export.errors.is_some()) {
            return Err(
                "wasm-binding/feature-unsupported: error mappings require a richer binding target"
                    .into(),
            );
        }

        let discovered = inspection
            .direct_exports()
            .map_err(|error| format!("wasm-binding/module-incompatible: {error}"))?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let mut raw_names = BTreeSet::new();

        for export in &self.exports {
            if !raw_names.insert(export.wasm_export.as_str()) {
                return Err(format!(
                    "wasm-binding/export-ambiguous: multiple Hara exports map to {}",
                    export.wasm_export
                ));
            }
            let expected = ExtensionExport {
                arguments: export
                    .arguments
                    .iter()
                    .map(|argument| argument.wasm_type.as_keyword().to_owned())
                    .collect(),
                returns: export.returns.wasm_type.as_keyword().to_owned(),
                asynchronous: false,
                raw_export: None,
            };
            let found = discovered.get(&export.wasm_export).ok_or_else(|| {
                format!(
                    "wasm-binding/export-missing: {} maps to absent Wasm export {}",
                    export.name, export.wasm_export
                )
            })?;
            if found != &expected {
                return Err(format!(
                    "wasm-binding/signature-mismatch: {} -> {} expected {:?}, found {:?}",
                    export.name, export.wasm_export, expected, found
                ));
            }
        }
        Ok(())
    }
}

fn signature_forms(signature: &ExtensionExport) -> Vec<(Form, Form)> {
    vec![
        (
            keyword_form("arguments"),
            Form::Vector(
                signature
                    .arguments
                    .iter()
                    .map(|argument| keyword_form(argument))
                    .collect(),
            ),
        ),
        (keyword_form("returns"), keyword_form(&signature.returns)),
    ]
}

fn unique_binding_name(raw: &str, index: usize, used: &mut BTreeSet<String>) -> String {
    let base = sanitize_binding_name(raw, index);
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded suffix search must find a unique binding name")
}

fn sanitize_binding_name(raw: &str, index: usize) -> String {
    let mut output = String::new();
    let mut separated = false;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            if separated && !output.is_empty() {
                output.push('-');
            }
            output.push(character.to_ascii_lowercase());
            separated = false;
        } else {
            separated = true;
        }
    }
    if output.is_empty() {
        format!("function-{index}")
    } else {
        output
    }
}

fn valid_namespace(value: &str) -> bool {
    value.contains('.') && value.split('.').all(valid_component)
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn valid_module_path(value: &str) -> bool {
    value.ends_with(".wasm")
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.contains(':')
        && !value.bytes().any(|byte| byte == 0)
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn keyword_form(value: &str) -> Form {
    Form::Keyword(value.to_owned())
}

fn symbol_form(value: &str) -> Form {
    Form::Symbol(value.to_owned())
}

fn string_form(value: &str) -> Form {
    Form::String(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        direct_inspection_source, direct_interface_skeleton, inspect_direct, WasmInterface,
    };

    const ADD: &[u8] = b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
    const IMPORT: &[u8] =
        b"\0asm\x01\0\0\0\x01\x05\x01\x60\x01\x7f\0\x02\x0b\x01\x03env\x03log\0\0";

    const INTERFACE: &str = r#"
      (wasm/interface
       {:schema "hara.wasm-interface/0-alpha"
        :namespace math.scalar
        :module "math.wasm"
        :exports
        {sum {:wasm/export "add"
              :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                          {:name right :hara/type :i64 :wasm/type :i64}]
              :returns {:hara/type :i64 :wasm/type :i64}}}})"#;

    #[test]
    fn emits_an_explicitly_unresolved_interface_skeleton() {
        let inspection = inspect_direct(ADD).unwrap();
        let source = direct_interface_skeleton("generated.math", "math.wasm", &inspection).unwrap();
        assert!(source.contains(":hara/type :unresolved"));
        assert!(source.contains(":wasm/export \"add\""));
        assert!(source.contains(":wasm/type :i64"));
        let error = WasmInterface::parse(&source, "skeleton").unwrap_err();
        assert!(error.contains("unsupported Hara type :unresolved"));
    }

    #[test]
    fn verifies_hara_names_against_exact_raw_exports() {
        let interface = WasmInterface::parse(INTERFACE, "fixture").unwrap();
        let inspection = inspect_direct(ADD).unwrap();
        interface.verify_direct(&inspection).unwrap();

        let mut drifted = inspection.clone();
        drifted.exports[0].signature.returns = "i32".into();
        assert!(interface
            .verify_direct(&drifted)
            .unwrap_err()
            .starts_with("wasm-binding/signature-mismatch"));
    }

    #[test]
    fn renders_imports_without_claiming_they_are_bindable() {
        let inspection = inspect_direct(IMPORT).unwrap();
        let report = direct_inspection_source(&inspection);
        assert!(report.contains(":kind :function"));
        assert!(report.contains(":module \"env\""));
        assert!(report.contains(":arguments [:i32]"));
        let interface = WasmInterface::parse(INTERFACE, "fixture").unwrap();
        assert!(interface
            .verify_direct(&inspection)
            .unwrap_err()
            .contains("import-free"));
    }
}
