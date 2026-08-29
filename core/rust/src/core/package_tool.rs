fn package_tool_descriptor() -> Value {
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

fn package_tool_string(value: Option<&Value>, field: &str) -> Result<String, String> {
    match value {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!(
            "tool.package.provider/pack expects {field} as a non-empty String"
        )),
    }
}

fn package_tool_module(value: &Value) -> Result<crate::vm::bundle::BytecodeBundleModule, String> {
    let resource = package_tool_string(
        map_value(value, &vm_tool_keyword("module/resource")),
        ":module/resource",
    )?;
    let namespace_form = package_tool_string(
        map_value(value, &vm_tool_keyword("module/namespace-form")),
        ":module/namespace-form",
    )?;
    let source_digest = vm_tool_bytes(
        map_value(value, &vm_tool_keyword("module/source-digest"))
            .ok_or("tool.package.provider/pack requires :module/source-digest")?,
        "pack",
    )?;
    let source_digest: [u8; 32] = source_digest.try_into().map_err(|_| {
        "tool.package.provider/pack expects a 32-byte :module/source-digest".to_owned()
    })?;
    let dependencies = match map_value(value, &vm_tool_keyword("module/dependencies")) {
        Some(Value::Vector(values)) => values
            .iter()
            .map(|value| match value {
                Value::String(value) => Ok(value.clone()),
                _ => Err("tool.package.provider/pack expects String dependencies".to_owned()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(
                "tool.package.provider/pack expects :module/dependencies as a vector".into(),
            )
        }
    };
    let eager = match map_value(value, &vm_tool_keyword("module/eager")) {
        Some(Value::Bool(value)) => *value,
        _ => return Err("tool.package.provider/pack expects :module/eager as a boolean".into()),
    };
    let artifact = vm_tool_bytes(
        map_value(value, &vm_tool_keyword("module/artifact"))
            .ok_or("tool.package.provider/pack requires :module/artifact")?,
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

fn package_tool_value(module: crate::vm::bundle::BytecodeBundleModule) -> Value {
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
            Value::Bytes(module.source_digest.to_vec()),
        ),
        (
            vm_tool_keyword("module/dependencies"),
            vm_tool_vector(module.dependencies.into_iter().map(Value::String)),
        ),
        (vm_tool_keyword("module/eager"), Value::Bool(module.eager)),
        (
            vm_tool_keyword("module/artifact"),
            Value::Bytes(module.artifact),
        ),
    ])
}

fn package_tool_decode(
    value: &Value,
    operation: &str,
) -> Result<Vec<crate::vm::bundle::BytecodeBundleModule>, String> {
    let bytes = vm_tool_bytes(value, operation)?;
    crate::vm::bundle::decode_bytecode_bundle(&bytes)
}

pub(crate) fn package_tool_provider_values() -> Vec<(&'static str, Value)> {
    vec![
        (
            "provider",
            native_function("tool.package.provider/provider", 0, |_| {
                Ok(package_tool_descriptor())
            }),
        ),
        (
            "validate",
            native_function("tool.package.provider/validate", 1, |arguments| {
                package_tool_decode(&arguments[0], "validate")?;
                Ok(Value::Bool(true))
            }),
        ),
        (
            "inspect",
            native_function("tool.package.provider/inspect", 1, |arguments| {
                let modules = package_tool_decode(&arguments[0], "inspect")?;
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
            }),
        ),
        (
            "pack",
            native_function("tool.package.provider/pack", 1, |arguments| {
                let modules = match &arguments[0] {
                    Value::Vector(values) => values
                        .iter()
                        .map(package_tool_module)
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => {
                        return Err("tool.package.provider/pack expects a vector of modules".into())
                    }
                };
                for module in &modules {
                    crate::vm::decode_program(&module.artifact).map_err(|error| {
                        format!("{}: invalid HBC0 artifact: {error}", module.resource)
                    })?;
                }
                crate::vm::bundle::encode_bytecode_bundle(&modules).map(Value::Bytes)
            }),
        ),
        (
            "unpack",
            native_function("tool.package.provider/unpack", 1, |arguments| {
                package_tool_decode(&arguments[0], "unpack")
                    .map(|modules| vm_tool_vector(modules.into_iter().map(package_tool_value)))
            }),
        ),
    ]
}
