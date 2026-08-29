//! Target-neutral discovery for the deliberately small direct `core.v1` ABI.

use std::collections::HashSet;

use crate::extension::ExtensionExport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectWasmImportKind {
    Function,
    Table,
    Memory,
    Global,
    Tag,
}

impl DirectWasmImportKind {
    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Table => "table",
            Self::Memory => "memory",
            Self::Global => "global",
            Self::Tag => "tag",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectWasmImport {
    pub module: String,
    pub name: String,
    pub kind: DirectWasmImportKind,
    pub signature: Option<ExtensionExport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectWasmMemory {
    pub imported: bool,
    pub minimum_pages: u32,
    pub maximum_pages: Option<u32>,
    pub shared: bool,
    pub export_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectWasmFunctionExport {
    pub name: String,
    pub signature: ExtensionExport,
    pub imported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectWasmInspection {
    pub imports: Vec<DirectWasmImport>,
    pub memories: Vec<DirectWasmMemory>,
    pub exports: Vec<DirectWasmFunctionExport>,
    pub start: Option<u32>,
}

impl DirectWasmInspection {
    pub fn direct_exports(&self) -> Result<Vec<(String, ExtensionExport)>, String> {
        if !self.imports.is_empty() {
            return Err("native/module-import-denied: direct WASM must be import-free".into());
        }
        validate_direct_memories(&self.memories)?;
        self.exports
            .iter()
            .map(|export| {
                if export.imported {
                    return Err(
                        "native/module-import-denied: direct WASM must be import-free".into(),
                    );
                }
                Ok((export.name.clone(), export.signature.clone()))
            })
            .collect()
    }
}

pub fn inspect(bytes: &[u8]) -> Result<DirectWasmInspection, String> {
    if bytes.get(..8) != Some(b"\0asm\x01\0\0\0") {
        return Err("native/module-invalid: invalid WebAssembly header".into());
    }

    let mut cursor = 8;
    let mut types = Vec::new();
    let mut imported_functions = Vec::new();
    let mut functions = Vec::new();
    let mut imports = Vec::new();
    let mut memories = Vec::new();
    let mut exported_functions = Vec::new();
    let mut exported_memories = Vec::new();
    let mut export_names = HashSet::new();
    let mut start = None;

    while cursor < bytes.len() {
        let id = byte(bytes, &mut cursor)?;
        let size = unsigned(bytes, &mut cursor)? as usize;
        let end = cursor
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or("native/module-invalid: section exceeds module")?;
        let section = &bytes[cursor..end];
        cursor = end;
        let mut at = 0;

        match id {
            1 => parse_types(section, &mut at, &mut types)?,
            2 => parse_imports(
                section,
                &mut at,
                &types,
                &mut imported_functions,
                &mut imports,
                &mut memories,
            )?,
            3 => parse_functions(section, &mut at, &mut functions)?,
            5 => parse_memories(section, &mut at, &mut memories)?,
            7 => parse_exports(
                section,
                &mut at,
                &mut export_names,
                &mut exported_functions,
                &mut exported_memories,
            )?,
            8 => {
                start = Some(unsigned(section, &mut at)? as u32);
            }
            _ => {}
        }

        if matches!(id, 1 | 2 | 3 | 5 | 7 | 8) && at != section.len() {
            return Err(format!(
                "native/module-invalid: trailing bytes in section {id}"
            ));
        }
    }

    let exports = exported_functions
        .into_iter()
        .map(|(name, index)| {
            let (type_index, imported) = if index < imported_functions.len() {
                (imported_functions[index], true)
            } else {
                let defined = index - imported_functions.len();
                (
                    *functions.get(defined).ok_or_else(|| {
                        format!("native/module-invalid: bad function export {name}")
                    })?,
                    false,
                )
            };
            let signature = types
                .get(type_index)
                .cloned()
                .ok_or_else(|| format!("native/module-invalid: bad type for export {name}"))?;
            Ok(DirectWasmFunctionExport {
                name,
                signature,
                imported,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    for (name, index) in exported_memories {
        let memory = memories
            .get_mut(index)
            .ok_or_else(|| format!("native/module-invalid: bad memory export {name}"))?;
        memory.export_names.push(name);
    }
    for memory in &mut memories {
        memory.export_names.sort();
    }

    Ok(DirectWasmInspection {
        imports,
        memories,
        exports,
        start,
    })
}

pub(crate) fn exports(bytes: &[u8]) -> Result<Vec<(String, ExtensionExport)>, String> {
    inspect(bytes)?.direct_exports()
}

fn parse_types(
    bytes: &[u8],
    at: &mut usize,
    types: &mut Vec<ExtensionExport>,
) -> Result<(), String> {
    for _ in 0..unsigned(bytes, at)? {
        if byte(bytes, at)? != 0x60 {
            return Err("native/abi-type-unsupported: non-function type".into());
        }
        let arguments = value_types(bytes, at)?;
        let results = value_types(bytes, at)?;
        if results.len() > 1 {
            return Err("native/abi-type-unsupported: multiple results".into());
        }
        types.push(ExtensionExport {
            arguments: arguments.into_iter().map(str::to_owned).collect(),
            returns: results.into_iter().next().unwrap_or("void").to_owned(),
            asynchronous: false,
            raw_export: None,
        });
    }
    Ok(())
}

fn parse_imports(
    bytes: &[u8],
    at: &mut usize,
    types: &[ExtensionExport],
    imported_functions: &mut Vec<usize>,
    imports: &mut Vec<DirectWasmImport>,
    memories: &mut Vec<DirectWasmMemory>,
) -> Result<(), String> {
    for _ in 0..unsigned(bytes, at)? {
        let module = name(bytes, at)?;
        let name = name(bytes, at)?;
        let descriptor = byte(bytes, at)?;
        let (kind, signature) = match descriptor {
            0 => {
                let type_index = unsigned(bytes, at)? as usize;
                let signature = types.get(type_index).cloned().ok_or_else(|| {
                    format!("native/module-invalid: bad type for import {module}/{name}")
                })?;
                imported_functions.push(type_index);
                (DirectWasmImportKind::Function, Some(signature))
            }
            1 => {
                table_type(bytes, at)?;
                (DirectWasmImportKind::Table, None)
            }
            2 => {
                memories.push(memory_type(bytes, at, true)?);
                (DirectWasmImportKind::Memory, None)
            }
            3 => {
                global_type(bytes, at)?;
                (DirectWasmImportKind::Global, None)
            }
            4 => {
                tag_type(bytes, at, types.len())?;
                (DirectWasmImportKind::Tag, None)
            }
            value => {
                return Err(format!(
                    "native/module-invalid: unknown import kind 0x{value:02x}"
                ))
            }
        };
        imports.push(DirectWasmImport {
            module,
            name,
            kind,
            signature,
        });
    }
    Ok(())
}

fn parse_functions(bytes: &[u8], at: &mut usize, functions: &mut Vec<usize>) -> Result<(), String> {
    for _ in 0..unsigned(bytes, at)? {
        functions.push(unsigned(bytes, at)? as usize);
    }
    Ok(())
}

fn parse_memories(
    bytes: &[u8],
    at: &mut usize,
    memories: &mut Vec<DirectWasmMemory>,
) -> Result<(), String> {
    for _ in 0..unsigned(bytes, at)? {
        memories.push(memory_type(bytes, at, false)?);
    }
    Ok(())
}

fn parse_exports(
    bytes: &[u8],
    at: &mut usize,
    names: &mut HashSet<String>,
    functions: &mut Vec<(String, usize)>,
    memories: &mut Vec<(String, usize)>,
) -> Result<(), String> {
    for _ in 0..unsigned(bytes, at)? {
        let name = name(bytes, at)?;
        if !names.insert(name.clone()) {
            return Err(format!(
                "native/module-invalid: duplicate export name {name}"
            ));
        }
        let kind = byte(bytes, at)?;
        let index = unsigned(bytes, at)? as usize;
        match kind {
            0 => functions.push((name, index)),
            2 => memories.push((name, index)),
            1 | 3 | 4 => {}
            value => {
                return Err(format!(
                    "native/module-invalid: unknown export kind 0x{value:02x}"
                ))
            }
        }
    }
    Ok(())
}

fn memory_type(bytes: &[u8], at: &mut usize, imported: bool) -> Result<DirectWasmMemory, String> {
    let (minimum_pages, maximum_pages, shared) = limits(bytes, at, "memory")?;
    Ok(DirectWasmMemory {
        imported,
        minimum_pages,
        maximum_pages,
        shared,
        export_names: Vec::new(),
    })
}

fn table_type(bytes: &[u8], at: &mut usize) -> Result<(), String> {
    match byte(bytes, at)? {
        0x70 | 0x6f => {}
        value => {
            return Err(format!(
                "native/abi-type-unsupported: table reference type 0x{value:02x}"
            ))
        }
    }
    limits(bytes, at, "table")?;
    Ok(())
}

fn global_type(bytes: &[u8], at: &mut usize) -> Result<(), String> {
    match byte(bytes, at)? {
        0x7f | 0x7e | 0x7d | 0x7c | 0x7b | 0x70 | 0x6f => {}
        value => {
            return Err(format!(
                "native/abi-type-unsupported: global value type 0x{value:02x}"
            ))
        }
    }
    match byte(bytes, at)? {
        0 | 1 => Ok(()),
        value => Err(format!(
            "native/module-invalid: global mutability 0x{value:02x}"
        )),
    }
}

fn tag_type(bytes: &[u8], at: &mut usize, type_count: usize) -> Result<(), String> {
    if byte(bytes, at)? != 0 {
        return Err("native/module-invalid: unsupported tag attribute".into());
    }
    let type_index = unsigned(bytes, at)? as usize;
    if type_index >= type_count {
        return Err("native/module-invalid: bad tag type".into());
    }
    Ok(())
}

fn limits(bytes: &[u8], at: &mut usize, subject: &str) -> Result<(u32, Option<u32>, bool), String> {
    let flags = unsigned(bytes, at)?;
    if flags & !0x03 != 0 {
        return Err(format!("native/abi-type-unsupported: {subject}64 limits"));
    }
    let minimum = unsigned(bytes, at)?;
    let maximum = if flags & 1 != 0 {
        Some(unsigned(bytes, at)?)
    } else {
        None
    };
    let shared = flags & 2 != 0;
    if shared && maximum.is_none() {
        return Err(format!(
            "native/module-invalid: shared {subject} requires a maximum"
        ));
    }
    if maximum.is_some_and(|value| value < minimum) {
        return Err(format!(
            "native/module-invalid: {subject} maximum is below minimum"
        ));
    }
    Ok((minimum, maximum, shared))
}

fn validate_direct_memories(memories: &[DirectWasmMemory]) -> Result<(), String> {
    if memories.len() > 1 {
        return Err("native/resource-limit: at most one memory is allowed".into());
    }
    for memory in memories {
        if memory.shared {
            return Err("native/resource-limit: shared memories are unsupported".into());
        }
        if memory.minimum_pages > 1024
            || memory.maximum_pages.is_none()
            || memory.maximum_pages.is_some_and(|value| value > 1024)
        {
            return Err("native/resource-limit: memory must be bounded to 64 MiB".into());
        }
    }
    Ok(())
}

fn value_types(bytes: &[u8], at: &mut usize) -> Result<Vec<&'static str>, String> {
    (0..unsigned(bytes, at)?)
        .map(|_| match byte(bytes, at)? {
            0x7f => Ok("i32"),
            0x7e => Ok("i64"),
            0x7d => Ok("f32"),
            0x7c => Ok("f64"),
            value => Err(format!("native/abi-type-unsupported: 0x{value:02x}")),
        })
        .collect()
}

fn name(bytes: &[u8], at: &mut usize) -> Result<String, String> {
    let size = unsigned(bytes, at)? as usize;
    let end = at
        .checked_add(size)
        .filter(|end| *end <= bytes.len())
        .ok_or("native/module-invalid: name exceeds section")?;
    let value = std::str::from_utf8(&bytes[*at..end])
        .map_err(|_| "native/module-invalid: name is not UTF-8")?
        .to_owned();
    *at = end;
    Ok(value)
}

fn byte(bytes: &[u8], at: &mut usize) -> Result<u8, String> {
    let value = *bytes
        .get(*at)
        .ok_or("native/module-invalid: unexpected end of module")?;
    *at += 1;
    Ok(value)
}

fn unsigned(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = byte(bytes, at)?;
        if shift == 28 && byte & 0xf0 != 0 {
            return Err("native/module-invalid: integer overflow".into());
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("native/module-invalid: invalid integer".into())
}

#[cfg(test)]
mod tests {
    use super::{exports, inspect, DirectWasmImportKind};

    const ADD: &[u8] = b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
    const IMPORT: &[u8] =
        b"\0asm\x01\0\0\0\x01\x05\x01\x60\x01\x7f\0\x02\x0b\x01\x03env\x03log\0\0";

    #[test]
    fn discovers_scalar_exports_without_a_host_engine() {
        let found = exports(ADD).unwrap();
        assert_eq!(found[0].0, "add");
        assert_eq!(found[0].1.arguments, ["i64", "i64"]);
        assert_eq!(found[0].1.returns, "i64");
    }

    #[test]
    fn reports_imports_before_direct_policy_rejects_them() {
        let found = inspect(IMPORT).unwrap();
        assert_eq!(found.imports.len(), 1);
        assert_eq!(found.imports[0].module, "env");
        assert_eq!(found.imports[0].name, "log");
        assert_eq!(found.imports[0].kind, DirectWasmImportKind::Function);
        assert_eq!(
            found.imports[0].signature.as_ref().unwrap().arguments,
            ["i32"]
        );
        assert!(found.direct_exports().unwrap_err().contains("import-free"));
    }

    #[test]
    fn reports_bounded_exported_memory() {
        let memory = b"\0asm\x01\0\0\0\x05\x04\x01\x01\x01\x02\x07\x0a\x01\x06memory\x02\0";
        let found = inspect(memory).unwrap();
        assert_eq!(found.memories.len(), 1);
        assert_eq!(found.memories[0].minimum_pages, 1);
        assert_eq!(found.memories[0].maximum_pages, Some(2));
        assert_eq!(found.memories[0].export_names, ["memory"]);
        assert!(found.direct_exports().is_ok());
    }
}
