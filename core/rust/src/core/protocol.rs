pub(crate) fn value_to_metadata(value: &Value) -> Result<MetadataValue, String> {
    match value {
        Value::Nil => Ok(MetadataValue::Nil),
        Value::Bool(value) => Ok(MetadataValue::Boolean(*value)),
        Value::Number(value) => Ok(MetadataValue::Number(*value)),
        Value::Float(value) => Ok(MetadataValue::Float(crate::numeric::finite_float(*value)?)),
        Value::BigInteger(value) => Ok(MetadataValue::BigInteger(value.clone())),
        Value::Character(value) => Ok(MetadataValue::Character(*value)),
        Value::Regex(value) => Ok(MetadataValue::Regex(value.clone())),
        Value::Tagged(value) => Ok(MetadataValue::Tagged(
            value.tag().get_name().into(),
            Box::new(value_to_metadata(value.form())?),
        )),
        Value::String(value) => Ok(MetadataValue::String(value.clone())),
        Value::Keyword(value) => Ok(MetadataValue::Keyword(value.clone())),
        Value::Symbol(value) => Ok(MetadataValue::Symbol(value.clone())),
        Value::Tuple(values) => Ok(MetadataValue::Vector(
            values
                .iter()
                .map(value_to_metadata)
                .collect::<Result<_, _>>()?,
        )),
        Value::Vector(values) => Ok(MetadataValue::Vector(
            values
                .iter()
                .map(value_to_metadata)
                .collect::<Result<_, _>>()?,
        )),
        Value::MapEntry(entry) => Ok(MetadataValue::Vector(
            entry
                .iter()
                .map(value_to_metadata)
                .collect::<Result<_, _>>()?,
        )),
        Value::Queue(values) => Ok(MetadataValue::List(
            values
                .iter()
                .map(value_to_metadata)
                .collect::<Result<_, _>>()?,
        )),
        Value::Deque(values) => Ok(MetadataValue::List(
            values
                .iter()
                .map(value_to_metadata)
                .collect::<Result<_, _>>()?,
        )),
        Value::List(values) => Ok(MetadataValue::List(
            values
                .iter()
                .map(value_to_metadata)
                .collect::<Result<_, _>>()?,
        )),
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            Ok(MetadataValue::Set(
                set_items(value)
                    .unwrap()
                    .into_iter()
                    .map(value_to_metadata)
                    .collect::<Result<_, _>>()?,
            ))
        }
        value @ (Value::Map(_) | Value::OrderedMap(_) | Value::SortedMap(_) | Value::Trie(_)) => {
            Ok(MetadataValue::Map(
                map_entries(value)
                    .unwrap()
                    .iter()
                    .map(|(key, value)| Ok((value_to_metadata(key)?, value_to_metadata(value)?)))
                    .collect::<Result<_, String>>()?,
            ))
        }
        _ => Err("value cannot be stored in runtime-neutral metadata".into()),
    }
}

fn metadata_to_value(value: &MetadataValue) -> Result<Value, String> {
    match value {
        MetadataValue::Nil => Ok(Value::Nil),
        MetadataValue::Boolean(value) => Ok(Value::Bool(*value)),
        MetadataValue::Number(value) => Ok(Value::Number(*value)),
        MetadataValue::Float(value) => Ok(Value::Float(crate::numeric::finite_float(*value)?)),
        MetadataValue::BigInteger(value) => Ok(crate::numeric::compact_integer(value.clone())),
        MetadataValue::Character(value) => Ok(Value::Character(*value)),
        MetadataValue::Regex(value) => Ok(Value::Regex(value.clone())),
        MetadataValue::Tagged(tag, value) => Ok(Value::Tagged(Box::new(PTaggedLiteral::new(
            Symbol::parse(tag),
            metadata_to_value(value)?,
        )))),
        MetadataValue::String(value) => Ok(Value::String(value.clone())),
        MetadataValue::Keyword(value) => Ok(Value::Keyword(value.clone())),
        MetadataValue::Symbol(value) => Ok(Value::Symbol(value.clone())),
        MetadataValue::Vector(values) => Ok(Value::Vector(
            values
                .iter()
                .map(metadata_to_value)
                .collect::<Result<_, _>>()?,
        )),
        MetadataValue::List(values) => Ok(Value::List(
            values
                .iter()
                .map(metadata_to_value)
                .collect::<Result<_, _>>()?,
        )),
        MetadataValue::Set(values) => Ok(Value::Set(
            values
                .iter()
                .map(metadata_to_value)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        )),
        MetadataValue::Map(values) => Ok(Value::Map(
            values
                .iter()
                .map(|(key, value)| Ok((metadata_to_value(key)?, metadata_to_value(value)?)))
                .collect::<Result<Vec<_>, String>>()?
                .into_iter()
                .collect(),
        )),
    }
}

fn value_metadata(value: &Value) -> Option<Rc<Metadata>> {
    match value {
        Value::Symbol(value) => value.meta().cloned(),
        Value::Pointer(value) => value.meta().cloned(),
        Value::Tuple(value) => value.meta().cloned(),
        Value::Vector(value) => value.meta().cloned(),
        Value::MapEntry(value) => value.meta().cloned(),
        Value::List(value) => value.meta().cloned(),
        Value::Cons(value) => value.meta().cloned(),
        Value::Queue(value) => value.meta().cloned(),
        Value::Deque(value) => value.meta().cloned(),
        Value::Map(value) => value.meta().cloned(),
        Value::OrderedMap(value) => value.meta().cloned(),
        Value::SortedMap(value) => value.meta().cloned(),
        Value::Trie(value) => value.meta().cloned(),
        Value::PriorityMap(value) => value.meta().cloned(),
        Value::Set(value) => value.meta().cloned(),
        Value::OrderedSet(value) => value.meta().cloned(),
        Value::SortedSet(value) => value.meta().cloned(),
        Value::Seq(value) => value.meta().cloned(),
        Value::Var(value) => value.hara_metadata(),
        Value::Function(value) => value.metadata.clone(),
        Value::Struct(value) => value.metadata.clone(),
        Value::Mutable(value) => value.metadata.clone(),
        Value::NativeType(value) => value.metadata.clone(),
        _ => None,
    }
}

fn protocol_meta(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 1 {
        return Err("IObjType/meta expects one argument".into());
    }
    match value_metadata(&arguments[0]) {
        None => Ok(Value::Nil),
        Some(metadata) => metadata_to_value(&MetadataValue::Map(metadata.entries().to_vec())),
    }
}

fn protocol_with_meta(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 2 {
        return Err("IObjType/with-meta expects a value and metadata map".into());
    }
    let metadata = match &arguments[1] {
        Value::Nil => None,
        value => {
            let MetadataValue::Map(entries) = value_to_metadata(value)? else {
                return Err("IObjType/with-meta expects a metadata map or nil".into());
            };
            Some(Metadata::new(entries))
        }
    };
    attach_optional_metadata(arguments[0].clone(), metadata)
}

fn collection_delimiters(value: &Value) -> Option<(&'static str, &'static str)> {
    match value {
        Value::List(_) | Value::Cons(_) | Value::Seq(_) => Some(("(", ")")),
        Value::Queue(_) | Value::Deque(_) | Value::Tuple(_) | Value::Vector(_) => Some(("[", "]")),
        Value::Map(_) | Value::OrderedMap(_) | Value::SortedMap(_) | Value::PriorityMap(_) => {
            Some(("{", "}"))
        }
        Value::Trie(_) | Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_) => {
            Some(("#{", "}"))
        }
        _ => None,
    }
}

fn protocol_coll_start(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => collection_delimiters(value)
            .map(|(start, _)| Value::String(start.into()))
            .ok_or_else(|| "IColl/start-string expects a collection".into()),
        _ => Err("IColl/start-string expects one collection".into()),
    }
}

fn protocol_coll_end(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => collection_delimiters(value)
            .map(|(_, end)| Value::String(end.into()))
            .ok_or_else(|| "IColl/end-string expects a collection".into()),
        _ => Err("IColl/end-string expects one collection".into()),
    }
}

fn protocol_coll_sep(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [_] => Ok(Value::String(" ".into())),
        _ => Err("IColl/sep-string expects one collection".into()),
    }
}

fn protocol_metatype(arguments: &[Value]) -> Result<Value, String> {
    let [value] = arguments else {
        return Err("IMetadata/metatype expects one value".into());
    };
    if !Value::supports_native_iobjtype(value) {
        return Err("IMetadata/metatype expects a metadata-capable value".into());
    }
    let name = match value {
        Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_) => "map",
        Value::Keyword(_) | Value::Symbol(_) => "string",
        _ => "object",
    };
    Ok(Value::Keyword(Keyword::from(name)))
}

fn protocol_count(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() == 1 {
        collection_count(&arguments[0])
            .map_err(|error| format!("protocol/unsupported-receiver: {error}"))
    } else {
        Err("ICount/count expects one argument".into())
    }
}

fn protocol_nth(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 2 {
        return Err("INth/nth expects a collection and index".into());
    }
    if let Value::Bytes(bytes) = &arguments[0] {
        let index = value_index(&arguments[1])?;
        return bytes
            .get(index)
            .map(|byte| Value::Number(*byte as i8 as i64))
            .ok_or_else(|| "nth index out of bounds".into());
    }
    if let Value::ByteBuffer(bytes) = &arguments[0] {
        let index = value_index(&arguments[1])?;
        return bytes
            .borrow()
            .get(index)
            .map(|byte| Value::Number(*byte as i8 as i64))
            .ok_or_else(|| "nth index out of bounds".into());
    }
    collection_nth(&arguments[0], &arguments[1])
}

fn namespaced_parts(value: &Value) -> Option<(String, Option<String>)> {
    match value {
        Value::Keyword(value) => Some((
            value.get_name().to_owned(),
            value.get_namespace().map(str::to_owned),
        )),
        Value::Symbol(value) => Some((
            value.get_name().to_owned(),
            value.get_namespace().map(str::to_owned),
        )),
        Value::Var(value) => Some((
            value.get_name().to_owned(),
            value.get_namespace().map(str::to_owned),
        )),
        Value::NativeType(value) => value
            .name
            .rsplit_once('.')
            .map(|(namespace, name)| (name.to_owned(), Some(namespace.to_owned()))),
        _ => None,
    }
}

fn protocol_namespaced_name(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 1 {
        return Err("INamespaced/name expects one value".into());
    }
    namespaced_parts(&arguments[0])
        .map(|(name, _)| Value::String(name))
        .ok_or_else(|| "INamespaced/name has no implementation for this value".into())
}

fn protocol_namespaced_namespace(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 1 {
        return Err("INamespaced/namespace expects one value".into());
    }
    namespaced_parts(&arguments[0])
        .map(|(_, namespace)| namespace.map(Value::String).unwrap_or(Value::Nil))
        .ok_or_else(|| "INamespaced/namespace has no implementation for this value".into())
}

fn protocol_string_like_to_string(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Keyword(value)] => Ok(Value::String(value.as_str().into())),
        [Value::Symbol(value)] => Ok(Value::String(value.as_str().into())),
        [_] => Err("IStringLike/to-string expects a string-like value".into()),
        _ => Err("IStringLike/to-string expects one argument".into()),
    }
}

fn protocol_string_like_from_string(arguments: &[Value]) -> Result<Value, String> {
    let [sample, Value::String(text)] = arguments else {
        return Err("IStringLike/from-string expects a sample and string".into());
    };
    match sample {
        Value::Keyword(_) => Keyword::parse(text).map(Value::Keyword),
        Value::Symbol(_) => Ok(Value::Symbol(Symbol::parse(text))),
        _ => Err("IStringLike/from-string expects a string-like sample".into()),
    }
}

fn protocol_lookup(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() == 2 || arguments.len() == 3 {
        collection_get(
            &arguments[0],
            &arguments[1],
            arguments.get(2).cloned().unwrap_or(Value::Nil),
        )
    } else {
        Err("ILookup/lookup expects a collection, key, and optional default".into())
    }
}

fn protocol_pointer_context(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Pointer(pointer)] => Ok(Value::Keyword(pointer.context().clone())),
        _ => Err("IPointer/ptr-context expects one pointer".into()),
    }
}

fn pointer_default(pointer: &PPointer) -> Result<Value, String> {
    let resolver = vm_resolve_global("std.lib.context.space/space:rt-current")?.deref_value();
    call_value(resolver, vec![Value::Keyword(pointer.context().clone())])
        .map_err(|error| format!("pointer/runtime-unavailable: {error}"))
}

fn pointer_context_eval(
    pointer: &PPointer,
    runtime: Value,
    method: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    let mut call = vec![runtime, Value::Pointer(pointer.clone())];
    call.extend_from_slice(arguments);
    protocol_call("std.protocol.icontexteval.IContextEval", method, &call)
}

fn pointer_arguments(arguments: &[Value]) -> Value {
    Value::Vector(PVector::from_iter(arguments.iter().cloned()))
}

fn pointer_invoke_ptr(
    pointer: &PPointer,
    runtime: Value,
    arguments: Value,
) -> Result<Value, String> {
    pointer_context_eval(pointer, runtime, "invoke-ptr", &[arguments])
}

fn pointer_transform_in(
    pointer: &PPointer,
    runtime: Value,
    arguments: Value,
) -> Result<Value, String> {
    pointer_context_eval(pointer, runtime, "transform-in-ptr", &[arguments])
}

fn pointer_transform_out(
    pointer: &PPointer,
    runtime: Value,
    value: Value,
) -> Result<Value, String> {
    pointer_context_eval(pointer, runtime, "transform-out-ptr", &[value])
}

pub(crate) fn pointer_invoke(
    pointer: &PPointer,
    runtime: Value,
    arguments: &[Value],
) -> Result<Value, String> {
    let input = pointer_transform_in(pointer, runtime.clone(), pointer_arguments(arguments))?;
    let output = pointer_invoke_ptr(pointer, runtime.clone(), input)?;
    pointer_transform_out(pointer, runtime, output)
}

fn protocol_apply_default(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Pointer(pointer)] => pointer_default(pointer),
        _ => Err("IApplicable/apply-default expects one pointer".into()),
    }
}

fn protocol_apply_in(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Pointer(pointer), runtime, values] => {
            pointer_invoke_ptr(pointer, runtime.clone(), values.clone())
        }
        _ => Err("IApplicable/apply-in expects a pointer, runtime, and arguments".into()),
    }
}

fn protocol_transform_in(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Pointer(pointer), runtime, values] => {
            pointer_transform_in(pointer, runtime.clone(), values.clone())
        }
        _ => Err("IApplicable/transform-in expects a pointer, runtime, and arguments".into()),
    }
}

fn protocol_transform_out(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Pointer(pointer), runtime, _, value] => {
            pointer_transform_out(pointer, runtime.clone(), value.clone())
        }
        _ => {
            Err("IApplicable/transform-out expects a pointer, runtime, arguments, and value".into())
        }
    }
}

fn protocol_assoc(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() == 3 {
        collection_assoc(&arguments[0], &arguments[1], arguments[2].clone())
    } else {
        Err("IAssoc/assoc expects a collection, key, and value".into())
    }
}

fn protocol_dissoc(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() == 2 {
        collection_dissoc(&arguments[0], &[arguments[1].clone()])
    } else {
        Err("IDissoc/dissoc expects a collection and key".into())
    }
}

fn pair_parts(value: &Value) -> Option<(Value, Value)> {
    match value {
        Value::MapEntry(entry) => Some((entry.key().clone(), entry.value().clone())),
        _ => None,
    }
}

/// Map construction accepts the conventional two-item vector syntax without
/// conflating that structural convenience with the dedicated `IPair`/MapEntry
/// representation.  `pair?` and `IPair` remain MapEntry-only.
fn map_conj_parts(value: &Value) -> Option<(Value, Value)> {
    if let Some(parts) = pair_parts(value) {
        return Some(parts);
    }
    match value {
        Value::Tuple(values) if values.len() == 2 => {
            Some((
                values.get(0).expect("two-item tuple").clone(),
                values.get(1).expect("two-item tuple").clone(),
            ))
        }
        Value::Vector(values) if values.len() == 2 => {
            Some((
                values.get(0).expect("two-item vector").clone(),
                values.get(1).expect("two-item vector").clone(),
            ))
        }
        _ => None,
    }
}

fn pair_value(key: Value, value: Value) -> Value {
    Value::MapEntry(Box::new(PMapEntry::new(key, value)))
}

fn indexed_find(value: Option<&Value>, index: usize) -> Result<Value, String> {
    Ok(value
        .map(|value| pair_value(Value::Number(index as i64), value.clone()))
        .unwrap_or(Value::Nil))
}

fn protocol_find(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 2 {
        return Err("IFind/find expects a collection and key".into());
    }
    let collection = &arguments[0];
    let key = &arguments[1];
    match collection {
        Value::Extension(receiver) => {
            extension_protocol_call(receiver, "std.protocol.ifind.IFind", "find", arguments)
        }
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => Ok(map_entries(value)
            .unwrap()
            .into_iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(candidate, value)| pair_value(candidate, value))
            .unwrap_or(Value::Nil)),
        Value::Pointer(pointer) => Ok(pointer
            .fields()
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(candidate, value)| pair_value(candidate.clone(), value.clone()))
            .unwrap_or(Value::Nil)),
        Value::Object(values) => {
            let key = match key {
                Value::String(value) => value.as_str(),
                Value::Keyword(value) => value.as_str(),
                _ => return Err("IFind/find object expects a string or keyword key".into()),
            };
            Ok(values
                .borrow()
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(candidate, value)| {
                    pair_value(Value::String(candidate.clone()), value.clone())
                })
                .unwrap_or(Value::Nil))
        }
        Value::Struct(value) => Ok(named_field_name(key)
            .and_then(|name| value.get(name).cloned().map(|item| (name, item)))
            .map(|(name, item)| pair_value(named_field_key(name), item))
            .unwrap_or(Value::Nil)),
        Value::Mutable(value) => Ok(named_field_name(key)
            .and_then(|name| value.get(name).map(|item| (name, item)))
            .map(|(name, item)| pair_value(named_field_key(name), item))
            .unwrap_or(Value::Nil)),
        Value::MutableCollection(collection) => {
            let borrowed = collection.borrow();
            let mutable = borrowed
                .as_ref()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::Map(values) => Ok(values
                    .find_entry(key)
                    .map(|(candidate, value)| pair_value(candidate.clone(), value.clone()))
                    .unwrap_or(Value::Nil)),
                MutableCollection::OrderedMap(values) => Ok(values
                    .find_entry(key)
                    .map(|(candidate, value)| pair_value(candidate.clone(), value.clone()))
                    .unwrap_or(Value::Nil)),
                MutableCollection::SortedMap(values) => Ok(values
                    .find_entry(key)
                    .map(|(candidate, value)| pair_value(candidate.clone(), value.clone()))
                    .unwrap_or(Value::Nil)),
                MutableCollection::Trie(values) => {
                    let key = marker_key(key, "IFind/find trie")?;
                    Ok(values
                        .get(&key)
                        .map(|value| pair_value(Value::String(key), value.clone()))
                        .unwrap_or(Value::Nil))
                }
                MutableCollection::Set(values) => {
                    Ok(values.get(key).cloned().unwrap_or(Value::Nil))
                }
                MutableCollection::OrderedSet(values) => {
                    Ok(values.get(key).cloned().unwrap_or(Value::Nil))
                }
                MutableCollection::SortedSet(values) => {
                    Ok(values.get(key).cloned().unwrap_or(Value::Nil))
                }
                MutableCollection::List(values) => {
                    let index = value_index(key)?;
                    indexed_find(values.get(index), index)
                }
                MutableCollection::Queue(values) => {
                    let index = value_index(key)?;
                    indexed_find(values.get(index), index)
                }
                MutableCollection::Vector(values) => {
                    let index = value_index(key)?;
                    indexed_find(values.get(index), index)
                }
            }
        }
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            Ok(set_find(value, key).unwrap_or(Value::Nil))
        }
        Value::Tuple(values) => indexed_find(values.get(value_index(key)?), value_index(key)?),
        Value::Vector(values) => indexed_find(values.get(value_index(key)?), value_index(key)?),
        Value::MapEntry(entry) => indexed_find(entry.nth(value_index(key)?), value_index(key)?),
        Value::Seq(values) => {
            let index = value_index(key)?;
            let value = values.iter().nth(index).transpose()?;
            indexed_find(value.as_ref(), index)
        }
        Value::List(values) => indexed_find(values.get(value_index(key)?), value_index(key)?),
        Value::Cons(values) => {
            let index = value_index(key)?;
            indexed_find(values.iter().nth(index).as_ref(), index)
        }
        Value::Queue(values) => indexed_find(values.get(value_index(key)?), value_index(key)?),
        Value::Deque(values) => indexed_find(values.get(value_index(key)?), value_index(key)?),
        _ => Err("IFind/find has no implementation for this value".into()),
    }
}

fn protocol_iter(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Extension(receiver)] => {
            extension_protocol_call(receiver, "std.protocol.iiter.IIter", "iter", arguments)
        }
        [value]
            if matches!(
                value,
                Value::Iterator(_)
                    | Value::Nil
                    | Value::String(_)
                    | Value::Bytes(_)
                    | Value::ByteBuffer(_)
                    | Value::Array(_)
                    | Value::Object(_)
                    | Value::Struct(_)
                    | Value::Mutable(_)
                    | Value::Map(_)
                    | Value::OrderedMap(_)
                    | Value::SortedMap(_)
                    | Value::Trie(_)
                    | Value::PriorityMap(_)
                    | Value::Pointer(_)
                    | Value::Set(_)
                    | Value::OrderedSet(_)
                    | Value::SortedSet(_)
                    | Value::List(_)
                    | Value::Cons(_)
                    | Value::Queue(_)
                    | Value::Deque(_)
                    | Value::Tuple(_)
                    | Value::MapEntry(_)
                    | Value::Vector(_)
            ) =>
        {
            make_iterator(value.clone())
        }
        _ => Err("IIter/iter expects one value".into()),
    }
}

fn protocol_deref(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Atom(atom)] => Ok(atom.deref_value()),
        [Value::Var(var)] => Ok(var.deref_value()),
        [Value::Promise(promise)] => promise_value_result(promise),
        [Value::Result(result)] => result.deref_value(),
        [Value::Pointer(pointer)] => pointer_context_eval(
            pointer,
            pointer_default(pointer)?,
            "deref-ptr",
            &[],
        ),
        [Value::Schema(schema)] => {
            form_to_value(&crate::lang::protocol::IDeref::deref(&schema.ast))
        }
        _ => Err("IDeref/deref has no implementation for this value".into()),
    }
}

pub(crate) fn protocol_deref_fiber(arguments: Vec<Value>, k: Cont) -> Step {
    match arguments.as_slice() {
        [Value::Promise(promise)] => match promise.state() {
            PromiseState::Fulfilled(value) => k(Ok(value)),
            PromiseState::Rejected(error) => k(Err(promise_rejection_error(error))),
            PromiseState::Pending => Step::Wait(
                promise.clone(),
                Box::new(move |state| match state {
                    PromiseState::Fulfilled(value) => k(Ok(value)),
                    PromiseState::Rejected(error) => k(Err(promise_rejection_error(error))),
                    PromiseState::Pending => k(Err("deref resumed pending promise".into())),
                }),
            ),
        },
        _ => k(protocol_call(
            "std.protocol.ideref.IDeref",
            "deref",
            &arguments,
        )),
    }
}

fn protocol_deref_timeout(arguments: &[Value]) -> Result<Value, String> {
    let [target, milliseconds, timeout] = arguments else {
        return Err("IDerefTimeout/deref-timeout expects three arguments".into());
    };
    let milliseconds = value_u64_integer(milliseconds, "IDerefTimeout/deref-timeout")
        .map_err(|_| "IDerefTimeout/deref-timeout expects non-negative milliseconds".to_string())?;
    match target {
        Value::Promise(promise) => {
            match promise.wait_state_timeout(std::time::Duration::from_millis(milliseconds)) {
                PromiseState::Fulfilled(value) => Ok(value),
                PromiseState::Rejected(error) => Err(promise_rejection_error(error)),
                PromiseState::Pending => Ok(timeout.clone()),
            }
        }
        Value::Atom(atom) => Ok(atom.deref_value()),
        Value::Var(var) => Ok(var.deref_value()),
        _ => Err(
            "IDerefTimeout/deref-timeout expects a dereferenceable value, milliseconds, and timeout value"
                .into(),
        ),
    }
}

fn protocol_reset(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Atom(atom), value] => atom.reset(value.clone()),
        _ => Err("IReset/reset expects an atom and value".into()),
    }
}

fn protocol_cas(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Atom(atom), old_value, new_value] => Ok(Value::Bool(
            atom.compare_and_set(old_value, new_value.clone())?,
        )),
        _ => Err("ICas/cas expects an atom, old value, and new value".into()),
    }
}

const REDUCED_TAG_NAMESPACE: &str = "hara.internal";
const REDUCED_TAG_NAME: &str = "reduced";

fn reduced_value(value: Value) -> Value {
    Value::Tagged(Box::new(PTaggedLiteral::new(
        Symbol::create(Some(REDUCED_TAG_NAMESPACE), REDUCED_TAG_NAME),
        value,
    )))
}

fn reduced_value_ref(value: &Value) -> Option<&Value> {
    match value {
        Value::Tagged(tagged)
            if tagged.tag().get_namespace() == Some(REDUCED_TAG_NAMESPACE)
                && tagged.tag().get_name() == REDUCED_TAG_NAME =>
        {
            Some(tagged.form())
        }
        _ => None,
    }
}

fn is_reduced_value(value: &Value) -> bool {
    reduced_value_ref(value).is_some()
}

fn unreduced_value(value: Value) -> Value {
    match value {
        Value::Tagged(tagged)
            if tagged.tag().get_namespace() == Some(REDUCED_TAG_NAMESPACE)
                && tagged.tag().get_name() == REDUCED_TAG_NAME =>
        {
            tagged.into_form()
        }
        value => value,
    }
}

fn reduce_iterator(
    function: &Rc<Function>,
    initial: Option<Value>,
    source: Value,
    operation: &str,
) -> Result<Value, String> {
    let iterator = make_iterator(source)?;
    let result = (|| {
        let mut accumulator = initial;
        while let Some(value) = iterator_try_next(&iterator)? {
            let next = match accumulator {
                Some(current) => call_function(function, vec![current, value])?,
                None => value,
            };
            if is_reduced_value(&next) {
                return Ok(unreduced_value(next));
            }
            accumulator = Some(next);
        }
        accumulator.ok_or_else(|| format!("{operation} cannot reduce an empty value without init"))
    })();
    let close = iterator_close(&iterator);
    match result {
        Err(error) => {
            let _ = close;
            Err(error)
        }
        Ok(value) => {
            close?;
            Ok(value)
        }
    }
}

fn schema_kind(schema: &crate::kernel::SchemaType) -> &'static str {
    use crate::kernel::SchemaType::*;
    match schema {
        Primitive(_) => "primitive",
        Reference(_) => "reference",
        Union(_) => "union",
        Vector(_) => "vector",
        Set(_) => "set",
        Tuple(_) => "tuple",
        Map(_) => "map",
        Struct { .. } => "struct",
        WithProperties { schema, .. } => schema_kind(schema),
        Function(arities) if arities.len() == 1 => "fn",
        Function(_) => "function",
        Enum(_) => "enum",
        Extension { .. } => "extension",
        Unknown(_) => "unknown",
    }
}

fn schema_ast_map(entries: Vec<(&str, Form)>) -> Form {
    Form::Map(
        entries
            .into_iter()
            .map(|(key, value)| (Form::Keyword(key.into()), value))
            .collect(),
    )
}

fn schema_function_ast(arity: &crate::kernel::FunctionSchema) -> Form {
    schema_ast_map(vec![
        ("kind", Form::Keyword("fn".into())),
        (
            "inputs",
            schema_ast_map(vec![
                (
                    "fixed",
                    Form::Vector(arity.fixed.iter().map(schema_ast_form).collect()),
                ),
                (
                    "rest",
                    arity
                        .rest
                        .as_deref()
                        .map(schema_ast_form)
                        .unwrap_or(Form::Nil),
                ),
            ]),
        ),
        ("output", schema_ast_form(&arity.output)),
    ])
}

fn schema_ast_form(schema: &crate::kernel::SchemaType) -> Form {
    use crate::kernel::SchemaType::*;
    match schema {
        Primitive(name) => schema_ast_map(vec![
            ("kind", Form::Keyword("primitive".into())),
            ("name", Form::Keyword(name.clone())),
        ]),
        Reference(name) => schema_ast_map(vec![
            ("kind", Form::Keyword("reference".into())),
            ("name", Form::Symbol(name.clone())),
        ]),
        Union(values) => schema_ast_map(vec![
            ("kind", Form::Keyword("union".into())),
            (
                "types",
                Form::Vector(values.iter().map(schema_ast_form).collect()),
            ),
        ]),
        Vector(value) => schema_ast_map(vec![
            ("kind", Form::Keyword("vector".into())),
            ("item", schema_ast_form(value)),
        ]),
        Set(value) => schema_ast_map(vec![
            ("kind", Form::Keyword("set".into())),
            ("item", schema_ast_form(value)),
        ]),
        Tuple(values) => schema_ast_map(vec![
            ("kind", Form::Keyword("tuple".into())),
            (
                "items",
                Form::Vector(values.iter().map(schema_ast_form).collect()),
            ),
        ]),
        Map(fields) => schema_ast_map(vec![
            ("kind", Form::Keyword("map".into())),
            (
                "fields",
                Form::Vector(
                    fields
                        .iter()
                        .map(|field| {
                            let mut entries = vec![("name", field.name.clone())];
                            if let Some(properties) = &field.properties {
                                entries.push(("properties", properties.clone()));
                            }
                            entries.push(("type", schema_ast_form(&field.value_type)));
                            schema_ast_map(entries)
                        })
                        .collect(),
                ),
            ),
        ]),
        Struct {
            name,
            mutable,
            fields,
        } => schema_ast_map(vec![
            ("kind", Form::Keyword("struct".into())),
            ("name", Form::Symbol(name.clone())),
            ("mutable?", Form::Bool(*mutable)),
            (
                "fields",
                Form::Vector(
                    fields
                        .iter()
                        .map(|field| {
                            let mut entries = vec![("name", field.name.clone())];
                            if let Some(properties) = &field.properties {
                                entries.push(("properties", properties.clone()));
                            }
                            entries.push(("type", schema_ast_form(&field.value_type)));
                            schema_ast_map(entries)
                        })
                        .collect(),
                ),
            ),
        ]),
        Function(arities) if arities.len() == 1 => schema_function_ast(&arities[0]),
        Function(arities) => schema_ast_map(vec![
            ("kind", Form::Keyword("function".into())),
            (
                "arities",
                Form::Vector(arities.iter().map(schema_function_ast).collect()),
            ),
        ]),
        Enum(values) => schema_ast_map(vec![
            ("kind", Form::Keyword("enum".into())),
            ("values", Form::Vector(values.clone())),
        ]),
        WithProperties { schema, properties } => {
            let Form::Map(mut entries) = schema_ast_form(schema) else {
                unreachable!("canonical schema AST must be a map");
            };
            entries.push((Form::Keyword("properties".into()), properties.clone()));
            Form::Map(entries)
        }
        Extension { head, arguments } => {
            let surface = Form::Vector(
                std::iter::once(Form::Keyword(head.clone()))
                    .chain(arguments.iter().cloned())
                    .collect(),
            );
            schema_ast_map(vec![
                ("kind", Form::Keyword("extension".into())),
                ("head", Form::Keyword(head.clone())),
                ("arguments", Form::Vector(arguments.clone())),
                ("surface", surface),
            ])
        }
        Unknown(value) => schema_ast_map(vec![
            ("kind", Form::Keyword("unknown".into())),
            ("surface", value.clone()),
        ]),
    }
}

fn schema_value_to_form(value: &Value) -> Result<Form, String> {
    match value {
        Value::Var(var) => Ok(Form::List(vec![
            Form::Symbol("var".into()),
            Form::Symbol(var.symbol().as_str().into()),
        ])),
        Value::Tagged(value) => Ok(Form::Tagged(
            value.tag().get_name().into(),
            Box::new(schema_value_to_form(value.form())?),
        )),
        Value::Tuple(values) => Ok(Form::Vector(
            values
                .iter()
                .map(|value| schema_value_to_form(&value))
                .collect::<Result<_, _>>()?,
        )),
        Value::Vector(values) => Ok(Form::Vector(
            values
                .iter()
                .map(schema_value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        Value::List(values) => Ok(Form::List(
            values
                .iter()
                .map(schema_value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        Value::Queue(values) => Ok(Form::List(
            values
                .iter()
                .map(schema_value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        Value::Deque(values) => Ok(Form::List(
            values
                .iter()
                .map(schema_value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        Value::Cons(values) => Ok(Form::List(
            values
                .iter()
                .map(|value| schema_value_to_form(&value))
                .collect::<Result<_, _>>()?,
        )),
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => Ok(Form::Set(
            set_items(value)
                .unwrap()
                .into_iter()
                .map(schema_value_to_form)
                .collect::<Result<_, _>>()?,
        )),
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => Ok(Form::Map(
            map_entries(value)
                .unwrap()
                .into_iter()
                .map(|(key, value)| {
                    Ok((schema_value_to_form(&key)?, schema_value_to_form(&value)?))
                })
                .collect::<Result<_, String>>()?,
        )),
        value => value_to_form(value),
    }
}

fn compile_schema_value(value: &Value, origin: Option<KernelVar<Value>>) -> Result<Value, String> {
    if let Value::Schema(schema) = value {
        return Ok(Value::Schema(schema.clone()));
    }
    if let Value::Var(var) = value {
        return compile_schema_value(&var.deref_value(), Some(var.clone()));
    }
    let form = schema_value_to_form(value).map_err(|_| "schema expects schema data".to_string())?;
    let ast = crate::kernel::normalize_schema(&form)
        .map_err(|error| format!("invalid schema: {error}"))?;
    if matches!(ast, crate::kernel::SchemaType::Unknown(_)) {
        return Err("schema expects schema data".into());
    }
    Ok(Value::Schema(Rc::new(RuntimeSchema { form, ast, origin })))
}

fn declared_schema_contract(var: &KernelVar<Value>) -> Result<Option<Value>, String> {
    let Some(metadata) = var.hara_metadata() else {
        return Ok(None);
    };
    let Some(raw) = metadata.get_keyword("schema") else {
        return Ok(None);
    };
    let form = metadata_value_to_form(raw);
    if let Form::List(reference) = &form {
        if let [Form::Symbol(operator), Form::Symbol(target)] = reference.as_slice() {
            if operator == "var" {
                let registry = namespace_registry()?;
                let referenced = registry
                    .resolve(&Symbol::parse(target))
                    .ok_or_else(|| format!("schema Var does not exist: {target}"))?;
                return compile_schema_value(&referenced.deref_value(), Some(referenced)).map(Some);
            }
        }
    }
    let value = form_to_value(&form)?;
    compile_schema_value(&value, Some(var.clone())).map(Some)
}

fn refresh_schema_contract(var: &KernelVar<Value>) -> Result<(), String> {
    let contract = declared_schema_contract(var)?;
    var.set_schema_contract(contract);
    Ok(())
}

fn schema_contract(var: &KernelVar<Value>) -> Result<Value, String> {
    Ok(var.schema_contract().unwrap_or(Value::Nil))
}

fn native_schema_values(method: &str, values: &[Value]) -> Result<Value, String> {
    let [value] = values else {
        return Err(format!("Schema/{method} expects one value"));
    };
    match method {
        "compile" => compile_schema_value(value, None),
        "of" => match value {
            Value::Var(var) => schema_contract(var),
            _ => Err("Schema/of expects a Var".into()),
        },
        "kind" => match value {
            Value::Schema(schema) => Ok(Value::Keyword(Keyword::from(schema_kind(&schema.ast)))),
            _ => Err("Schema/kind expects a schema".into()),
        },
        "form" => match value {
            Value::Schema(schema) => form_to_value(&schema.form),
            _ => Err("Schema/form expects a schema".into()),
        },
        "ast" => match value {
            Value::Schema(schema) => form_to_value(&schema_ast_form(&schema.ast)),
            _ => Err("Schema/ast expects a schema".into()),
        },
        "origin" => match value {
            Value::Schema(schema) => {
                Ok(schema.origin.clone().map(Value::Var).unwrap_or(Value::Nil))
            }
            _ => Err("Schema/origin expects a schema".into()),
        },
        _ => Err(format!(
            "unknown Schema operation: std.native.Schema/{method}"
        )),
    }
}

fn protocol_reduce(arguments: &[Value]) -> Result<Value, String> {
    let (source, function, accumulator) = match arguments {
        [source, Value::Function(function), initial] => (source, function, Some(initial.clone())),
        [source, Value::Function(function)] => (source, function, None),
        _ => {
            return Err(
                "IReduce/reduce expects a value, function, and optional initial value".into(),
            )
        }
    };
    reduce_iterator(function, accumulator, source.clone(), "IReduce/reduce")
}

fn base_namespace(value: &Value, operation: &str) -> Result<crate::kernel::Namespace<Value>, String> {
    let Value::Namespace(namespace) = value else {
        return Err(format!("Base/{operation} expects a Namespace value"));
    };
    let registry = namespace_registry()?;
    registry
        .find(namespace.name().as_str())
        .filter(|candidate| candidate.same_identity(namespace.as_ref()))
        .ok_or_else(|| format!("Base/{operation} received a Namespace from another runtime"))
}

fn base_namespace_name(value: &Value, operation: &str) -> Result<String, String> {
    match value {
        Value::Symbol(name) if name.get_namespace().is_none() => Ok(name.as_str().to_owned()),
        _ => Err(format!("Base/{operation} expects an unqualified namespace symbol")),
    }
}

fn base_symbol(value: &Value, operation: &str) -> Result<String, String> {
    match value {
        Value::Symbol(symbol) if symbol.get_namespace().is_none() => Ok(symbol.as_str().to_owned()),
        _ => Err(format!("Base/{operation} expects an unqualified symbol")),
    }
}

fn base_metadata(value: &Value, operation: &str) -> Result<Option<Rc<Metadata>>, String> {
    match value {
        Value::Nil => Ok(None),
        value => match value_to_metadata(value)? {
            MetadataValue::Map(entries) => Ok(Some(Metadata::new(entries))),
            _ => Err(format!("Base/{operation} expects a metadata map or nil")),
        },
    }
}

fn base_fields(value: &Value, operation: &str) -> Result<Vec<NamedField>, String> {
    let values = match value {
        Value::Vector(values) => values.iter().cloned().collect::<Vec<_>>(),
        _ => return Err(format!("Base/{operation} expects a field vector")),
    };
    values
        .iter()
        .map(|value| match value {
            Value::Symbol(name) if name.get_namespace().is_none() => Ok(NamedField::legacy(name.as_str())),
            _ => NamedField::from_value(value, operation),
        })
        .collect()
}

fn base_protocol(value: &Value, operation: &str) -> Result<Rc<GuestProtocol>, String> {
    match value {
        Value::Protocol(protocol) => Ok(protocol.clone()),
        Value::Var(var) => match var.deref_value() {
            Value::Protocol(protocol) => Ok(protocol),
            _ => Err(format!("Base/{operation} expects a protocol")),
        },
        _ => Err(format!("Base/{operation} expects a protocol")),
    }
}

fn base_function(value: &Value, operation: &str) -> Result<Rc<Function>, String> {
    match value {
        Value::Function(function) => Ok(function.clone()),
        Value::Var(var) => match var.deref_value() {
            Value::Function(function) => Ok(function),
            _ => Err(format!("Base/{operation} expects a function")),
        },
        _ => Err(format!("Base/{operation} expects a function")),
    }
}

fn base_multimethod_name(
    namespace: &crate::kernel::Namespace<Value>,
    value: &Value,
    operation: &str,
) -> Result<String, String> {
    match value {
        Value::Symbol(symbol) => match symbol.get_namespace() {
            Some(_) => Ok(symbol.as_str().to_owned()),
            None => Ok(format!("{}/{}", namespace.name().as_str(), symbol.as_str())),
        },
        Value::Var(var) => Ok(var.symbol().as_str().to_owned()),
        _ => Err(format!("Base/{operation} expects a multimethod symbol or Var")),
    }
}

fn publish_multimethod(
    namespace: crate::kernel::Namespace<Value>,
    name: String,
    dispatch: Rc<Function>,
) -> Value {
    let qualified = format!("{}/{}", namespace.name().as_str(), name);
    let state = Rc::new(RefCell::new(MultiMethod {
        dispatch,
        methods: Vec::new(),
        default: None,
    }));
    let invoke_state = state.clone();
    let value = native_variadic_function(&qualified, move |arguments| {
        let state = invoke_state.borrow();
        let key = call_value(Value::Function(state.dispatch.clone()), arguments.clone())?;
        let method = state
            .methods
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, method)| method.clone())
            .or_else(|| state.default.clone())
            .ok_or_else(|| format!("No multimethod method for dispatch value {}", key.display()))?;
        call_value(Value::Function(method), arguments)
    });
    let var = namespace.intern(&name, value.clone());
    var.set_origin(definition_origin());
    register_multimethod(qualified, state);
    value
}

fn base_type_name(value: &Value, operation: &str) -> Result<String, String> {
    let mut current = value.clone();
    let mut seen = Vec::new();
    loop {
        match current {
            Value::StructType(ty) => return Ok(ty.name.clone()),
            Value::MutableType(ty) => return Ok(ty.name.clone()),
            Value::Var(var) => {
                let symbol = var.symbol().as_str().to_owned();
                if seen.iter().any(|candidate| candidate == &symbol) {
                    return Err(format!("Base/{operation} type Var cycle: {symbol}"));
                }
                seen.push(symbol);
                current = var.deref_value();
            }
            value => {
                return Err(format!(
                    "Base/{operation} expects a struct or mutable type, received {}",
                    value.display()
                ))
            }
        }
    }
}

fn with_base_namespace<R>(
    value: &Value,
    operation: &str,
    action: impl FnOnce() -> Result<R, String>,
) -> Result<R, String> {
    let namespace = base_namespace(value, operation)?;
    let registry = namespace_registry()?;
    let previous = registry.current().name().as_str().to_owned();
    registry.set_current(namespace.name().as_str());
    let result = action();
    registry.set_current(previous);
    result
}

fn native_base_values(operation: &str, values: &[Value]) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Base/")
        .or_else(|| operation.strip_prefix("Base/"))
        .unwrap_or(operation);
    match operation {
        "list" => Ok(Value::List(values.to_vec().into())),
        "vector" => Ok(Value::Vector(values.to_vec().into())),
        "vec" => match values {
            [value @ Value::Vector(_)] => Ok(value.clone()),
            [value] => Ok(Value::Vector(PVector::from_iter(iterator_values(
                value.clone(),
            )?))),
            _ => Err("Base/vec expects one collection".into()),
        },
        "set" => match values {
            [value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_))] => {
                Ok(value.clone())
            }
            [value] => Ok(Value::Set(
                unique_values(iterator_values(value.clone())?).into(),
            )),
            _ => Err("Base/set expects one collection".into()),
        },
        "hash-map" if values.len() % 2 == 0 => Ok(Value::Map(PMap::from_iter(
            values
                .chunks_exact(2)
                .map(|pair| (pair[0].clone(), pair[1].clone())),
        ))),
        "hash-map" => Err("Base/hash-map expects an even number of arguments".into()),
        "hash-set" => Ok(Value::Set(values.iter().cloned().collect())),
        "map-entry" => match values {
            [key, value] => Ok(Value::MapEntry(Box::new(PMapEntry::new(
                key.clone(),
                value.clone(),
            )))),
            _ => Err("Base/map-entry expects a key and value".into()),
        },
        "atom" => match values {
            [value] => Ok(Value::Atom(Box::new(RuntimeAtom::new(value.clone(), true)))),
            _ => Err("Base/atom expects one value".into()),
        },
        "bytes" => native_bytes_new(values),
        "pointer" => match values {
            [descriptor] => pointer_from_descriptor(descriptor.clone()),
            _ => Err("Base/pointer expects one descriptor map".into()),
        },
        "symbol" => match values {
            [Value::String(name)] => Ok(Value::Symbol(Symbol::parse(name))),
            [Value::String(namespace), Value::String(name)] => {
                Ok(Value::Symbol(Symbol::create(Some(namespace), name)))
            }
            _ => Err("Base/symbol expects a name or namespace and name".into()),
        },
        "keyword" => match values {
            [Value::String(name)] => Keyword::parse(name)
                .map(Value::Keyword)
                .map_err(|error| format!("Base/keyword failed: {error}")),
            [Value::String(namespace), Value::String(name)] => {
                Keyword::create(Some(namespace), name)
                    .map(Value::Keyword)
                    .map_err(|error| format!("Base/keyword failed: {error}"))
            }
            _ => Err("Base/keyword expects a name or namespace and name".into()),
        },
        "uuid" => uuid_value(values),
        "reduced" => match values {
            [value] => Ok(reduced_value(value.clone())),
            _ => Err("Base/reduced expects one value".into()),
        },
        "unreduced" => match values {
            [value] => Ok(unreduced_value(value.clone())),
            _ => Err("Base/unreduced expects one value".into()),
        },
        "hash" => match values {
            [value] => Ok(Value::Number(value.stable_hash() as i64)),
            _ => Err("Base/hash expects one value".into()),
        },
        "apply" => {
            if values.len() < 2 {
                return Err("Base/apply expects a function and a final sequential value".into());
            }
            let function = values[0].clone();
            let mut arguments = values[1..values.len() - 1].to_vec();
            arguments.extend(iterator_values(values.last().cloned().unwrap())?);
            call_value(function, arguments)
        }
        "resolve" => match values {
            [Value::Symbol(symbol)] => Ok(crate::core::namespace_registry()?
                .resolve(symbol)
                .map(Value::Var)
                .unwrap_or(Value::Nil)),
            [namespace, Value::Symbol(symbol)] if symbol.get_namespace().is_none() => {
                Ok(base_namespace(namespace, "resolve")?
                    .resolve(symbol)
                    .map(Value::Var)
                    .unwrap_or(Value::Nil))
            }
            _ => Err("Base/resolve expects one symbol".into()),
        },
        "namespace" => match values {
            [name] => {
                let name = base_namespace_name(name, "namespace")?;
                Ok(Value::Namespace(Rc::new(namespace_registry()?.find_or_create(name))))
            }
            _ => Err("Base/namespace expects one namespace symbol".into()),
        },
        "current-namespace" => match values {
            [] => Ok(Value::Namespace(Rc::new(namespace_registry()?.current()))),
            _ => Err("Base/current-namespace expects no arguments".into()),
        },
        "select-namespace" => match values {
            [namespace] => {
                let namespace = base_namespace(namespace, "select-namespace")?;
                Ok(Value::Namespace(Rc::new(
                    namespace_registry()?.set_current(namespace.name().as_str()),
                )))
            }
            _ => Err("Base/select-namespace expects one Namespace value".into()),
        },
        "def" => match values {
            [namespace, name, value, metadata] => {
                let name = base_symbol(name, "def")?;
                let metadata = base_metadata(metadata, "def")?;
                with_base_namespace(namespace, "def", || {
                    let macro_definition = metadata.as_ref().is_some_and(|metadata| {
                        matches!(metadata.get_keyword("macro"), Some(MetadataValue::Boolean(true)))
                    });
                    let var = if macro_definition {
                        vm_def_macro(&name, value.clone(), metadata)?
                    } else {
                        vm_def_global(&name, value.clone(), metadata)?
                    };
                    Ok(Value::Var(var))
                })
            }
            _ => Err("Base/def expects Namespace, symbol, value, and metadata".into()),
        },
        "struct" | "mutable" => match values {
            [namespace, name, fields] | [namespace, name, fields, Value::Nil] => {
                let name = base_symbol(name, operation)?;
                let fields = base_fields(fields, operation)?;
                let kind = if operation == "struct" { "defstruct" } else { "defmutable" };
                with_base_namespace(namespace, operation, || {
                    let mut environment = HashMap::new();
                    publish_named_value(kind, &name, fields, &mut environment, None)?;
                    Ok(namespace_registry()?
                        .current()
                        .resolve(&Symbol::parse(&name))
                        .map(|var| var.deref_value())
                        .ok_or_else(|| format!("Base/{operation} did not publish {name}"))?)
                })
            }
            [namespace, name, fields, metadata] => {
                let name = base_symbol(name, operation)?;
                let fields = base_fields(fields, operation)?;
                let metadata = base_metadata(metadata, operation)?;
                let kind = if operation == "struct" { "defstruct" } else { "defmutable" };
                with_base_namespace(namespace, operation, || {
                    let mut environment = HashMap::new();
                    publish_named_value(kind, &name, fields, &mut environment, metadata)?;
                    Ok(namespace_registry()?
                        .current()
                        .resolve(&Symbol::parse(&name))
                        .map(|var| var.deref_value())
                        .ok_or_else(|| format!("Base/{operation} did not publish {name}"))?)
                })
            }
            _ => Err(format!("Base/{operation} expects Namespace, symbol, fields, and optional metadata")),
        },
        "protocol" => match values {
            [namespace, name, methods, parents] => {
                let name = base_symbol(name, "protocol")?;
                let entries = map_entries(methods)
                    .ok_or_else(|| "Base/protocol expects a method arity map".to_string())?;
                let mut declarations = HashMap::new();
                for (method, arity) in entries {
                    let method = base_symbol(&method, "protocol")?;
                    let Value::Number(arity) = arity else {
                        return Err("Base/protocol method arities must be positive integers".into());
                    };
                    if arity <= 0 || declarations.insert(method, arity as usize).is_some() {
                        return Err("Base/protocol method declarations must be unique and have a receiver".into());
                    }
                }
                let parents = match parents {
                    Value::Vector(values) => values
                        .iter()
                        .map(|value| Ok(base_protocol(value, "protocol")?.name.clone()))
                        .collect::<Result<Vec<_>, String>>()?,
                    _ => return Err("Base/protocol expects a parent protocol vector".into()),
                };
                with_base_namespace(namespace, "protocol", || {
                    publish_guest_protocol(&name, declarations, parents, &mut HashMap::new())
                })
            }
            _ => Err("Base/protocol expects Namespace, symbol, method arities, and parents".into()),
        },
        "with-declaration" => match values {
            [namespace, thunk] => {
                let thunk = base_function(thunk, "with-declaration")?;
                if !thunk.accepts_arity(0) {
                    return Err("Base/with-declaration expects a zero-argument function".into());
                }
                with_base_namespace(namespace, "with-declaration", || {
                    with_declaration_transaction(&mut HashMap::new(), |_| {
                        call_value(Value::Function(thunk), Vec::new())
                    })
                })
            }
            _ => Err("Base/with-declaration expects Namespace and a zero-argument function".into()),
        },
        "extend" => match values {
            [namespace, type_value, protocol_value, implementations] => {
                let type_name = base_type_name(type_value, "extend")?;
                let protocol = base_protocol(protocol_value, "extend")?;
                let entries = map_entries(implementations)
                    .ok_or_else(|| "Base/extend expects a method function map".to_string())?;
                with_base_namespace(namespace, "extend", || {
                    with_declaration_transaction(&mut HashMap::new(), |_| {
                        let registry = active_protocol_registry()?;
                        for (method, function) in &entries {
                            let method = base_symbol(method, "extend")?;
                            if !protocol.methods.contains_key(&method) {
                                return Err(format!("Base/extend has no declared method: {method}"));
                            }
                            let function = base_function(function, "extend")?;
                            let expected_arity = protocol.methods[&method];
                            if !function.accepts_arity(expected_arity) {
                                return Err(format!(
                                    "Base/extend implementation for {method} does not accept {expected_arity} arguments"
                                ));
                            }
                            registry.register_guest(protocol.name.clone(), type_name.clone(), method, function);
                        }
                        Ok(Value::Nil)
                    })?;
                    Ok(type_value.clone())
                })
            }
            _ => Err("Base/extend expects Namespace, type, protocol, and method functions".into()),
        },
        "multimethod" => match values {
            [namespace, name, dispatch] => {
                let namespace_value = base_namespace(namespace, "multimethod")?;
                let name = base_symbol(name, "multimethod")?;
                let dispatch = base_function(dispatch, "multimethod")?;
                with_base_namespace(namespace, "multimethod", || {
                    with_declaration_transaction(&mut HashMap::new(), |_| {
                        Ok(publish_multimethod(namespace_value, name, dispatch))
                    })
                })
            }
            _ => Err("Base/multimethod expects Namespace, name, and dispatch function".into()),
        },
        "method" => match values {
            [namespace, multimethod, key, implementation] => {
                let namespace_value = base_namespace(namespace, "method")?;
                let multimethod = base_multimethod_name(&namespace_value, multimethod, "method")?;
                let implementation = base_function(implementation, "method")?;
                with_base_namespace(namespace, "method", || {
                    with_declaration_transaction(&mut HashMap::new(), |_| {
                        let state = multimethod_state(&multimethod)
                            .ok_or_else(|| "Base/method expects an existing multimethod".to_string())?;
                        let mut state = state.borrow_mut();
                        if matches!(key, Value::Keyword(keyword) if keyword.get_namespace().is_none() && keyword.get_name() == "default") {
                            state.default = Some(implementation);
                        } else if let Some((_, existing)) = state
                            .methods
                            .iter_mut()
                            .find(|(candidate, _)| candidate == key)
                        {
                            *existing = implementation;
                        } else {
                            state.methods.push((key.clone(), implementation));
                        }
                        Ok(Value::Nil)
                    })
                })
            }
            _ => Err("Base/method expects Namespace, multimethod, dispatch value, and function".into()),
        },
        "field" => match values {
            [value, field] => {
                let field = match field {
                    Value::Keyword(field) if field.get_namespace().is_none() => field.as_str(),
                    Value::Symbol(field) if field.get_namespace().is_none() => field.as_str(),
                    _ => return Err("Base/field expects an unqualified field keyword or symbol".into()),
                };
                mutable_field_value(value, field)
            }
            _ => Err("Base/field expects a mutable value and field name".into()),
        },
        "satisfies?" => match values {
            [protocol, value] => {
                let protocol = match protocol {
                    Value::Protocol(protocol) => protocol.clone(),
                    Value::Var(var) => match var.deref_value() {
                        Value::Protocol(protocol) => protocol,
                        _ => return Err("Base/satisfies? expects a protocol and value".into()),
                    },
                    _ => return Err("Base/satisfies? expects a protocol and value".into()),
                };
                Ok(Value::Bool(protocol_satisfies(protocol.as_ref(), value)))
            }
            _ => Err("Base/satisfies? expects a protocol and value".into()),
        },
        "special-symbol?" => match values {
            [Value::Symbol(symbol)] => Ok(Value::Bool(syntax_symbol(symbol.as_str()))),
            _ => Err("Base/special-symbol? expects one symbol".into()),
        },
        "type" => match values {
            [value] => Ok(Value::Keyword(portable_type_keyword(value)?)),
            _ => Err("Base/type expects one value".into()),
        },
        "instance?" => match values {
            [Value::StructType(_), value] | [Value::MutableType(_), value] => {
                named_instance_of(&values[0], value)
            }
            [Value::NativeType(native), value] => {
                Ok(Value::Bool(native_type_instance(native, value)?))
            }
            _ => Err("Base/instance? expects a type descriptor and value".into()),
        },
        predicate if predicate.ends_with('?') => match values {
            [value] => Ok(Value::Bool(match predicate {
                "number?" => numeric::is_numeric_value(value),
                "long?" => numeric::is_long_value(value),
                _ => return Err(format!("unknown Base predicate: {predicate}")),
            })),
            _ => Err(format!("Base/{predicate} expects one value")),
        },
        _ => Err(format!("unknown Base operation: {operation}")),
    }
}

fn native_algo_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let method = operation
        .strip_prefix("std.native.Algo/")
        .ok_or_else(|| format!("invalid Algo operation: {operation}"))?;
    if let Some(family) = method.strip_suffix('?') {
        if values.len() != 1 {
            return Err(format!("Algo/{method} expects one value"));
        }
        let value = &values[0];
        return Ok(Value::Bool(match family {
            "deque" => matches!(value, Value::Deque(_)),
            "ordered-map" => matches!(value, Value::OrderedMap(_)),
            "ordered-set" => matches!(value, Value::OrderedSet(_)),
            "priority-map" => matches!(value, Value::PriorityMap(_)),
            "queue" => matches!(value, Value::Queue(_)),
            "sorted-map" => matches!(value, Value::SortedMap(_)),
            "sorted-set" => matches!(value, Value::SortedSet(_)),
            "trie" => matches!(value, Value::Trie(_)),
            _ => return Err(format!("unknown Algo predicate: {method}")),
        }));
    }
    match method {
        "deque" | "ordered-map" | "ordered-set" | "priority-map" | "queue" | "sorted-map"
        | "sorted-set" | "trie" => collection_constructor_values(method, values),
        _ => Err(format!("unknown Algo operation: {operation}")),
    }
}

fn protocol_promise_state(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Promise(promise)] => Ok(promise_state_value(promise)),
        _ => Err("IPromise/state expects a promise".into()),
    }
}

fn protocol_promise_value(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Promise(promise)] => promise_value_result(promise),
        _ => Err("IPromise/value expects a promise".into()),
    }
}

fn protocol_promise_chain(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Promise(promise), Value::Function(function)] => Ok(Value::Promise(promise_chain(
            promise.clone(),
            operation,
            function.clone(),
        ))),
        _ => Err(format!(
            "IPromise/{operation} expects a promise and function"
        )),
    }
}

fn protocol_promise_cancel(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Promise(promise)] => {
            promise.cancel();
            Ok(Value::Promise(promise.clone()))
        }
        _ => Err("IPromise/cancel expects a promise".into()),
    }
}

fn protocol_coroutine_status(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Coroutine(coroutine)] => Ok(coroutine_status(coroutine)),
        _ => Err("ICoroutine/status expects a coroutine".into()),
    }
}

fn protocol_coroutine_resume(arguments: &[Value]) -> Result<Value, String> {
    let Some(Value::Coroutine(coroutine)) = arguments.first() else {
        return Err("ICoroutine/resume expects a coroutine".into());
    };
    fiber::coroutine::resume_sync(coroutine.clone(), arguments[1..].to_vec())
}

/// Fiber-aware protocol entry for coroutine resumption. The synchronous
/// protocol callback remains available to callers that explicitly request a
/// blocking value, while native bytecode and EvalFiber can retain a pending
/// await inside the coroutine instead of entering the tree evaluator.
pub(crate) fn protocol_coroutine_resume_fiber(arguments: Vec<Value>, k: Cont) -> Step {
    let Some(Value::Coroutine(coroutine)) = arguments.first() else {
        return k(protocol_coroutine_resume(&arguments));
    };
    fiber::coroutine::coroutine_resume(coroutine.clone(), arguments[1..].to_vec(), k)
}

fn protocol_watch_add(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Atom(atom), key, Value::Function(function)] => {
            atom.add_watch(key.clone(), function.clone())?;
            Ok(Value::Atom(atom.clone()))
        }
        _ => Err("IWatch/watch-add expects an atom, key, and function".into()),
    }
}

fn protocol_watch_remove(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Atom(atom), key] => {
            atom.remove_watch(key)?;
            Ok(Value::Atom(atom.clone()))
        }
        _ => Err("IWatch/watch-remove expects an atom and key".into()),
    }
}

fn protocol_watch_list(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Atom(atom)] => Ok(iterator_from_values(atom.watch_entries()?)),
        _ => Err("IWatch/watch-list expects an atom".into()),
    }
}

fn protocol_empty(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => collection_empty_value(value.clone()),
        _ => Err("IEmpty/empty expects one collection".into()),
    }
}

fn protocol_equality(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [left, right] => Ok(Value::Bool(left == right)),
        _ => Err("IEquality/equality expects two values".into()),
    }
}

fn protocol_display(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::Pointer(pointer)] => {
            let fallback = Value::Pointer(pointer.clone()).display();
            let rendered = (|| -> Result<String, String> {
                let runtime = pointer_default(pointer)?;
                let tags = pointer_context_eval(pointer, runtime.clone(), "tags-ptr", &[])?;
                let tags = match tags {
                    Value::Vector(values) => values.iter().cloned().collect::<Vec<_>>(),
                    Value::Tuple(values) => values.iter().cloned().collect::<Vec<_>>(),
                    Value::List(values) => values.iter().cloned().collect::<Vec<_>>(),
                    _ => return Err("IContextEval/tags-ptr must return a sequential value".into()),
                };
                let display = pointer_context_eval(pointer, runtime, "display-ptr", &[])?;
                let display = match display {
                    Value::String(value) => value,
                    value => value.display(),
                };
                let path = Value::Vector(PVector::from_iter(
                    std::iter::once(Value::Keyword(pointer.context().clone())).chain(tags),
                ));
                Ok(format!("!{}\n{}", path.display(), display))
            })()
            .unwrap_or(fallback);
            Ok(Value::String(rendered))
        }
        [value] => Ok(Value::String(value.display())),
        _ => Err("IDisplay/display expects one value".into()),
    }
}

fn protocol_encode_with(arguments: &[Value]) -> Result<Value, String> {
    let [value, visitor] = arguments else {
        return Err("IEncodable/encode-with expects a value and visitor".into());
    };
    let (method, visitor_arguments) = match value {
        Value::Nil => ("visit-nil", vec![visitor.clone()]),
        Value::Bool(_) => ("visit-boolean", vec![visitor.clone(), value.clone()]),
        Value::Number(_) | Value::Float(_) | Value::BigInteger(_) => {
            ("visit-number", vec![visitor.clone(), value.clone()])
        }
        Value::Character(_) => ("visit-character", vec![visitor.clone(), value.clone()]),
        Value::String(_) => ("visit-string", vec![visitor.clone(), value.clone()]),
        Value::Keyword(_) => ("visit-keyword", vec![visitor.clone(), value.clone()]),
        Value::Symbol(_) => ("visit-symbol", vec![visitor.clone(), value.clone()]),
        Value::List(_) | Value::Cons(_) | Value::Queue(_) | Value::Deque(_) => {
            ("visit-seq", vec![visitor.clone(), value.clone()])
        }
        Value::Vector(_) | Value::Tuple(_) => {
            ("visit-vector", vec![visitor.clone(), value.clone()])
        }
        Value::MapEntry(_) => ("visit-unknown", vec![visitor.clone(), value.clone()]),
        Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_) => ("visit-map", vec![visitor.clone(), value.clone()]),
        Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_) => {
            ("visit-set", vec![visitor.clone(), value.clone()])
        }
        Value::Tagged(tagged) => (
            "visit-tagged",
            vec![
                visitor.clone(),
                Value::Symbol(tagged.tag().clone()),
                tagged.form().clone(),
            ],
        ),
        _ => ("visit-unknown", vec![visitor.clone(), value.clone()]),
    };
    protocol_call(
        "std.protocol.iencodevisitor.IEncodeVisitor",
        method,
        &visitor_arguments,
    )
}

fn protocol_hash(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => Ok(Value::Number(value.stable_hash() as i64)),
        _ => Err("IHash/hash expects one value".into()),
    }
}

fn protocol_hash_current(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => Ok(Value::Number(value.stable_hash() as i64)),
        _ => Err("IHashCached/hash-current expects one value".into()),
    }
}

fn protocol_hash_put(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value, Value::Number(_)] => Ok(value.clone()),
        [_, _] => Err("IHashCached/hash-put expects a numeric hash".into()),
        _ => Err("IHashCached/hash-put expects two values".into()),
    }
}

fn protocol_invoke(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [callable, rest @ ..] => callable.invoke(rest.to_vec()),
        _ => Err("IFn/invoke expects a callable receiver".into()),
    }
}

impl Value {
    fn supports_native_ifn(value: &Self) -> bool {
        matches!(
            value,
            Self::Function(_)
                | Self::Keyword(_)
                | Self::Map(_)
                | Self::OrderedMap(_)
                | Self::SortedMap(_)
                | Self::Trie(_)
                | Self::PriorityMap(_)
                | Self::Set(_)
                | Self::OrderedSet(_)
                | Self::SortedSet(_)
                | Self::Pointer(_)
                | Self::StructType(_)
                | Self::MutableType(_)
        ) || mutable_map_satisfies(value)
            || mutable_set_satisfies(value)
    }
}

impl IFn<Vec<Value>> for Value {
    type Output = Result<Value, String>;

    fn invoke(&self, arguments: Vec<Value>) -> Self::Output {
        call_value(self.clone(), arguments)
    }
}

fn protocol_pair_key(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => pair_parts(value)
            .map(|(key, _)| key)
            .ok_or_else(|| "IPair/key has no implementation for this value".into()),
        _ => Err("IPair/key expects one pair".into()),
    }
}

fn protocol_pair_value(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] => pair_parts(value)
            .map(|(_, value)| value)
            .ok_or_else(|| "IPair/value has no implementation for this value".into()),
        _ => Err("IPair/value expects one pair".into()),
    }
}

fn protocol_peek_first(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] if native_protocol_supports("IPeekFirst", value) => collection_first(value.clone()),
        [_] => Err("protocol/unsupported-receiver: IPeekFirst/peek-first".into()),
        _ => Err("IPeekFirst/peek-first expects one collection".into()),
    }
}

fn protocol_peek_last(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [value] if native_protocol_supports("IPeekLast", value) => collection_last(value.clone()),
        [_] => Err("protocol/unsupported-receiver: IPeekLast/peek-last".into()),
        _ => Err("IPeekLast/peek-last expects one collection".into()),
    }
}

fn protocol_pop_first(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::MutableCollection(collection)] => {
            let mut borrowed = collection.borrow_mut();
            let mutable = borrowed
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::List(values) => {
                    values.pop_first();
                }
                MutableCollection::Queue(values) => {
                    values.pop_first();
                }
                _ => return Err("protocol/unsupported-receiver: IPopFirst/pop-first".into()),
            }
            Ok(Value::MutableCollection(collection.clone()))
        }
        [Value::List(values)] => Ok(Value::List(values.pop_first())),
        [Value::Cons(values)] => Ok(Value::List(values.clone().pop_first())),
        [Value::Tuple(values)] => Ok(Value::Tuple(Box::new(values.pop_first()))),
        [Value::Queue(values)] => Ok(Value::Queue(Box::new(values.pop_first()))),
        [Value::Deque(values)] => Ok(Value::Deque(Box::new(values.pop_first()))),
        [Value::PriorityMap(values)] => Ok(Value::PriorityMap(Box::new(values.pop_first()))),
        [value @ Value::Seq(_)] => collection_rest(value.clone()),
        [_] => Err("protocol/unsupported-receiver: IPopFirst/pop-first".into()),
        _ => Err("IPopFirst/pop-first expects one collection".into()),
    }
}

fn protocol_pop_last(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::MutableCollection(collection)] => {
            let mut borrowed = collection.borrow_mut();
            let mutable = borrowed
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::List(values) => {
                    values.pop_last();
                }
                MutableCollection::Queue(values) => {
                    values.pop_last();
                }
                MutableCollection::Vector(values) => {
                    values.pop_last();
                }
                _ => return Err("protocol/unsupported-receiver: IPopLast/pop-last".into()),
            }
            Ok(Value::MutableCollection(collection.clone()))
        }
        [Value::List(values)] => Ok(Value::List(values.pop_last())),
        [Value::Tuple(values)] => Ok(Value::Tuple(Box::new(values.pop_last()))),
        [Value::Vector(values)] => Ok(Value::Vector(values.pop_last())),
        [Value::Queue(values)] => Ok(Value::Queue(Box::new(values.pop_last()))),
        [Value::Deque(values)] => Ok(Value::Deque(Box::new(values.pop_last()))),
        [Value::PriorityMap(values)] => Ok(Value::PriorityMap(Box::new(values.pop_last()))),
        [_] => Err("protocol/unsupported-receiver: IPopLast/pop-last".into()),
        _ => Err("IPopLast/pop-last expects one collection".into()),
    }
}

fn protocol_push_first(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::MutableCollection(collection), value] => {
            let mut borrowed = collection.borrow_mut();
            let mutable = borrowed
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::List(values) => {
                    values.push_first(value.clone());
                }
                MutableCollection::Queue(values) => {
                    values.push_first(value.clone());
                }
                _ => return Err("protocol/unsupported-receiver: IPushFirst/push-first".into()),
            }
            Ok(Value::MutableCollection(collection.clone()))
        }
        [Value::List(values), value] => Ok(Value::List(values.push_first(value.clone()))),
        [Value::Cons(values), value] => Ok(Value::Cons(Box::new(
            PCons::new(value.clone(), values.to_list()).with_meta(values.meta().cloned()),
        ))),
        [Value::Tuple(values), value] => tuple_push_first(values, value.clone()),
        [Value::Deque(values), value] => {
            Ok(Value::Deque(Box::new(values.push_first(value.clone()))))
        }
        [Value::Queue(values), value] => {
            Ok(Value::Queue(Box::new(values.push_first(value.clone()))))
        }
        [_, _] => Err("protocol/unsupported-receiver: IPushFirst/push-first".into()),
        _ => Err("IPushFirst/push-first expects a collection and value".into()),
    }
}

fn protocol_push_last(arguments: &[Value]) -> Result<Value, String> {
    match arguments {
        [Value::MutableCollection(collection), value] => {
            let mut borrowed = collection.borrow_mut();
            let mutable = borrowed
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::List(values) => {
                    values.push_last(value.clone());
                }
                MutableCollection::Queue(values) => {
                    values.push_last(value.clone());
                }
                MutableCollection::Vector(values) => {
                    values.push_last(value.clone());
                }
                _ => return Err("protocol/unsupported-receiver: IPushLast/push-last".into()),
            }
            Ok(Value::MutableCollection(collection.clone()))
        }
        [Value::List(values), value] => Ok(Value::List(values.push_last(value.clone()))),
        [Value::Tuple(values), value] => tuple_push_last(values, value.clone()),
        [Value::Vector(values), value] => Ok(Value::Vector(values.push_last(value.clone()))),
        [Value::Queue(values), value] => {
            Ok(Value::Queue(Box::new(values.push_last(value.clone()))))
        }
        [Value::Deque(values), value] => {
            Ok(Value::Deque(Box::new(values.push_last(value.clone()))))
        }
        [_, _] => Err("protocol/unsupported-receiver: IPushLast/push-last".into()),
        _ => Err("IPushLast/push-last expects a collection and value".into()),
    }
}

fn protocol_cons(arguments: &[Value]) -> Result<Value, String> {
    let [collection, item] = arguments else {
        return Err("ICons/cons expects a collection and value".into());
    };
    match collection {
        Value::Cons(values) => Ok(Value::Cons(Box::new(
            PCons::new(item.clone(), values.iter().collect()).with_meta(values.meta().cloned()),
        ))),
        Value::Tuple(values) => Ok(Value::Cons(Box::new(PCons::new(
            item.clone(),
            values.iter().cloned().collect(),
        )))),
        Value::Vector(values) => Ok(Value::Cons(Box::new(PCons::new(
            item.clone(),
            values.iter().cloned().collect(),
        )))),
        Value::List(values) => Ok(Value::Cons(Box::new(PCons::new(
            item.clone(),
            values.clone(),
        )))),
        Value::Queue(values) => Ok(Value::Cons(Box::new(PCons::new(
            item.clone(),
            values.iter().cloned().collect(),
        )))),
        Value::Deque(values) => Ok(Value::Deque(Box::new(values.push_first(item.clone())))),
        Value::Nil => Ok(Value::Cons(Box::new(PCons::new(
            item.clone(),
            PList::new(),
        )))),
        Value::Seq(_) => iterator_seq(iterator_prepend(item.clone(), collection.clone())?),
        _ => Err("ICons/cons has no implementation for this value".into()),
    }
}

fn tuple_push_last(values: &PTuple<Value>, item: Value) -> Result<Value, String> {
    if values.len() < 8 {
        return Ok(Value::Tuple(Box::new(values.push_last(item)?)));
    }
    Ok(Value::Vector(
        PVector::from_iter(values.iter().cloned().chain(std::iter::once(item)))
            .with_meta(values.meta().cloned()),
    ))
}

fn tuple_push_first(values: &PTuple<Value>, item: Value) -> Result<Value, String> {
    if values.len() < 8 {
        return Ok(Value::Tuple(Box::new(values.push_first(item)?)));
    }
    Ok(Value::Vector(
        PVector::from_iter(std::iter::once(item).chain(values.iter().cloned()))
            .with_meta(values.meta().cloned()),
    ))
}

fn protocol_conj(arguments: &[Value]) -> Result<Value, String> {
    if arguments.len() != 2 {
        return Err("IConj/conj expects a collection and value".into());
    }
    let collection = &arguments[0];
    let item = &arguments[1];
    match collection {
        Value::Nil => Ok(Value::List(std::iter::once(item.clone()).collect())),
        Value::Extension(receiver) => {
            extension_protocol_call(receiver, "std.protocol.iconj.IConj", "conj", arguments)
        }
        Value::MutableCollection(collection) => {
            let mut borrowed = collection.borrow_mut();
            let mutable = borrowed
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::Set(values) => {
                    values.conj(item.clone());
                }
                MutableCollection::OrderedSet(values) => {
                    values.conj(item.clone());
                }
                MutableCollection::SortedSet(values) => {
                    values.conj(item.clone());
                }
                MutableCollection::List(values) => {
                    values.push_first(item.clone());
                }
                MutableCollection::Queue(values) => {
                    values.push_last(item.clone());
                }
                MutableCollection::Vector(values) => {
                    values.push_last(item.clone());
                }
                MutableCollection::Map(values) => {
                    let (key, value) = map_conj_parts(item)
                        .ok_or_else(|| "IConj/conj map expects a two-element entry".to_string())?;
                    values.assoc(key, value);
                }
                MutableCollection::OrderedMap(values) => {
                    let (key, value) = map_conj_parts(item)
                        .ok_or_else(|| "IConj/conj map expects a two-element entry".to_string())?;
                    values.assoc(key, value);
                }
                MutableCollection::SortedMap(values) => {
                    let (key, value) = map_conj_parts(item)
                        .ok_or_else(|| "IConj/conj map expects a two-element entry".to_string())?;
                    values.assoc(key, value);
                }
                MutableCollection::Trie(values) => {
                    let (key, value) = map_conj_parts(item)
                        .ok_or_else(|| "IConj/conj trie expects a two-element entry".to_string())?;
                    values.assoc(marker_key(&key, "trie")?, value);
                }
            }
            Ok(Value::MutableCollection(collection.clone()))
        }
        Value::Array(values) => {
            values.borrow_mut().push(item.clone());
            Ok(Value::Array(values.clone()))
        }
        Value::Object(values) => {
            let (key, value) = pair_parts(item)
                .ok_or_else(|| "IConj/conj object expects a two-element entry".to_string())?;
            let key = marker_key(&key, "IConj/conj object")?;
            let mut output = values.borrow_mut();
            if let Some((_, current)) = output.iter_mut().find(|(candidate, _)| candidate == &key) {
                *current = value;
            } else {
                output.push((key, value));
            }
            drop(output);
            Ok(Value::Object(values.clone()))
        }
        Value::Tuple(values) => tuple_push_last(values, item.clone()),
        Value::Vector(values) => {
            let output = values.push_last(item.clone());
            Ok(Value::Vector(output))
        }
        Value::Queue(values) => Ok(Value::Queue(Box::new(values.push_last(item.clone())))),
        Value::Deque(values) => Ok(Value::Deque(Box::new(values.push_last(item.clone())))),
        Value::Cons(values) => Ok(Value::Cons(Box::new(
            PCons::new(item.clone(), values.iter().collect()).with_meta(values.meta().cloned()),
        ))),
        Value::List(values) => {
            let output = std::iter::once(item.clone())
                .chain(values.iter().cloned())
                .collect();
            Ok(Value::List(output))
        }
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            set_conj_value(value, item.clone())
        }
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => {
            let (entry_key, entry_value) = map_conj_parts(item)
                .ok_or_else(|| "IConj/conj map expects a two-element entry".to_string())?;
            map_assoc_value(value, entry_key, entry_value)
        }
        _ => Err("IConj/conj expects a collection".into()),
    }
}

pub(crate) fn protocol_call(protocol: &str, method: &str, arguments: &[Value]) -> Result<Value, String> {
    let registry = ACTIVE_PROTOCOLS.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .unwrap_or_else(ProtocolRegistry::core)
    });
    registry.invoke(protocol, method, arguments)
}

pub(crate) fn protocol_intrinsic_call(
    target: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    let (protocol, method) = target
        .rsplit_once('/')
        .ok_or_else(|| format!("invalid protocol intrinsic target: {target}"))?;
    protocol_call(protocol, method, arguments)
}

fn extension_protocol_call(
    receiver: &ExtensionValue,
    protocol: &str,
    method: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    let registry = ACTIVE_PROTOCOLS.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .unwrap_or_else(ProtocolRegistry::core)
    });
    registry.invoke_extension(receiver, protocol, method, arguments)
}

fn mutable_collection_satisfies(
    value: &Value,
    predicate: impl FnOnce(&MutableCollection) -> bool,
) -> bool {
    let Value::MutableCollection(collection) = value else {
        return false;
    };
    let borrowed = collection.borrow();
    let Some(collection) = borrowed.as_ref() else {
        return false;
    };
    predicate(collection)
}

fn mutable_linear_satisfies(value: &Value, list_or_queue: bool, vector: bool) -> bool {
    mutable_collection_satisfies(value, |collection| {
        (matches!(
            collection,
            MutableCollection::List(_) | MutableCollection::Queue(_)
        ) && list_or_queue)
            || (matches!(collection, MutableCollection::Vector(_)) && vector)
    })
}

fn mutable_map_satisfies(value: &Value) -> bool {
    mutable_collection_satisfies(value, |collection| {
        matches!(
            collection,
            MutableCollection::Map(_)
                | MutableCollection::OrderedMap(_)
                | MutableCollection::SortedMap(_)
        )
    })
}

fn mutable_set_satisfies(value: &Value) -> bool {
    mutable_collection_satisfies(value, |collection| {
        matches!(
            collection,
            MutableCollection::Set(_)
                | MutableCollection::OrderedSet(_)
                | MutableCollection::SortedSet(_)
        )
    })
}

impl Value {
    fn supports_native_imaptype(value: &Self) -> bool {
        matches!(
            value,
            Self::Map(_) | Self::OrderedMap(_) | Self::SortedMap(_) | Self::PriorityMap(_)
        ) || mutable_map_satisfies(value)
    }
    fn supports_native_ilineartype(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Tuple(_)
                | Self::Vector(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_isequential(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Cons(_)
                | Self::Seq(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Tuple(_)
                | Self::Vector(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_isettype(value: &Self) -> bool {
        matches!(
            value,
            Self::Set(_) | Self::OrderedSet(_) | Self::SortedSet(_)
        ) || mutable_set_satisfies(value)
    }
    fn supports_native_icoll(value: &Self) -> bool {
        matches!(
            value,
            Self::Map(_)
                | Self::OrderedMap(_)
                | Self::SortedMap(_)
                | Self::Trie(_)
                | Self::PriorityMap(_)
                | Self::Set(_)
                | Self::OrderedSet(_)
                | Self::SortedSet(_)
                | Self::List(_)
                | Self::Cons(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Tuple(_)
                | Self::Vector(_)
                | Self::MutableCollection(_)
        )
    }
    fn supports_native_iconj(value: &Self) -> bool {
        matches!(
            value,
            Self::Nil | Self::Array(_) | Self::Object(_) | Self::MutableCollection(_)
        ) || Self::supports_native_icoll(value)
    }
    fn supports_native_icons(value: &Self) -> bool {
        matches!(
            value,
            Self::Cons(_)
                | Self::Tuple(_)
                | Self::Vector(_)
                | Self::List(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Nil
                | Self::Seq(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_iempty(value: &Self) -> bool {
        Self::supports_native_icoll(value)
            || matches!(
                value,
                Self::Nil | Self::Array(_) | Self::Object(_) | Self::Struct(_) | Self::Seq(_)
            )
    }
    fn supports_native_itomutable(value: &Self) -> bool {
        matches!(
            value,
            Self::Map(_)
                | Self::OrderedMap(_)
                | Self::SortedMap(_)
                | Self::Trie(_)
                | Self::Set(_)
                | Self::OrderedSet(_)
                | Self::SortedSet(_)
                | Self::List(_)
                | Self::Queue(_)
                | Self::Vector(_)
        )
    }
    fn supports_native_itopersistent(value: &Self) -> bool {
        matches!(value, Self::MutableCollection(_))
    }
    fn supports_native_iiter(value: &Self) -> bool {
        Self::supports_native_icoll(value)
            || matches!(
                value,
                Self::Iterator(_)
                    | Self::Nil
                    | Self::String(_)
                    | Self::Bytes(_)
                    | Self::ByteBuffer(_)
                    | Self::Array(_)
                    | Self::Object(_)
                    | Self::Struct(_)
                    | Self::Mutable(_)
                    | Self::MutableCollection(_)
                    | Self::Seq(_)
                    | Self::Pointer(_)
                    | Self::MapEntry(_)
            )
    }
    fn supports_native_ireduce(value: &Self) -> bool {
        Self::supports_native_iiter(value)
    }
    fn supports_native_ipeekfirst(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Cons(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Tuple(_)
                | Self::Vector(_)
                | Self::Seq(_)
                | Self::PriorityMap(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_ipeeklast(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Cons(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Tuple(_)
                | Self::Vector(_)
                | Self::PriorityMap(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_iiterator(value: &Self) -> bool {
        matches!(value, Self::Iterator(_))
    }
    fn supports_native_icount(value: &Self) -> bool {
        Self::supports_native_icoll(value)
            || matches!(
                value,
                Self::Seq(_)
                    | Self::String(_)
                    | Self::Bytes(_)
                    | Self::ByteBuffer(_)
                    | Self::Array(_)
                    | Self::Object(_)
                    | Self::Struct(_)
                    | Self::Mutable(_)
                    | Self::Pointer(_)
                    | Self::MutableCollection(_)
                    | Self::Iterator(_)
                    | Self::Nil
                    | Self::MapEntry(_)
            )
    }
    fn supports_native_inth(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Cons(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::Tuple(_)
                | Self::Vector(_)
                | Self::String(_)
                | Self::Bytes(_)
                | Self::ByteBuffer(_)
                | Self::Array(_)
                | Self::MapEntry(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_map(value: &Self) -> bool {
        matches!(
            value,
            Self::Map(_)
                | Self::OrderedMap(_)
                | Self::SortedMap(_)
                | Self::Trie(_)
                | Self::PriorityMap(_)
        )
    }
    fn supports_native_iassoc(value: &Self) -> bool {
            Self::supports_native_map(value)
            || matches!(
                value,
                Self::Deque(_)
                    | Self::Tuple(_)
                    | Self::Vector(_)
                    | Self::Struct(_)
                    | Self::MutableCollection(_)
            )
    }
    fn supports_native_idissoc(value: &Self) -> bool {
        Self::supports_native_map(value)
            || matches!(
                value,
                Self::Set(_)
                    | Self::OrderedSet(_)
                    | Self::SortedSet(_)
                    | Self::Struct(_)
                    | Self::MutableCollection(_)
            )
    }
    fn supports_native_ifind(value: &Self) -> bool {
        Self::supports_native_map(value)
            || matches!(
                value,
                Self::Set(_)
                    | Self::OrderedSet(_)
                    | Self::SortedSet(_)
                    | Self::List(_)
                    | Self::Cons(_)
                    | Self::Queue(_)
                    | Self::Deque(_)
                    | Self::Array(_)
                    | Self::Vector(_)
                    | Self::Tuple(_)
                    | Self::Seq(_)
                    | Self::Pointer(_)
                    | Self::Object(_)
                    | Self::Struct(_)
                    | Self::Mutable(_)
                    | Self::MutableCollection(_)
            )
    }
    fn supports_native_ilookup(value: &Self) -> bool {
        matches!(
            value,
            Self::Nil
                | Self::Array(_)
                | Self::Bytes(_)
                | Self::ByteBuffer(_)
                | Self::Cons(_)
                | Self::Deque(_)
                | Self::List(_)
                | Self::Queue(_)
                | Self::Seq(_)
                | Self::String(_)
                | Self::Set(_)
                | Self::OrderedSet(_)
                | Self::SortedSet(_)
                | Self::Result(_)
                | Self::MapEntry(_)
        )
            || Self::supports_native_map(value)
            || matches!(
                value,
                Self::Object(_)
                    | Self::Vector(_)
                    | Self::Tuple(_)
                    | Self::Pointer(_)
                    | Self::Struct(_)
                    | Self::Mutable(_)
            )
            || mutable_map_satisfies(value)
    }
    fn supports_native_ideref(value: &Self) -> bool {
        matches!(
            value,
            Self::Atom(_)
                | Self::Promise(_)
                | Self::Var(_)
                | Self::Result(_)
                | Self::Pointer(_)
                | Self::Schema(_)
        )
    }
    fn supports_native_idereftimeout(value: &Self) -> bool {
        matches!(value, Self::Promise(_) | Self::Atom(_) | Self::Var(_))
    }
    fn supports_native_ireset(value: &Self) -> bool {
        matches!(value, Self::Atom(_))
    }
    fn supports_native_icas(value: &Self) -> bool {
        matches!(value, Self::Atom(_))
    }
    fn supports_native_iwatch(value: &Self) -> bool {
        matches!(value, Self::Atom(_))
    }
    fn supports_native_ipointer(value: &Self) -> bool {
        matches!(value, Self::Pointer(_))
    }
    fn supports_native_iapplicable(value: &Self) -> bool {
        Self::supports_native_ipointer(value)
    }
    fn supports_native_ipair(value: &Self) -> bool {
        pair_parts(value).is_some()
    }
    fn supports_native_iobjtype(value: &Self) -> bool {
        Self::supports_native_icoll(value)
            || matches!(
                value,
                Self::Symbol(_)
                    | Self::Keyword(_)
                    | Self::Pointer(_)
                    | Self::Seq(_)
                    | Self::Var(_)
                    | Self::Function(_)
                    | Self::Struct(_)
                    | Self::Mutable(_)
                    | Self::NativeType(_)
                    | Self::MapEntry(_)
            )
    }
    fn supports_native_istringlike(value: &Self) -> bool {
        matches!(
            value,
            Self::Keyword(_) | Self::Symbol(_)
        )
    }
    fn supports_native_inamespaced(value: &Self) -> bool {
        matches!(
            value,
            Self::Keyword(_) | Self::Symbol(_) | Self::Var(_) | Self::NativeType(_)
        )
    }
    fn supports_native_ipushfirst(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_) | Self::Cons(_) | Self::Tuple(_) | Self::Queue(_) | Self::Deque(_)
        ) || mutable_linear_satisfies(value, true, false)
    }
    fn supports_native_ipushlast(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_) | Self::Tuple(_) | Self::Vector(_) | Self::Queue(_) | Self::Deque(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_ipopfirst(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Cons(_)
                | Self::Tuple(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::PriorityMap(_)
                | Self::Seq(_)
        ) || mutable_linear_satisfies(value, true, false)
    }
    fn supports_native_ipoplast(value: &Self) -> bool {
        matches!(
            value,
            Self::List(_)
                | Self::Tuple(_)
                | Self::Vector(_)
                | Self::Queue(_)
                | Self::Deque(_)
                | Self::PriorityMap(_)
        ) || mutable_linear_satisfies(value, true, true)
    }
    fn supports_native_imutable(value: &Self) -> bool {
        matches!(value, Self::Mutable(_) | Self::MutableCollection(_))
    }
    fn supports_native_ipersistent(value: &Self) -> bool {
        (Self::supports_native_icoll(value) && !matches!(value, Self::MutableCollection(_)))
            || matches!(value, Self::Struct(_) | Self::MapEntry(_))
    }
    fn supports_native_iequality(value: &Self) -> bool {
        !matches!(value, Self::Protocol(_))
    }
    fn supports_native_idisplay(value: &Self) -> bool {
        !matches!(value, Self::Protocol(_))
    }
    fn supports_native_iencodable(_: &Self) -> bool {
        true
    }
    fn supports_native_iexinfo(value: &Self) -> bool {
        matches!(value, Self::ExceptionInfo(_))
    }
    fn supports_native_ihash(value: &Self) -> bool {
        Self::supports_native_iobjtype(value)
            || matches!(value, Self::Bytes(_) | Self::MutableCollection(_))
    }
    fn supports_native_ihashcached(value: &Self) -> bool {
        Self::supports_native_icoll(value)
            || matches!(value, Self::Symbol(_) | Self::Struct(_) | Self::MapEntry(_))
    }
    fn supports_native_ipromise(value: &Self) -> bool {
        matches!(value, Self::Promise(_))
    }
    fn supports_native_icoroutine(value: &Self) -> bool {
        matches!(value, Self::Coroutine(_))
    }
    fn supports_native_istream(value: &Self) -> bool {
        matches!(value, Self::Stream(_))
    }
    fn supports_native_iclose(value: &Self) -> bool {
        matches!(
            value,
            Self::Stream(_) | Self::Coroutine(_) | Self::Iterator(_)
        )
    }
}

fn native_protocol_supports(protocol: &str, value: &Value) -> bool {
    let name = protocol
        .rsplit(|character| character == '/' || character == '.')
        .next()
        .unwrap_or(protocol);
    match name {
        "IColl" => Value::supports_native_icoll(value),
        "ISequential" => Value::supports_native_isequential(value),
        "IMapType" => Value::supports_native_imaptype(value),
        "ILinearType" => Value::supports_native_ilineartype(value),
        "ISetType" => Value::supports_native_isettype(value),
        "IMetadata" => Value::supports_native_iobjtype(value),
        "IConj" => Value::supports_native_iconj(value),
        "ICons" => Value::supports_native_icons(value),
        "IEmpty" => Value::supports_native_iempty(value),
        "IToMutable" => Value::supports_native_itomutable(value),
        "IToPersistent" => Value::supports_native_itopersistent(value),
        "IIter" => Value::supports_native_iiter(value),
        "IReduce" => Value::supports_native_ireduce(value),
        "IPeekFirst" => Value::supports_native_ipeekfirst(value),
        "IPeekLast" => Value::supports_native_ipeeklast(value),
        "IIterator" => Value::supports_native_iiterator(value),
        "ICount" => Value::supports_native_icount(value),
        "INth" => Value::supports_native_inth(value),
        "IAssoc" => Value::supports_native_iassoc(value),
        "IDissoc" => Value::supports_native_idissoc(value),
        "IFind" => Value::supports_native_ifind(value),
        "ILookup" => Value::supports_native_ilookup(value),
        "IDeref" => Value::supports_native_ideref(value),
        "IDerefTimeout" => Value::supports_native_idereftimeout(value),
        "IReset" => Value::supports_native_ireset(value),
        "ICas" => Value::supports_native_icas(value),
        "IWatch" => Value::supports_native_iwatch(value),
        "IFn" => Value::supports_native_ifn(value),
        "IPointer" => Value::supports_native_ipointer(value),
        "IApplicable" => Value::supports_native_iapplicable(value),
        "IPair" => Value::supports_native_ipair(value),
        "IObjType" => Value::supports_native_iobjtype(value),
        "IStringLike" => Value::supports_native_istringlike(value),
        "INamespaced" => Value::supports_native_inamespaced(value),
        "IPushFirst" => Value::supports_native_ipushfirst(value),
        "IPushLast" => Value::supports_native_ipushlast(value),
        "IPopFirst" => Value::supports_native_ipopfirst(value),
        "IPopLast" => Value::supports_native_ipoplast(value),
        "IMutable" => Value::supports_native_imutable(value),
        "IPersistent" => Value::supports_native_ipersistent(value),
        "IEquality" => Value::supports_native_iequality(value),
        "IDisplay" => Value::supports_native_idisplay(value),
        "IEncodable" => Value::supports_native_iencodable(value),
        "IExInfo" => Value::supports_native_iexinfo(value),
        "IHash" => Value::supports_native_ihash(value),
        "IHashCached" => Value::supports_native_ihashcached(value),
        "IPromise" => Value::supports_native_ipromise(value),
        "ICoroutine" => Value::supports_native_icoroutine(value),
        "IStream" => Value::supports_native_istream(value),
        "IClose" => Value::supports_native_iclose(value),
        _ => false,
    }
}

fn protocol_satisfies(protocol: &GuestProtocol, value: &Value) -> bool {
    let registry = ACTIVE_PROTOCOLS.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .unwrap_or_else(ProtocolRegistry::core)
    });
    registry.satisfies(protocol, value)
}

fn promise_state_value(promise: &Promise) -> Value {
    Value::Keyword(
        match promise.state() {
            PromiseState::Pending => "pending",
            PromiseState::Fulfilled(_) => "fulfilled",
            PromiseState::Rejected(error) if error.is_cancelled() => "cancelled",
            PromiseState::Rejected(_) => "rejected",
        }
        .into(),
    )
}

fn promise_value_result(promise: &Promise) -> Result<Value, String> {
    match promise.state() {
        PromiseState::Pending => Err("promise is pending".into()),
        PromiseState::Fulfilled(value) => Ok(value),
        PromiseState::Rejected(error) => Err(promise_rejection_error(error)),
    }
}

fn promise_from(value: Value) -> Promise {
    match value {
        Value::Promise(promise) => promise,
        value => {
            let promise = Promise::new();
            promise.resolve(value);
            promise
        }
    }
}

fn promise_all(values: Vec<Value>) -> Promise {
    let output = Promise::new();
    if values.is_empty() {
        output.resolve(Value::Array(Rc::new(RefCell::new(Vec::new()))));
        return output;
    }
    let count = values.len();
    let remaining = Rc::new(Cell::new(count));
    let results = Rc::new(RefCell::new(vec![Value::Nil; count]));
    let mut sources = Vec::with_capacity(count);
    for (index, value) in values.into_iter().enumerate() {
        let source = match value {
            Value::Promise(promise) => promise,
            value => {
                let promise = Promise::new();
                promise.resolve(value);
                promise
            }
        };
        sources.push(source.clone());
        let destination = output.clone();
        let remaining = remaining.clone();
        let results = results.clone();
        source.on_settle(Rc::new(move |state| match state {
            PromiseState::Fulfilled(value) => {
                results.borrow_mut()[index] = value;
                let left = remaining.get() - 1;
                remaining.set(left);
                if left == 0 {
                    destination.resolve(Value::Array(Rc::new(RefCell::new(
                        results.borrow().clone(),
                    ))));
                }
            }
            PromiseState::Rejected(error) => {
                destination.reject_rejection(error);
            }
            PromiseState::Pending => {}
        }));
    }
    let poll_sources = sources.clone();
    output.set_poller(Rc::new(move || {
        for source in &poll_sources {
            source.state();
        }
    }));
    output.set_waiter(Rc::new(move || {
        for source in &sources {
            source.wait_state();
        }
    }));
    output
}
fn settle_promise_result(destination: &Promise, result: Result<Value, String>) {
    match result {
        Ok(Value::Promise(source)) => {
            destination.adopt(&source);
        }
        Ok(value) => {
            destination.resolve(value);
        }
        Err(error) => {
            destination.reject(error);
        }
    }
}

fn finish_promise(destination: Promise, original: PromiseState, cleanup: Result<Value, String>) {
    let preserved_destination = destination.clone();
    let preserve = move || match original.clone() {
        PromiseState::Fulfilled(value) => {
            preserved_destination.resolve(value);
        }
        PromiseState::Rejected(error) => {
            preserved_destination.reject_rejection(error);
        }
        PromiseState::Pending => {}
    };
    match cleanup {
        Ok(Value::Promise(cleanup)) => {
            cleanup.on_settle(Rc::new(move |state| match state {
                PromiseState::Fulfilled(_) => preserve(),
                PromiseState::Rejected(error) => {
                    destination.reject_rejection(error);
                }
                PromiseState::Pending => {}
            }));
        }
        Ok(_) => preserve(),
        Err(error) => {
            destination.reject(error);
        }
    }
}

fn promise_chain(source: Promise, operation: &str, function: Rc<Function>) -> Promise {
    let output = Promise::new();
    let poll_source = source.clone();
    output.set_poller(Rc::new(move || {
        poll_source.state();
    }));
    let wait_source = source.clone();
    output.set_waiter(Rc::new(move || {
        wait_source.wait_state();
    }));
    let operation = operation.to_string();
    let destination = output.clone();
    let context = crate::core::NativeCallbackContext::capture();
    source.on_settle(Rc::new(move |state| {
        context.with(|| match state.clone() {
            PromiseState::Fulfilled(value) if operation == "promise/then" => {
                settle_promise_result(&destination, call_function(&function, vec![value]));
            }
            PromiseState::Rejected(error) if operation == "promise/catch" => {
                settle_promise_result(&destination, call_function(&function, vec![error.value()]));
            }
            PromiseState::Fulfilled(_) | PromiseState::Rejected(_)
                if operation == "promise/finally" =>
            {
                finish_promise(
                    destination.clone(),
                    state,
                    call_function(&function, Vec::new()),
                );
            }
            PromiseState::Fulfilled(value) => {
                destination.resolve(value);
            }
            PromiseState::Rejected(error) => {
                destination.reject_rejection(error);
            }
            PromiseState::Pending => {}
        })
    }));
    output
}

#[cfg(test)]
mod protocol_tests {
    use super::{
        call_value, native_base_values, native_function, native_variadic_function, protocol_call,
        protocol_display, with_namespace_registry, with_protocols, NamespaceRegistry, PMap,
        ProtocolRegistry, Symbol, Value,
    };

    #[test]
    fn native_base_bytes_constructs_a_byte_buffer() {
        let value = native_base_values(
            "Base/bytes",
            &[Value::Number(1), Value::Number(2), Value::Number(-3)],
        )
        .unwrap();
        let Value::ByteBuffer(bytes) = value else {
            panic!("Base/bytes did not return a byte buffer");
        };
        assert_eq!(*bytes.borrow(), vec![1, 2, 253]);
    }

    #[test]
    fn native_base_map_entry_constructs_the_dedicated_pair_type() {
        let value = native_base_values(
            "Base/map-entry",
            &[Value::Keyword("key".into()), Value::Number(42)],
        )
        .unwrap();
        assert!(matches!(value, Value::MapEntry(_)));
        assert_eq!(value.display(), "[:key 42]");
    }

    #[test]
    fn native_base_declaration_abi_uses_explicit_namespace_values() {
        let namespaces = NamespaceRegistry::new("user");
        let protocols = ProtocolRegistry::new();
        with_namespace_registry(&namespaces, || {
            with_protocols(&protocols, || {
                let namespace = native_base_values(
                    "Base/namespace",
                    &[Value::Symbol(Symbol::parse("example.native"))],
                )
                .expect("namespace");
                assert!(matches!(&namespace, Value::Namespace(value) if value.name().as_str() == "example.native"));

                let defined = native_base_values(
                    "Base/def",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("answer")),
                        Value::Number(42),
                        Value::Nil,
                    ],
                )
                .expect("def");
                assert!(matches!(defined, Value::Var(_)));
                let resolved = native_base_values(
                    "Base/resolve",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("answer")),
                    ],
                )
                .expect("resolve");
                assert!(matches!(resolved, Value::Var(var) if var.deref_value() == Value::Number(42)));

                let user_type = native_base_values(
                    "Base/struct",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("User")),
                        Value::Vector(vec![Value::Symbol(Symbol::parse("name"))].into()),
                    ],
                )
                .expect("struct");
                assert!(matches!(&user_type, Value::StructType(value) if value.name == "example.native/User"));

                let session_type = native_base_values(
                    "Base/mutable",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("Session")),
                        Value::Vector(vec![Value::Symbol(Symbol::parse("token"))].into()),
                    ],
                )
                .expect("mutable");
                assert!(matches!(&session_type, Value::MutableType(value) if value.name == "example.native/Session"));

                let target = namespaces.find("example.native").expect("declared namespace");
                let user_constructor = target
                    .resolve(&Symbol::parse("->User"))
                    .expect("struct constructor")
                    .deref_value();
                assert!(matches!(user_constructor, Value::StructType(_)));
                let user = call_value(user_constructor, vec![Value::String("Ada".into())])
                    .expect("construct user");

                let session_constructor = target
                    .resolve(&Symbol::parse("->Session"))
                    .expect("mutable constructor")
                    .deref_value();
                assert!(matches!(session_constructor, Value::MutableType(_)));
                let session = call_value(
                    session_constructor,
                    vec![Value::String("session-token".into())],
                )
                .expect("construct session");
                assert_eq!(
                    native_base_values(
                        "Base/field",
                        &[session, Value::Keyword("token".into())],
                    )
                    .expect("field"),
                    Value::String("session-token".into())
                );

                let protocol = native_base_values(
                    "Base/protocol",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("IGreeting")),
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("greet")),
                            Value::Number(1),
                        )])),
                        Value::Vector(Vec::<Value>::new().into()),
                    ],
                )
                .expect("protocol");
                let greeting = native_variadic_function("example.native/greet", |_| {
                    Ok(Value::String("hello Ada".into()))
                });
                native_base_values(
                    "Base/extend",
                    &[
                        namespace.clone(),
                        user_type.clone(),
                        protocol,
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("greet")),
                            greeting,
                        )])),
                    ],
                )
                .expect("extend");
                assert_eq!(
                    protocol_call("example.native.IGreeting", "greet", &[user])
                        .expect("guest protocol dispatch"),
                    Value::String("hello Ada".into())
                );

                native_base_values(
                    "Base/protocol",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("IGreeting")),
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("welcome")),
                            Value::Number(1),
                        )])),
                        Value::Vector(Vec::<Value>::new().into()),
                    ],
                )
                .expect("protocol reload");
                assert!(matches!(
                    native_base_values(
                        "Base/resolve",
                        &[namespace, Value::Symbol(Symbol::parse("greet"))],
                    )
                    .expect("resolve retired method"),
                    Value::Nil
                ));
            });
        });
    }

    #[test]
    fn foundation_declaration_names_are_not_native_syntax_forms() {
        for name in [
            "defstruct",
            "defmutable",
            "defprotocol",
            "extend-type",
            "defmulti",
            "defmethod",
        ] {
            assert!(
                !crate::core::syntax_symbol(name),
                "{name} must resolve through std.foundation macros"
            );
        }
    }

    #[test]
    fn native_base_declarations_validate_arity_parents_multimethods_and_transactions() {
        let namespaces = NamespaceRegistry::new("user");
        let protocols = ProtocolRegistry::new();
        with_namespace_registry(&namespaces, || {
            with_protocols(&protocols, || {
                let namespace = native_base_values(
                    "Base/namespace",
                    &[Value::Symbol(Symbol::parse("example.declaration"))],
                )
                .expect("namespace");
                let value_type = native_base_values(
                    "Base/struct",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("Value")),
                        Value::Vector(Vec::<Value>::new().into()),
                    ],
                )
                .expect("type");
                let parent = native_base_values(
                    "Base/protocol",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("IParent")),
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("parent")),
                            Value::Number(1),
                        )])),
                        Value::Vector(Vec::<Value>::new().into()),
                    ],
                )
                .expect("parent protocol");
                let child = native_base_values(
                    "Base/protocol",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("IChild")),
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("child")),
                            Value::Number(1),
                        )])),
                        Value::Vector(vec![parent.clone()].into()),
                    ],
                )
                .expect("child protocol");

                let bad_arity = native_function("example.declaration/bad", 0, |_| Ok(Value::Nil));
                assert!(native_base_values(
                    "Base/extend",
                    &[
                        namespace.clone(),
                        value_type.clone(),
                        parent.clone(),
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("parent")),
                            bad_arity,
                        )])),
                    ],
                )
                .is_err());

                let parent_implementation =
                    native_function("example.declaration/parent", 1, |_| Ok(Value::Keyword("parent".into())));
                let child_implementation =
                    native_function("example.declaration/child", 1, |_| Ok(Value::Keyword("child".into())));
                native_base_values(
                    "Base/extend",
                    &[
                        namespace.clone(),
                        value_type.clone(),
                        parent,
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("parent")),
                            parent_implementation,
                        )])),
                    ],
                )
                .expect("parent extension");
                native_base_values(
                    "Base/extend",
                    &[
                        namespace.clone(),
                        value_type.clone(),
                        child.clone(),
                        Value::Map(PMap::from_iter([(
                            Value::Symbol(Symbol::parse("child")),
                            child_implementation,
                        )])),
                    ],
                )
                .expect("child extension");
                let value = call_value(value_type.clone(), Vec::new()).expect("value");
                assert_eq!(
                    native_base_values("Base/satisfies?", &[child, value])
                        .expect("child satisfaction"),
                    Value::Bool(true)
                );

                let classify = native_base_values(
                    "Base/multimethod",
                    &[
                        namespace.clone(),
                        Value::Symbol(Symbol::parse("classify")),
                        native_function("example.declaration/dispatch", 1, |arguments| {
                            Ok(arguments[0].clone())
                        }),
                    ],
                )
                .expect("multimethod");
                assert_eq!(
                    native_base_values(
                        "Base/method",
                        &[
                            namespace.clone(),
                            Value::Symbol(Symbol::parse("classify")),
                            Value::Keyword("ok".into()),
                            native_function("example.declaration/ok", 1, |_| Ok(Value::Number(42))),
                        ],
                    )
                    .expect("method"),
                    Value::Nil
                );
                assert_eq!(
                    call_value(classify, vec![Value::Keyword("ok".into())])
                        .expect("multimethod dispatch"),
                    Value::Number(42)
                );

                let rollback_namespace = namespace.clone();
                let failing = native_function("example.declaration/failing", 0, move |_| {
                    native_base_values(
                        "Base/struct",
                        &[
                            rollback_namespace.clone(),
                            Value::Symbol(Symbol::parse("Transient")),
                            Value::Vector(Vec::<Value>::new().into()),
                        ],
                    )?;
                    Err("fixture failure".into())
                });
                assert!(native_base_values(
                    "Base/with-declaration",
                    &[namespace.clone(), failing],
                )
                .is_err());
                assert_eq!(
                    native_base_values(
                        "Base/resolve",
                        &[namespace, Value::Symbol(Symbol::parse("Transient"))],
                    )
                    .expect("resolve rollback"),
                    Value::Nil
                );
            });
        });
    }

    #[test]
    fn idisplay_renders_characters() {
        assert_eq!(
            protocol_display(&[Value::Character('a')]).unwrap(),
            Value::String("\\a".into())
        );
        assert_eq!(
            protocol_display(&[Value::Character(' ')]).unwrap(),
            Value::String("\\space".into())
        );
        assert_eq!(
            protocol_display(&[Value::Character('\n')]).unwrap(),
            Value::String("\\newline".into())
        );
    }
}
