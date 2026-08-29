fn vm_tool_keyword(name: &str) -> Value {
    Value::Keyword(name.into())
}

fn vm_tool_vector(values: impl IntoIterator<Item = Value>) -> Value {
    Value::Vector(PVector::from_iter(values))
}

fn vm_tool_map(entries: impl IntoIterator<Item = (Value, Value)>) -> Value {
    Value::OrderedMap(Box::new(POrderedMap::from_iter(entries)))
}

fn vm_tool_keywords(values: &[&str]) -> Value {
    vm_tool_vector(values.iter().map(|value| vm_tool_keyword(value)))
}

fn vm_tool_provider_descriptor() -> Value {
    #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
    let operations = &[
        "validate",
        "inspect",
        "transform",
        "execute",
        "disassemble",
        "conform",
    ][..];
    #[cfg(all(feature = "bytecode-vm", not(feature = "halc-encoder")))]
    let operations = &["validate", "inspect", "execute", "disassemble", "conform"][..];
    #[cfg(all(not(feature = "bytecode-vm"), feature = "halc-encoder"))]
    let operations = &["validate", "inspect", "transform"][..];
    #[cfg(all(not(feature = "bytecode-vm"), not(feature = "halc-encoder")))]
    let operations = &["validate", "inspect"][..];

    #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
    let formats = vec![
        (vm_tool_keyword("hal"), vm_tool_vector(std::iter::empty())),
        (
            vm_tool_keyword("halc"),
            vm_tool_keywords(&["validate", "inspect", "execute", "conform"]),
        ),
        (
            vm_tool_keyword("hbc"),
            vm_tool_keywords(&["validate", "inspect", "execute", "disassemble", "conform"]),
        ),
    ];
    #[cfg(all(feature = "bytecode-vm", not(feature = "halc-encoder")))]
    let formats = vec![
        (
            vm_tool_keyword("halc"),
            vm_tool_keywords(&["validate", "inspect"]),
        ),
        (
            vm_tool_keyword("hbc"),
            vm_tool_keywords(&["validate", "inspect", "execute", "disassemble", "conform"]),
        ),
    ];
    #[cfg(all(not(feature = "bytecode-vm"), feature = "halc-encoder"))]
    let formats = vec![
        (vm_tool_keyword("hal"), vm_tool_vector(std::iter::empty())),
        (
            vm_tool_keyword("halc"),
            vm_tool_keywords(&["validate", "inspect"]),
        ),
    ];
    #[cfg(all(not(feature = "bytecode-vm"), not(feature = "halc-encoder")))]
    let formats = vec![(
        vm_tool_keyword("halc"),
        vm_tool_keywords(&["validate", "inspect"]),
    )];

    let mut transforms = Vec::new();
    #[cfg(feature = "halc-encoder")]
    transforms.push(vm_tool_vector([
        vm_tool_keyword("hal"),
        vm_tool_keyword("halc"),
    ]));
    #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
    {
        transforms.push(vm_tool_vector([
            vm_tool_keyword("hal"),
            vm_tool_keyword("hbc"),
        ]));
        transforms.push(vm_tool_vector([
            vm_tool_keyword("halc"),
            vm_tool_keyword("hbc"),
        ]));
    }

    vm_tool_map([
        (vm_tool_keyword("provider/id"), vm_tool_keyword("rust")),
        (
            vm_tool_keyword("provider/operations"),
            vm_tool_keywords(operations),
        ),
        (vm_tool_keyword("provider/formats"), vm_tool_map(formats)),
        (
            vm_tool_keyword("provider/transforms"),
            vm_tool_vector(transforms),
        ),
        (vm_tool_keyword("provider/engines"), {
            let mut engines = Vec::new();
            #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
            engines.push((vm_tool_keyword("halc"), vm_tool_keyword("lower-and-run")));
            #[cfg(feature = "bytecode-vm")]
            engines.push((vm_tool_keyword("hbc"), vm_tool_keyword("stack-vm")));
            vm_tool_map(engines)
        }),
    ])
}

fn vm_tool_transform_format<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    match value {
        Value::Keyword(format)
            if format.get_namespace().is_none()
                && matches!(format.get_name(), "hal" | "halc" | "hbc") =>
        {
            Ok(format.get_name())
        }
        _ => Err(format!(
            "tool.vm.provider/transform expects :hal, :halc, or :hbc as {field}"
        )),
    }
}

fn vm_tool_source(value: &Value) -> Result<String, String> {
    match value {
        Value::String(source) => Ok(source.clone()),
        _ => Err("tool.vm.provider/transform expects HAL source as a String".into()),
    }
}

#[cfg(feature = "halc-encoder")]
fn vm_tool_namespace(forms: &[crate::kernel::Form]) -> Result<String, String> {
    use crate::kernel::Form;

    forms
        .iter()
        .find_map(|form| match form {
            Form::List(values)
                if matches!(values.first(), Some(Form::Symbol(head)) if head == "ns" || head == "ns+") =>
            {
                match values.get(1) {
                    Some(Form::Symbol(namespace)) => Some(namespace.clone()),
                    _ => None,
                }
            }
            _ => None,
        })
        .ok_or_else(|| "HAL source does not declare an ns or ns+ namespace".to_owned())
}

fn vm_tool_resource(options: &Value, default: String) -> Result<String, String> {
    let entries = map_entries(options)
        .ok_or_else(|| "tool.vm.provider/transform expects options as a map".to_owned())?;
    for (key, _) in entries {
        if key != vm_tool_keyword("resource") {
            return Err(format!(
                "tool.vm.provider/transform does not support option {}",
                key.display()
            ));
        }
    }
    match map_value(options, &vm_tool_keyword("resource")) {
        Some(Value::String(resource)) => Ok(resource.clone()),
        Some(_) => Err("tool.vm.provider/transform expects :resource as a String".into()),
        None => Ok(default),
    }
}

fn vm_tool_transform(
    from: &str,
    to: &str,
    input: &Value,
    options: &Value,
) -> Result<Vec<u8>, String> {
    match (from, to) {
        ("hal", "halc") => {
            #[cfg(feature = "halc-encoder")]
            {
                let source = vm_tool_source(input)?;
                let forms = crate::kernel::parse_forms(&source)?;
                let namespace = vm_tool_namespace(&forms)?;
                let resource =
                    vm_tool_resource(options, format!("{}.hal", namespace.replace('.', "/")))?;
                crate::kernel::halc::encode_halc_module(&namespace, &resource, &source, forms)
            }
            #[cfg(not(feature = "halc-encoder"))]
            {
                let _ = input;
                Err(
                    "tool.vm.provider does not support :hal -> :halc in this runtime profile"
                        .into(),
                )
            }
        }
        ("hal", "hbc") => {
            #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
            {
                vm_tool_resource(options, String::new())?;
                let program = crate::vm::compile_source(&vm_tool_source(input)?)
                    .map_err(|error| error.to_string())?;
                crate::vm::encode_program(&program)
            }
            #[cfg(not(all(feature = "bytecode-vm", feature = "halc-encoder")))]
            {
                let _ = input;
                Err("tool.vm.provider does not support :hal -> :hbc in this runtime profile".into())
            }
        }
        ("halc", "hbc") => {
            #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
            {
                vm_tool_resource(options, String::new())?;
                let bytes = vm_tool_bytes(input, "transform")?;
                let module = crate::kernel::halc::decode_halc(&bytes)?;
                let registry = crate::core::namespace_registry()?;
                let program = crate::vm::compile_halc_module(&module, &registry)
                    .map_err(|error| error.to_string())?;
                crate::vm::encode_program(&program)
            }
            #[cfg(not(all(feature = "bytecode-vm", feature = "halc-encoder")))]
            {
                let _ = input;
                Err(
                    "tool.vm.provider does not support :halc -> :hbc in this runtime profile"
                        .into(),
                )
            }
        }
        _ => Err(format!(
            "tool.vm.provider does not support :{from} -> :{to} in this runtime profile"
        )),
    }
}

fn vm_tool_empty_options(options: &Value, operation: &str) -> Result<(), String> {
    let entries = map_entries(options)
        .ok_or_else(|| format!("tool.vm.provider/{operation} expects options as a map"))?;
    if let Some((key, _)) = entries.first() {
        return Err(format!(
            "tool.vm.provider/{operation} does not support option {}",
            key.display()
        ));
    }
    Ok(())
}

#[cfg(feature = "bytecode-vm")]
fn vm_tool_execute_program(program: crate::vm::Program) -> Result<Value, String> {
    use std::rc::Rc;

    let registry = crate::core::namespace_registry()?;
    let snapshot = registry.snapshot();
    match crate::vm::execute_program_with_globals(Rc::new(program), &registry) {
        Ok(value) => Ok(value),
        Err(error) => {
            registry.restore(snapshot);
            Err(error.to_string())
        }
    }
}

fn vm_tool_execute(format: &str, bytes: &[u8], options: &Value) -> Result<Value, String> {
    vm_tool_empty_options(options, "execute")?;
    match format {
        "halc" => {
            #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
            {
                let module = crate::kernel::halc::decode_halc(bytes)?;
                let registry = crate::core::namespace_registry()?;
                let snapshot = registry.snapshot();
                let program = match crate::vm::compile_halc_module(&module, &registry) {
                    Ok(program) => program,
                    Err(error) => {
                        registry.restore(snapshot);
                        return Err(error.to_string());
                    }
                };
                match crate::vm::execute_program_with_globals(std::rc::Rc::new(program), &registry)
                {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        registry.restore(snapshot);
                        Err(error.to_string())
                    }
                }
            }
            #[cfg(not(all(feature = "bytecode-vm", feature = "halc-encoder")))]
            {
                let _ = bytes;
                Err(
                    "tool.vm.provider does not support HALC execution in this runtime profile"
                        .into(),
                )
            }
        }
        "hbc" => {
            #[cfg(feature = "bytecode-vm")]
            {
                let program = crate::vm::decode_program(bytes)?;
                vm_tool_execute_program(program)
            }
            #[cfg(not(feature = "bytecode-vm"))]
            {
                let _ = bytes;
                Err(
                    "tool.vm.provider does not support HBC execution in this runtime profile"
                        .into(),
                )
            }
        }
        _ => Err(format!("unknown tool.vm format: :{format}")),
    }
}

fn vm_tool_format<'a>(value: &'a Value, operation: &str) -> Result<&'a str, String> {
    match value {
        Value::Keyword(format)
            if format.get_namespace().is_none() && matches!(format.get_name(), "halc" | "hbc") =>
        {
            Ok(format.get_name())
        }
        _ => Err(format!(
            "tool.vm.provider/{operation} expects :halc or :hbc as its format"
        )),
    }
}

fn vm_tool_bytes(value: &Value, operation: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        Value::ByteBuffer(bytes) => Ok(bytes.borrow().clone()),
        _ => Err(format!("tool.vm.provider/{operation} expects Bytes")),
    }
}

fn vm_tool_validate(format: &str, bytes: &[u8]) -> Result<(), String> {
    match format {
        "halc" => crate::kernel::halc::decode_halc(bytes).map(|_| ()),
        "hbc" => {
            #[cfg(feature = "bytecode-vm")]
            {
                crate::vm::decode_program(bytes).map(|_| ())
            }
            #[cfg(not(feature = "bytecode-vm"))]
            {
                let _ = bytes;
                Err("tool.vm.provider does not support :hbc in this runtime profile".into())
            }
        }
        _ => Err(format!("unknown tool.vm format: :{format}")),
    }
}

fn vm_tool_checksum(bytes: &[u8], start: usize) -> Value {
    Value::Bytes(bytes[start..start + 32].to_vec())
}

fn vm_tool_names(values: impl Iterator<Item = String>) -> Value {
    let mut values = values.collect::<Vec<_>>();
    values.sort();
    vm_tool_vector(values.into_iter().map(Value::String))
}

fn vm_tool_inspect_halc(bytes: &[u8]) -> Result<Value, String> {
    let module = crate::kernel::halc::decode_halc(bytes)?;
    let payload_bytes = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let origin = match module.origin {
        crate::kernel::halc::HalcOrigin::Halc => "halc",
        crate::kernel::halc::HalcOrigin::LegacyHir => "legacy-hir",
    };
    Ok(vm_tool_map([
        (vm_tool_keyword("artifact/format"), vm_tool_keyword("halc")),
        (vm_tool_keyword("artifact/version"), Value::Number(1)),
        (vm_tool_keyword("artifact/origin"), vm_tool_keyword(origin)),
        (
            vm_tool_keyword("artifact/bytes"),
            Value::Number(bytes.len() as i64),
        ),
        (
            vm_tool_keyword("payload/bytes"),
            Value::Number(payload_bytes as i64),
        ),
        (
            vm_tool_keyword("payload/checksum"),
            vm_tool_checksum(bytes, 12),
        ),
        (
            vm_tool_keyword("module/namespace"),
            Value::String(module.namespace),
        ),
        (
            vm_tool_keyword("module/resource"),
            Value::String(module.resource),
        ),
        (
            vm_tool_keyword("source/hash"),
            Value::Bytes(module.source_hash),
        ),
        (
            vm_tool_keyword("forms/count"),
            Value::Number(module.forms.len() as i64),
        ),
        (
            vm_tool_keyword("schemas/definitions"),
            vm_tool_names(module.schemas.definitions.keys().cloned()),
        ),
        (
            vm_tool_keyword("schemas/functions"),
            vm_tool_names(module.schemas.functions.keys().cloned()),
        ),
    ]))
}

#[cfg(feature = "bytecode-vm")]
fn vm_tool_inspect_hbc(bytes: &[u8]) -> Result<Value, String> {
    let program = crate::vm::decode_program(bytes)?;
    let payload_bytes = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let checksum_start = 8 + payload_bytes;
    let instructions = program
        .functions
        .iter()
        .map(|function| function.code.len())
        .sum::<usize>();
    let handlers = program
        .functions
        .iter()
        .map(|function| function.handlers.len())
        .sum::<usize>();
    Ok(vm_tool_map([
        (vm_tool_keyword("artifact/format"), vm_tool_keyword("hbc")),
        (vm_tool_keyword("artifact/version"), Value::Number(0)),
        (
            vm_tool_keyword("artifact/bytes"),
            Value::Number(bytes.len() as i64),
        ),
        (
            vm_tool_keyword("payload/bytes"),
            Value::Number(payload_bytes as i64),
        ),
        (
            vm_tool_keyword("payload/checksum"),
            vm_tool_checksum(bytes, checksum_start),
        ),
        (
            vm_tool_keyword("module/namespace"),
            program.namespace.map(Value::String).unwrap_or(Value::Nil),
        ),
        (
            vm_tool_keyword("program/entry"),
            Value::Number(i64::from(program.entry)),
        ),
        (
            vm_tool_keyword("constants/count"),
            Value::Number(program.constants.len() as i64),
        ),
        (
            vm_tool_keyword("functions/count"),
            Value::Number(program.functions.len() as i64),
        ),
        (
            vm_tool_keyword("instructions/count"),
            Value::Number(instructions as i64),
        ),
        (
            vm_tool_keyword("handlers/count"),
            Value::Number(handlers as i64),
        ),
    ]))
}

fn vm_tool_inspect(format: &str, bytes: &[u8]) -> Result<Value, String> {
    match format {
        "halc" => vm_tool_inspect_halc(bytes),
        "hbc" => {
            #[cfg(feature = "bytecode-vm")]
            {
                vm_tool_inspect_hbc(bytes)
            }
            #[cfg(not(feature = "bytecode-vm"))]
            {
                let _ = bytes;
                Err("tool.vm.provider does not support :hbc in this runtime profile".into())
            }
        }
        _ => Err(format!("unknown tool.vm format: :{format}")),
    }
}

fn vm_tool_disassemble(bytes: &[u8]) -> Result<String, String> {
    #[cfg(feature = "bytecode-vm")]
    {
        let program = crate::vm::decode_program(bytes)?;
        Ok(crate::vm::disassemble(&program))
    }
    #[cfg(not(feature = "bytecode-vm"))]
    {
        let _ = bytes;
        Err("tool.vm.provider does not support HBC disassembly in this runtime profile".into())
    }
}

pub(crate) fn vm_tool_provider_values() -> Vec<(&'static str, Value)> {
    vec![
        (
            "provider",
            native_function("tool.vm.provider/provider", 0, |_| {
                Ok(vm_tool_provider_descriptor())
            }),
        ),
        (
            "validate",
            native_function("tool.vm.provider/validate", 2, |arguments| {
                let format = vm_tool_format(&arguments[0], "validate")?;
                let bytes = vm_tool_bytes(&arguments[1], "validate")?;
                vm_tool_validate(format, &bytes)?;
                Ok(Value::Bool(true))
            }),
        ),
        (
            "inspect",
            native_function("tool.vm.provider/inspect", 2, |arguments| {
                let format = vm_tool_format(&arguments[0], "inspect")?;
                let bytes = vm_tool_bytes(&arguments[1], "inspect")?;
                vm_tool_inspect(format, &bytes)
            }),
        ),
        (
            "disassemble",
            native_function("tool.vm.provider/disassemble", 1, |arguments| {
                let bytes = vm_tool_bytes(&arguments[0], "disassemble")?;
                vm_tool_disassemble(&bytes).map(Value::String)
            }),
        ),
        (
            "transform",
            native_function("tool.vm.provider/transform", 4, |arguments| {
                let from = vm_tool_transform_format(&arguments[0], "source format")?;
                let to = vm_tool_transform_format(&arguments[1], "target format")?;
                vm_tool_transform(from, to, &arguments[2], &arguments[3]).map(Value::Bytes)
            }),
        ),
        (
            "execute",
            native_function("tool.vm.provider/execute", 3, |arguments| {
                let format = vm_tool_format(&arguments[0], "execute")?;
                let bytes = vm_tool_bytes(&arguments[1], "execute")?;
                vm_tool_execute(format, &bytes, &arguments[2])
            }),
        ),
    ]
}

#[cfg(test)]
mod vm_tool_tests {
    use super::*;

    fn field(value: &Value, key: &str) -> Value {
        map_value(value, &vm_tool_keyword(key))
            .cloned()
            .unwrap_or(Value::Nil)
    }

    #[cfg(any(feature = "halc-encoder", feature = "bytecode-vm"))]
    fn hex_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn provider_reports_exact_feature_sensitive_capabilities() {
        let provider = vm_tool_provider_descriptor();
        assert_eq!(field(&provider, "provider/id").display(), ":rust");
        #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
        assert_eq!(
            field(&provider, "provider/operations").display(),
            "[:validate :inspect :transform :execute :disassemble :conform]"
        );
        #[cfg(all(feature = "bytecode-vm", not(feature = "halc-encoder")))]
        assert_eq!(
            field(&provider, "provider/operations").display(),
            "[:validate :inspect :execute :disassemble :conform]"
        );
        #[cfg(not(feature = "bytecode-vm"))]
        assert_eq!(
            field(&provider, "provider/operations").display(),
            if cfg!(feature = "halc-encoder") {
                "[:validate :inspect :transform]"
            } else {
                "[:validate :inspect]"
            }
        );
        #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
        assert_eq!(
            field(&provider, "provider/transforms").display(),
            "[[:hal :halc] [:hal :hbc] [:halc :hbc]]"
        );
        #[cfg(all(feature = "bytecode-vm", not(feature = "halc-encoder")))]
        assert_eq!(field(&provider, "provider/transforms").display(), "[]");
        #[cfg(all(not(feature = "bytecode-vm"), feature = "halc-encoder"))]
        assert_eq!(
            field(&provider, "provider/transforms").display(),
            "[[:hal :halc]]"
        );
        #[cfg(all(not(feature = "bytecode-vm"), not(feature = "halc-encoder")))]
        assert_eq!(field(&provider, "provider/transforms").display(), "[]");
        #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
        assert_eq!(
            field(&provider, "provider/engines").display(),
            "{:halc :lower-and-run :hbc :stack-vm}"
        );
        #[cfg(all(feature = "bytecode-vm", not(feature = "halc-encoder")))]
        assert_eq!(
            field(&provider, "provider/engines").display(),
            "{:hbc :stack-vm}"
        );
        #[cfg(not(feature = "bytecode-vm"))]
        assert_eq!(field(&provider, "provider/engines").display(), "{}");
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn hbc_execution_authenticates_before_transactional_execution() {
        let options = vm_tool_map(std::iter::empty());
        let bytes = hex_bytes(concat!(
            "484243300000005f0000010000000475736572000000010000000d4854413003",
            "000000000000002a000000000000000000000000000000000000000100000000",
            "0000000000000100000002000000000018000000020100000000000000010000",
            "000100000000006073811fa3086d8edff969b6f31169f2d358937b295630863e",
            "c63366450debec",
        ));
        let registry = crate::kernel::NamespaceRegistry::new("user");
        let value = crate::core::with_namespace_registry(&registry, || {
            vm_tool_execute("hbc", &bytes, &options)
        })
        .unwrap();
        assert_eq!(value, Value::Number(42));

        let mut corrupt = bytes;
        corrupt[12] ^= 1;
        let error = crate::core::with_namespace_registry(&registry, || {
            vm_tool_execute("hbc", &corrupt, &options)
        })
        .unwrap_err();
        assert!(error.contains("checksum"));

        let failing = crate::vm::encode_program(
            &crate::vm::compile_source("(do (def tool-vm-leaked 1) (/ 1 0))").unwrap(),
        )
        .unwrap();
        let error = crate::core::with_namespace_registry(&registry, || {
            vm_tool_execute("hbc", &failing, &options)
        })
        .unwrap_err();
        assert!(error.contains("division by zero"));
        assert!(registry
            .current()
            .resolve(&crate::lang::data::Symbol::parse("tool-vm-leaked"))
            .is_none());
    }

    #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
    #[test]
    fn halc_execution_lowers_and_matches_hbc_observable_value() {
        let options = vm_tool_map(std::iter::empty());
        let source = Value::String("(ns sample.execute) (+ 19 23)".into());
        let halc = vm_tool_transform("hal", "halc", &source, &options).unwrap();
        let registry = crate::kernel::NamespaceRegistry::new("user");
        let (halc_value, hbc_value) = crate::core::with_namespace_registry(&registry, || {
            let hbc =
                vm_tool_transform("halc", "hbc", &Value::Bytes(halc.clone()), &options).unwrap();
            (
                vm_tool_execute("halc", &halc, &options).unwrap(),
                vm_tool_execute("hbc", &hbc, &options).unwrap(),
            )
        });
        assert_eq!(halc_value, Value::Number(42));
        assert_eq!(hbc_value, Value::Number(42));
    }

    #[cfg(all(feature = "bytecode-vm", feature = "halc-encoder"))]
    #[test]
    fn transforms_use_canonical_halc_and_hbc_compilers() {
        let module_source = Value::String("(ns sample.transform) (def value 42)".into());
        let options = vm_tool_map(std::iter::empty());
        let halc = vm_tool_transform("hal", "halc", &module_source, &options).unwrap();
        vm_tool_validate("halc", &halc).unwrap();

        let direct_hbc =
            vm_tool_transform("hal", "hbc", &Value::String("(+ 19 23)".into()), &options).unwrap();
        vm_tool_validate("hbc", &direct_hbc).unwrap();
        assert_eq!(
            direct_hbc,
            vm_tool_transform("hal", "hbc", &Value::String("(+ 19 23)".into()), &options,).unwrap()
        );

        let registry = crate::kernel::NamespaceRegistry::new("user");
        let lowered_hbc = crate::core::with_namespace_registry(&registry, || {
            vm_tool_transform("halc", "hbc", &Value::Bytes(halc), &options)
        })
        .unwrap();
        vm_tool_validate("hbc", &lowered_hbc).unwrap();
    }

    #[cfg(feature = "halc-encoder")]
    #[test]
    fn transformations_reject_malformed_inputs_and_unknown_options() {
        let options = vm_tool_map(std::iter::empty());
        assert!(!vm_tool_transform(
            "hal",
            "halc",
            &Value::String("(ns malformed".into()),
            &options,
        )
        .unwrap_err()
        .is_empty());

        let unknown = vm_tool_map([(vm_tool_keyword("fallback"), vm_tool_keyword("rust"))]);
        assert_eq!(
            vm_tool_transform(
                "hal",
                "halc",
                &Value::String("(ns sample.options)".into()),
                &unknown,
            )
            .unwrap_err(),
            "tool.vm.provider/transform does not support option :fallback"
        );
    }

    #[cfg(feature = "halc-encoder")]
    #[test]
    fn hal_to_halc_transform_matches_the_truffle_golden() {
        let source = "(ns tool.vm.parity)\n(def answer (+ 19 23))\n";
        let expected = hex_bytes(concat!(
            "48414c4300010001000000b6865eb5f9ac7dc1198d8a6345686ffa257eee",
            "e47e15bd45418766fa26e8edaa520000000e746f6f6c2e766d2e70617269",
            "74790000001b66697874757265732f746f6f6c2d766d2d7061726974792e",
            "68616c25e7e2e6fedd97d111cd6f9554c8d4bf51e11dbdcf5746d4f35c27",
            "f96198d598000000020b000000020900000000026e730009000000000e74",
            "6f6f6c2e766d2e70617269747900000b0000000309000000000364656600",
            "090000000006616e73776572000b000000030900000000012b0003000000",
            "00000000130300000000000000170000",
        ));
        let options = vm_tool_map([(
            vm_tool_keyword("resource"),
            Value::String("fixtures/tool-vm-parity.hal".into()),
        )]);
        let actual =
            vm_tool_transform("hal", "halc", &Value::String(source.into()), &options).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn halc_validation_and_inspection_use_canonical_decoder() {
        let source = "(ns sample.vm) (def value 42)";
        let forms = crate::kernel::parse_forms(source).unwrap();
        let bytes =
            crate::kernel::halc::encode_halc_module("sample.vm", "sample/vm.hal", source, forms)
                .unwrap();
        vm_tool_validate("halc", &bytes).unwrap();
        let inspection = vm_tool_inspect("halc", &bytes).unwrap();
        assert_eq!(field(&inspection, "artifact/format").display(), ":halc");
        assert_eq!(
            field(&inspection, "module/namespace").display(),
            "\"sample.vm\""
        );
        assert_eq!(field(&inspection, "forms/count").display(), "2");
        assert_eq!(field(&inspection, "artifact/origin").display(), ":halc");
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn hbc_validation_inspection_and_disassembly_use_canonical_vm() {
        let program = crate::vm::compile_source("(+ 19 23)").unwrap();
        let bytes = crate::vm::encode_program(&program).unwrap();
        vm_tool_validate("hbc", &bytes).unwrap();
        let inspection = vm_tool_inspect("hbc", &bytes).unwrap();
        assert_eq!(field(&inspection, "artifact/format").display(), ":hbc");
        assert_eq!(field(&inspection, "functions/count").display(), "1");
        assert!(vm_tool_disassemble(&bytes)
            .unwrap()
            .starts_with("== program:"));
    }
}
