fn hbx_descriptor() -> Value {
    vm_tool_map([
        (vm_tool_keyword("provider/id"), vm_tool_keyword("rust")),
        (
            vm_tool_keyword("provider/operations"),
            vm_tool_keywords(&["validate", "inspect", "pack", "unpack", "conform"]),
        ),
        (
            vm_tool_keyword("provider/formats"),
            vm_tool_map([(
                vm_tool_keyword("hbx"),
                vm_tool_keywords(&["validate", "inspect", "pack", "unpack", "conform"]),
            )]),
        ),
    ])
}

fn hbx_string(value: Option<&Value>, field: &str) -> Result<String, String> {
    match value {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!(
            "std.native.HBX/pack expects {field} as a non-empty String"
        )),
    }
}

fn hbx_module(value: &Value) -> Result<crate::vm::bundle::BytecodeBundleModule, String> {
    let resource = hbx_string(
        map_value(value, &vm_tool_keyword("module/resource")),
        ":module/resource",
    )?;
    let namespace_form = hbx_string(
        map_value(value, &vm_tool_keyword("module/namespace-form")),
        ":module/namespace-form",
    )?;
    let source_digest = hbx_bytes(
        map_value(value, &vm_tool_keyword("module/source-digest"))
            .ok_or("std.native.HBX/pack requires :module/source-digest")?,
        "pack",
    )?;
    let source_digest: [u8; 32] = source_digest
        .try_into()
        .map_err(|_| "std.native.HBX/pack expects a 32-byte :module/source-digest".to_owned())?;
    let dependencies = hbx_dependencies(map_value(value, &vm_tool_keyword("module/dependencies")))?;
    let eager = match map_value(value, &vm_tool_keyword("module/eager")) {
        Some(Value::Bool(value)) => *value,
        _ => return Err("std.native.HBX/pack expects :module/eager as a boolean".into()),
    };
    let artifact = hbx_bytes(
        map_value(value, &vm_tool_keyword("module/artifact"))
            .ok_or("std.native.HBX/pack requires :module/artifact")?,
        "pack",
    )?;
    Ok(crate::vm::bundle::BytecodeBundleModule {
        resource,
        namespace_form,
        source_digest,
        dependencies,
        eager,
        artifact,
    })
}

fn hbx_value(module: crate::vm::bundle::BytecodeBundleModule) -> Value {
    vm_tool_map([
        (
            vm_tool_keyword("module/resource"),
            Value::String(module.resource),
        ),
        (
            vm_tool_keyword("module/namespace-form"),
            Value::String(module.namespace_form),
        ),
        (
            vm_tool_keyword("module/source-digest"),
            Value::ByteBuffer(Rc::new(RefCell::new(module.source_digest.to_vec()))),
        ),
        (
            vm_tool_keyword("module/dependencies"),
            vm_tool_vector(module.dependencies.into_iter().map(Value::String)),
        ),
        (vm_tool_keyword("module/eager"), Value::Bool(module.eager)),
        (
            vm_tool_keyword("module/artifact"),
            Value::ByteBuffer(Rc::new(RefCell::new(module.artifact))),
        ),
    ])
}

fn hbx_bytes(value: &Value, operation: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        Value::ByteBuffer(bytes) => Ok(bytes.borrow().clone()),
        _ => Err(format!("std.native.HBX/{operation} expects Bytes")),
    }
}

fn hbx_dependencies(values: Option<&Value>) -> Result<Vec<String>, String> {
    let dependencies = |values: Vec<&Value>| {
        values
            .into_iter()
            .map(|value| match value {
                Value::String(value) => Ok(value.clone()),
                _ => Err("std.native.HBX/pack expects String dependencies".to_owned()),
            })
            .collect::<Result<Vec<_>, _>>()
    };
    match values {
        Some(Value::Vector(values)) => dependencies(values.iter().collect()),
        Some(Value::Tuple(values)) => dependencies(values.iter().collect()),
        Some(Value::Array(values)) => dependencies(values.borrow().iter().collect()),
        _ => Err("std.native.HBX/pack expects :module/dependencies as a vector".into()),
    }
}

fn hbx_decode(
    value: &Value,
    operation: &str,
) -> Result<Vec<crate::vm::bundle::BytecodeBundleModule>, String> {
    let bytes = hbx_bytes(value, operation)?;
    crate::vm::bundle::decode_bytecode_bundle(&bytes)
}

pub(crate) fn hbx_operation(operation: &str, arguments: Vec<Value>) -> Result<Value, String> {
    match operation {
        "provider" => {
            if !arguments.is_empty() {
                return Err("std.native.HBX/provider expects no arguments".into());
            }
            Ok(hbx_descriptor())
        }
        "validate" => {
            if arguments.len() != 1 {
                return Err("std.native.HBX/validate expects one package value".into());
            }
            hbx_decode(&arguments[0], "validate")?;
            Ok(Value::Bool(true))
        }
        "inspect" => {
            if arguments.len() != 1 {
                return Err("std.native.HBX/inspect expects one package value".into());
            }
            let modules = hbx_decode(&arguments[0], "inspect")?;
            Ok(vm_tool_map([
                (vm_tool_keyword("package/format"), vm_tool_keyword("hbx")),
                (vm_tool_keyword("package/version"), Value::Number(0)),
                (
                    vm_tool_keyword("modules/count"),
                    Value::Number(modules.len() as i64),
                ),
                (
                    vm_tool_keyword("modules/resources"),
                    vm_tool_vector(
                        modules
                            .into_iter()
                            .map(|module| Value::String(module.resource)),
                    ),
                ),
            ]))
        }
        "pack" => {
            if arguments.len() != 1 {
                return Err("std.native.HBX/pack expects one module vector".into());
            }
            let modules = match &arguments[0] {
                Value::Vector(values) => values
                    .iter()
                    .map(hbx_module)
                    .collect::<Result<Vec<_>, _>>()?,
                Value::Tuple(values) => values
                    .iter()
                    .map(hbx_module)
                    .collect::<Result<Vec<_>, _>>()?,
                Value::Array(values) => values
                    .borrow()
                    .iter()
                    .map(hbx_module)
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err("std.native.HBX/pack expects a vector of modules".into()),
            };
            for module in &modules {
                crate::vm::decode_program(&module.artifact).map_err(|error| {
                    format!("{}: invalid HBC0 artifact: {error}", module.resource)
                })?;
            }
            crate::vm::bundle::encode_bytecode_bundle(&modules)
                .map(|bytes| Value::ByteBuffer(Rc::new(RefCell::new(bytes))))
        }
        "unpack" => {
            if arguments.len() != 1 {
                return Err("std.native.HBX/unpack expects one package value".into());
            }
            hbx_decode(&arguments[0], "unpack")
                .map(|modules| vm_tool_vector(modules.into_iter().map(hbx_value)))
        }
        _ => Err(format!("unknown std.native.HBX method: {operation}")),
    }
}

pub(crate) fn package_tool_provider_values() -> Vec<(&'static str, Value)> {
    vec![
        (
            "provider",
            native_function("tool.package.provider/provider", 0, |_| {
                hbx_operation("provider", Vec::new())
            }),
        ),
        (
            "validate",
            native_function("tool.package.provider/validate", 1, |arguments| {
                hbx_operation("validate", arguments)
            }),
        ),
        (
            "inspect",
            native_function("tool.package.provider/inspect", 1, |arguments| {
                hbx_operation("inspect", arguments)
            }),
        ),
        (
            "pack",
            native_function("tool.package.provider/pack", 1, |arguments| {
                hbx_operation("pack", arguments)
            }),
        ),
        (
            "unpack",
            native_function("tool.package.provider/unpack", 1, |arguments| {
                hbx_operation("unpack", arguments)
            }),
        ),
    ]
}

#[cfg(test)]
mod hbx_tests {
    use super::*;

    #[test]
    fn native_hbx_provider_round_trips_an_empty_package() {
        let provider = hbx_operation("provider", Vec::new()).expect("provider");
        assert_eq!(
            map_value(&provider, &vm_tool_keyword("provider/id")).cloned(),
            Some(Value::Keyword("rust".into()))
        );

        let package = hbx_operation(
            "pack",
            vec![Value::Array(Rc::new(RefCell::new(Vec::new())))],
        )
        .expect("pack");
        assert!(matches!(package, Value::ByteBuffer(_)));
        assert_eq!(
            hbx_operation("validate", vec![package.clone()]).expect("validate"),
            Value::Bool(true)
        );
        assert!(matches!(
            hbx_operation("unpack", vec![package]).expect("unpack"),
            Value::Vector(values) if values.is_empty()
        ));
    }

    #[test]
    fn native_hbx_accepts_hara_literal_module_collections() {
        let artifact =
            crate::vm::encode_program(&crate::vm::compile_source("1").expect("compile HBC"))
                .expect("encode HBC");
        let module = vm_tool_map([
            (
                vm_tool_keyword("module/resource"),
                Value::String("hbx/fixture.hal".into()),
            ),
            (
                vm_tool_keyword("module/namespace-form"),
                Value::String("(ns hbx.fixture)".into()),
            ),
            (
                vm_tool_keyword("module/source-digest"),
                Value::ByteBuffer(Rc::new(RefCell::new(vec![0; 32]))),
            ),
            (
                vm_tool_keyword("module/dependencies"),
                Value::Tuple(Box::new(PTuple::from_values(Vec::new()).expect("tuple"))),
            ),
            (vm_tool_keyword("module/eager"), Value::Bool(false)),
            (
                vm_tool_keyword("module/artifact"),
                Value::ByteBuffer(Rc::new(RefCell::new(artifact))),
            ),
        ]);
        let package = hbx_operation(
            "pack",
            vec![Value::Tuple(Box::new(
                PTuple::from_values(vec![module]).expect("tuple"),
            ))],
        )
        .expect("pack literal module collection");
        assert!(matches!(
            hbx_operation("unpack", vec![package]).expect("unpack"),
            Value::Vector(values) if values.len() == 1
        ));
    }
}
