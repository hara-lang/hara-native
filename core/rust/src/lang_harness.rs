//! Process-local host values for the Hara-owned `std.lang` compiler pipeline.
//!
//! The host stores identity, immutable configuration maps, and reversible lifecycle state.
//! It never runs grammar callbacks or emits code; those responsibilities remain in `lang.*` HAL.

use crate::core::{map_entries, native_variadic_function, ExtensionValue, Value};
use crate::lang::data::{Keyword, Map as PMap, Vector as PVector};
use std::cell::RefCell;
use std::collections::HashMap;

const PROVIDER: &str = "std.lang";

#[derive(Clone)]
enum SnapshotKind {
    Library,
    Harness,
}

#[derive(Clone)]
enum Record {
    Immutable {
        type_name: String,
        data: Value,
    },
    Library {
        config: Value,
        books: HashMap<String, Value>,
        revision: i64,
    },
    Snapshot {
        kind: SnapshotKind,
        owner: u64,
        books: HashMap<String, Value>,
        library_revision: i64,
        runtime_closed: bool,
        runtime_revision: i64,
        harness_closed: bool,
    },
    Runtime {
        config: Value,
        closed: bool,
        revision: i64,
    },
    Harness {
        config: Value,
        library: u64,
        runtime: u64,
        closed: bool,
    },
}

#[derive(Default)]
struct State {
    next: u64,
    records: HashMap<u64, Record>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Returns one directly callable host function for an annotated `std.lang` method.
pub fn function(type_name: &str, method: &str) -> Result<Value, String> {
    let type_name = type_name.to_owned();
    let method = method.to_owned();
    let display = format!("std.lang.{type_name}/{method}");
    Ok(native_variadic_function(&display, move |values| {
        invoke(&type_name, &method, values)
    }))
}

fn invoke(type_name: &str, method: &str, values: Vec<Value>) -> Result<Value, String> {
    match type_name {
        "BookMeta" | "BookEntry" | "BookModule" | "Book" | "Compilation" => {
            immutable(type_name, method, values, "data")
        }
        "Compiler" => immutable(type_name, method, values, "config"),
        "Library" => library(method, values),
        "Snapshot" => snapshot(method, values),
        "Runtime" => runtime(method, values),
        "Harness" => harness(method, values),
        _ => Err(failure(type_name, method, "is not installed")),
    }
}

fn immutable(
    type_name: &str,
    method: &str,
    values: Vec<Value>,
    accessor: &str,
) -> Result<Value, String> {
    match method {
        "create" => {
            require_arity(type_name, method, &values, 1)?;
            let data = config(&values[0], &format!("{type_name}/create"))?;
            Ok(allocate(Record::Immutable {
                type_name: type_name.into(),
                data,
            }, type_name))
        }
        value if value == accessor => {
            require_arity(type_name, method, &values, 1)?;
            let handle = handle(&values[0], type_name, method)?;
            STATE.with(|state| match state.borrow().records.get(&handle) {
                Some(Record::Immutable { type_name: actual, data }) if actual == type_name => {
                    Ok(data.clone())
                }
                _ => Err(failure(type_name, method, "expects its matching std.lang value")),
            })
        }
        _ => Err(failure(type_name, method, "is not installed")),
    }
}

fn library(method: &str, values: Vec<Value>) -> Result<Value, String> {
    match method {
        "create" => {
            require_arity("Library", method, &values, 1)?;
            Ok(allocate(
                Record::Library {
                    config: config(&values[0], "Library/create")?,
                    books: HashMap::new(),
                    revision: 0,
                },
                "Library",
            ))
        }
        "config" => {
            require_arity("Library", method, &values, 1)?;
            let handle = handle(&values[0], "Library", method)?;
            with_library(handle, method, |config, _, _| Ok(config.clone()))
        }
        "install" => {
            require_arity("Library", method, &values, 2)?;
            let library = handle(&values[0], "Library", method)?;
            let book = handle(&values[1], "Book", method)?;
            let key = immutable_data(book, "Book", method).and_then(|data| book_key(&data, method))?;
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                let Some(Record::Library { books, revision, .. }) = state.records.get_mut(&library) else {
                    return Err(failure("Library", method, "expects a std.lang.Library value"));
                };
                books.insert(key, values[1].clone());
                *revision += 1;
                Ok(values[0].clone())
            })
        }
        "remove" => {
            require_arity("Library", method, &values, 2)?;
            let library = handle(&values[0], "Library", method)?;
            let key = book_key_argument(&values[1], method)?;
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                let Some(Record::Library { books, revision, .. }) = state.records.get_mut(&library) else {
                    return Err(failure("Library", method, "expects a std.lang.Library value"));
                };
                let removed = books.remove(&key).unwrap_or(Value::Nil);
                if !matches!(removed, Value::Nil) {
                    *revision += 1;
                }
                Ok(removed)
            })
        }
        "resolve" => {
            require_arity("Library", method, &values, 2)?;
            let library = handle(&values[0], "Library", method)?;
            let key = book_key_argument(&values[1], method)?;
            with_library(library, method, |_, books, _| {
                Ok(books.get(&key).cloned().unwrap_or(Value::Nil))
            })
        }
        "books" => {
            require_arity("Library", method, &values, 1)?;
            let library = handle(&values[0], "Library", method)?;
            with_library(library, method, |_, books, _| {
                Ok(Value::Vector(PVector::from_iter(books.values().cloned())))
            })
        }
        "snapshot" => {
            require_arity("Library", method, &values, 1)?;
            let library = handle(&values[0], "Library", method)?;
            let (books, revision) = with_library(library, method, |_, books, revision| {
                Ok((books.clone(), *revision))
            })?;
            Ok(allocate(
                Record::Snapshot {
                    kind: SnapshotKind::Library,
                    owner: library,
                    books,
                    library_revision: revision,
                    runtime_closed: false,
                    runtime_revision: 0,
                    harness_closed: false,
                },
                "Snapshot",
            ))
        }
        "restore" => {
            require_arity("Library", method, &values, 2)?;
            let library = handle(&values[0], "Library", method)?;
            let snapshot = handle(&values[1], "Snapshot", method)?;
            let (books, revision) = snapshot_library(snapshot, library, method)?;
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                let Some(Record::Library { books: target, revision: target_revision, .. }) = state.records.get_mut(&library) else {
                    return Err(failure("Library", method, "expects a std.lang.Library value"));
                };
                *target = books;
                *target_revision = revision;
                Ok(values[0].clone())
            })
        }
        "reset" => {
            require_arity("Library", method, &values, 1)?;
            let library = handle(&values[0], "Library", method)?;
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                let Some(Record::Library { books, revision, .. }) = state.records.get_mut(&library) else {
                    return Err(failure("Library", method, "expects a std.lang.Library value"));
                };
                books.clear();
                *revision = 0;
                Ok(values[0].clone())
            })
        }
        "state" => {
            require_arity("Library", method, &values, 1)?;
            let library = handle(&values[0], "Library", method)?;
            with_library(library, method, |_, books, revision| {
                Ok(map(vec![
                    ("revision", Value::Number(*revision)),
                    ("book-count", Value::Number(books.len() as i64)),
                    ("books", Value::Vector(PVector::from_iter(books.values().cloned()))),
                ]))
            })
        }
        _ => Err(failure("Library", method, "is not installed")),
    }
}

fn snapshot(method: &str, values: Vec<Value>) -> Result<Value, String> {
    if method != "data" {
        return Err(failure("Snapshot", method, "is not installed"));
    }
    require_arity("Snapshot", method, &values, 1)?;
    let snapshot = handle(&values[0], "Snapshot", method)?;
    STATE.with(|state| match state.borrow().records.get(&snapshot) {
        Some(Record::Snapshot { kind, books, library_revision, runtime_closed, runtime_revision, harness_closed, .. }) => Ok(map(vec![
            ("kind", Value::Keyword(Keyword::from(match kind { SnapshotKind::Library => "library", SnapshotKind::Harness => "harness" }))),
            ("library-revision", Value::Number(*library_revision)),
            ("books", Value::Vector(PVector::from_iter(books.values().cloned()))),
            ("runtime-closed?", Value::Bool(*runtime_closed)),
            ("runtime-revision", Value::Number(*runtime_revision)),
            ("harness-closed?", Value::Bool(*harness_closed)),
        ])),
        _ => Err(failure("Snapshot", method, "expects a std.lang.Snapshot value")),
    })
}

fn runtime(method: &str, values: Vec<Value>) -> Result<Value, String> {
    match method {
        "create" => {
            require_arity("Runtime", method, &values, 1)?;
            Ok(allocate(Record::Runtime { config: config(&values[0], "Runtime/create")?, closed: false, revision: 0 }, "Runtime"))
        }
        "config" => runtime_config(method, values),
        "state" => runtime_state(method, values),
        "reset" => runtime_mutate(method, values, false, true),
        "close" => runtime_mutate(method, values, true, false),
        "closed?" => {
            require_arity("Runtime", method, &values, 1)?;
            let runtime = handle(&values[0], "Runtime", method)?;
            with_runtime(runtime, method, |_, closed, _| Ok(Value::Bool(*closed)))
        }
        _ => Err(failure("Runtime", method, "is not installed")),
    }
}

fn runtime_config(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Runtime", method, &values, 1)?;
    let runtime = handle(&values[0], "Runtime", method)?;
    with_runtime(runtime, method, |config, _, _| Ok(config.clone()))
}

fn runtime_state(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Runtime", method, &values, 1)?;
    let runtime = handle(&values[0], "Runtime", method)?;
    with_runtime(runtime, method, |_, closed, revision| {
        Ok(runtime_state_value(*closed, *revision))
    })
}

fn runtime_mutate(method: &str, values: Vec<Value>, close: bool, reset: bool) -> Result<Value, String> {
    require_arity("Runtime", method, &values, 1)?;
    let runtime = handle(&values[0], "Runtime", method)?;
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(Record::Runtime { closed, revision, .. }) = state.records.get_mut(&runtime) else {
            return Err(failure("Runtime", method, "expects a std.lang.Runtime value"));
        };
        if close { *closed = true; }
        if reset { *closed = false; *revision = 0; }
        Ok(values[0].clone())
    })
}

fn harness(method: &str, values: Vec<Value>) -> Result<Value, String> {
    match method {
        "create" => {
            require_arity("Harness", method, &values, 1)?;
            let config = config(&values[0], "Harness/create")?;
            let library = optional_handle(&config, "library", "Library", method)?
                .unwrap_or_else(|| allocate(Record::Library { config: empty_map(), books: HashMap::new(), revision: 0 }, "Library").extension_handle().unwrap());
            let runtime = optional_handle(&config, "runtime", "Runtime", method)?
                .unwrap_or_else(|| allocate(Record::Runtime { config: empty_map(), closed: false, revision: 0 }, "Runtime").extension_handle().unwrap());
            Ok(allocate(Record::Harness { config, library, runtime, closed: false }, "Harness"))
        }
        "config" => {
            require_arity("Harness", method, &values, 1)?;
            let harness = handle(&values[0], "Harness", method)?;
            with_harness(harness, method, |config, _, _, _| Ok(config.clone()))
        }
        "library" => {
            require_arity("Harness", method, &values, 1)?;
            let harness = handle(&values[0], "Harness", method)?;
            with_harness(harness, method, |_, library, _, _| Ok(extension("Library", *library)))
        }
        "runtime" => {
            require_arity("Harness", method, &values, 1)?;
            let harness = handle(&values[0], "Harness", method)?;
            with_harness(harness, method, |_, _, runtime, _| Ok(extension("Runtime", *runtime)))
        }
        "snapshot" => harness_snapshot(method, values),
        "restore" => harness_restore(method, values),
        "reset" => harness_reset(method, values),
        "close" => harness_close(method, values),
        "closed?" => {
            require_arity("Harness", method, &values, 1)?;
            let harness = handle(&values[0], "Harness", method)?;
            with_harness(harness, method, |_, _, _, closed| Ok(Value::Bool(*closed)))
        }
        "state" => harness_state(method, values),
        _ => Err(failure("Harness", method, "is not installed")),
    }
}

fn harness_snapshot(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Harness", method, &values, 1)?;
    let harness = handle(&values[0], "Harness", method)?;
    let (_, library, runtime, closed) = with_harness(harness, method, |config, library, runtime, closed| Ok((config.clone(), *library, *runtime, *closed)))?;
    let (books, library_revision) = with_library(library, method, |_, books, revision| Ok((books.clone(), *revision)))?;
    let (_, runtime_closed, runtime_revision) = with_runtime(runtime, method, |config, closed, revision| Ok((config.clone(), *closed, *revision)))?;
    Ok(allocate(Record::Snapshot { kind: SnapshotKind::Harness, owner: harness, books, library_revision, runtime_closed, runtime_revision, harness_closed: closed }, "Snapshot"))
}

fn harness_restore(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Harness", method, &values, 2)?;
    let harness = handle(&values[0], "Harness", method)?;
    let snapshot = handle(&values[1], "Snapshot", method)?;
    let (books, library_revision, runtime_closed, runtime_revision, harness_closed) = snapshot_harness(snapshot, harness, method)?;
    let (_, library, runtime, _) = with_harness(harness, method, |config, library, runtime, closed| Ok((config.clone(), *library, *runtime, *closed)))?;
    restore_library(library, books, library_revision, method)?;
    restore_runtime(runtime, runtime_closed, runtime_revision, method)?;
    STATE.with(|state| match state.borrow_mut().records.get_mut(&harness) {
        Some(Record::Harness { closed, .. }) => { *closed = harness_closed; Ok(values[0].clone()) }
        _ => Err(failure("Harness", method, "expects a std.lang.Harness value")),
    })
}

fn harness_reset(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Harness", method, &values, 1)?;
    let harness = handle(&values[0], "Harness", method)?;
    let (_, library, runtime, _) = with_harness(harness, method, |config, library, runtime, closed| Ok((config.clone(), *library, *runtime, *closed)))?;
    restore_library(library, HashMap::new(), 0, method)?;
    restore_runtime(runtime, false, 0, method)?;
    STATE.with(|state| match state.borrow_mut().records.get_mut(&harness) {
        Some(Record::Harness { closed, .. }) => { *closed = false; Ok(values[0].clone()) }
        _ => Err(failure("Harness", method, "expects a std.lang.Harness value")),
    })
}

fn harness_close(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Harness", method, &values, 1)?;
    let harness = handle(&values[0], "Harness", method)?;
    let (_, _, runtime, _) = with_harness(harness, method, |config, library, runtime, closed| Ok((config.clone(), *library, *runtime, *closed)))?;
    restore_runtime(runtime, true, runtime_revision(runtime, method)?, method)?;
    STATE.with(|state| match state.borrow_mut().records.get_mut(&harness) {
        Some(Record::Harness { closed, .. }) => { *closed = true; Ok(values[0].clone()) }
        _ => Err(failure("Harness", method, "expects a std.lang.Harness value")),
    })
}

fn harness_state(method: &str, values: Vec<Value>) -> Result<Value, String> {
    require_arity("Harness", method, &values, 1)?;
    let harness = handle(&values[0], "Harness", method)?;
    let (_, library, runtime, closed) = with_harness(harness, method, |config, library, runtime, closed| Ok((config.clone(), *library, *runtime, *closed)))?;
    let library_value = with_library(library, method, |_, books, revision| Ok(map(vec![("revision", Value::Number(*revision)), ("book-count", Value::Number(books.len() as i64)), ("books", Value::Vector(PVector::from_iter(books.values().cloned())))])))?;
    let runtime_value = with_runtime(runtime, method, |_, runtime_closed, revision| Ok(runtime_state_value(*runtime_closed, *revision)))?;
    Ok(map(vec![("state", Value::Keyword(Keyword::from(if closed { "closed" } else { "ready" }))), ("library", library_value), ("runtime", runtime_value)]))
}

fn allocate(record: Record, type_name: &str) -> Value {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.next += 1;
        let handle = state.next;
        state.records.insert(handle, record);
        extension(type_name, handle)
    })
}

fn extension(type_name: &str, handle: u64) -> Value {
    Value::Extension(ExtensionValue { provider: PROVIDER.into(), type_name: type_name.into(), handle })
}

trait ExtensionHandle {
    fn extension_handle(&self) -> Option<u64>;
}

impl ExtensionHandle for Value {
    fn extension_handle(&self) -> Option<u64> {
        match self { Value::Extension(value) if value.provider == PROVIDER => Some(value.handle), _ => None }
    }
}

fn handle(value: &Value, expected: &str, method: &str) -> Result<u64, String> {
    match value {
        Value::Extension(value) if value.provider == PROVIDER && value.type_name == expected => Ok(value.handle),
        _ => Err(failure(expected, method, &format!("expects a std.lang.{expected} value"))),
    }
}

fn config(value: &Value, operation: &str) -> Result<Value, String> {
    map_entries(value).map(|_| value.clone()).ok_or_else(|| format!("std.lang.{operation} expects a configuration map"))
}

fn lookup(config: &Value, name: &str) -> Option<Value> {
    map_entries(config)?.into_iter().find_map(|(key, value)| match key {
        Value::Keyword(key) if key.as_str() == name => Some(value),
        _ => None,
    })
}

fn book_key(config: &Value, method: &str) -> Result<String, String> {
    lookup(config, "coordinate")
        .map(|value| value.display())
        .ok_or_else(|| failure("Library", method, "requires Book :coordinate"))
}

fn book_key_argument(value: &Value, method: &str) -> Result<String, String> {
    if let Value::Extension(extension) = value {
        if extension.provider == PROVIDER && extension.type_name == "Book" {
            return immutable_data(extension.handle, "Book", method).and_then(|data| book_key(&data, method));
        }
    }
    if map_entries(value).is_some() { return book_key(value, method); }
    Ok(value.display())
}

fn immutable_data(handle: u64, expected: &str, method: &str) -> Result<Value, String> {
    STATE.with(|state| match state.borrow().records.get(&handle) {
        Some(Record::Immutable { type_name, data }) if type_name == expected => Ok(data.clone()),
        _ => Err(failure(expected, method, "expects its matching std.lang value")),
    })
}

fn with_library<T>(handle: u64, method: &str, operation: impl FnOnce(&Value, &HashMap<String, Value>, &i64) -> Result<T, String>) -> Result<T, String> {
    STATE.with(|state| match state.borrow().records.get(&handle) {
        Some(Record::Library { config, books, revision }) => operation(config, books, revision),
        _ => Err(failure("Library", method, "expects a std.lang.Library value")),
    })
}

fn with_runtime<T>(handle: u64, method: &str, operation: impl FnOnce(&Value, &bool, &i64) -> Result<T, String>) -> Result<T, String> {
    STATE.with(|state| match state.borrow().records.get(&handle) {
        Some(Record::Runtime { config, closed, revision }) => operation(config, closed, revision),
        _ => Err(failure("Runtime", method, "expects a std.lang.Runtime value")),
    })
}

fn with_harness<T>(handle: u64, method: &str, operation: impl FnOnce(&Value, &u64, &u64, &bool) -> Result<T, String>) -> Result<T, String> {
    STATE.with(|state| match state.borrow().records.get(&handle) {
        Some(Record::Harness { config, library, runtime, closed }) => operation(config, library, runtime, closed),
        _ => Err(failure("Harness", method, "expects a std.lang.Harness value")),
    })
}

fn snapshot_library(snapshot: u64, owner: u64, method: &str) -> Result<(HashMap<String, Value>, i64), String> {
    STATE.with(|state| match state.borrow().records.get(&snapshot) {
        Some(Record::Snapshot { kind: SnapshotKind::Library, owner: snapshot_owner, books, library_revision, .. }) if *snapshot_owner == owner => Ok((books.clone(), *library_revision)),
        _ => Err(failure("Library", method, "requires a snapshot from the same Library")),
    })
}

fn snapshot_harness(snapshot: u64, owner: u64, method: &str) -> Result<(HashMap<String, Value>, i64, bool, i64, bool), String> {
    STATE.with(|state| match state.borrow().records.get(&snapshot) {
        Some(Record::Snapshot { kind: SnapshotKind::Harness, owner: snapshot_owner, books, library_revision, runtime_closed, runtime_revision, harness_closed }) if *snapshot_owner == owner => Ok((books.clone(), *library_revision, *runtime_closed, *runtime_revision, *harness_closed)),
        _ => Err(failure("Harness", method, "requires a snapshot from the same Harness")),
    })
}

fn restore_library(handle: u64, books: HashMap<String, Value>, revision: i64, method: &str) -> Result<(), String> {
    STATE.with(|state| match state.borrow_mut().records.get_mut(&handle) {
        Some(Record::Library { books: target, revision: target_revision, .. }) => { *target = books; *target_revision = revision; Ok(()) }
        _ => Err(failure("Library", method, "expects a std.lang.Library value")),
    })
}

fn restore_runtime(handle: u64, closed: bool, revision: i64, method: &str) -> Result<(), String> {
    STATE.with(|state| match state.borrow_mut().records.get_mut(&handle) {
        Some(Record::Runtime { closed: target_closed, revision: target_revision, .. }) => { *target_closed = closed; *target_revision = revision; Ok(()) }
        _ => Err(failure("Runtime", method, "expects a std.lang.Runtime value")),
    })
}

fn runtime_revision(handle: u64, method: &str) -> Result<i64, String> {
    with_runtime(handle, method, |_, _, revision| Ok(*revision))
}

fn optional_handle(config: &Value, name: &str, expected: &str, method: &str) -> Result<Option<u64>, String> {
    lookup(config, name).map(|value| handle(&value, expected, method)).transpose()
}

fn runtime_state_value(closed: bool, revision: i64) -> Value {
    map(vec![
        ("state", Value::Keyword(Keyword::from(if closed { "closed" } else { "ready" }))),
        ("revision", Value::Number(revision)),
    ])
}

fn empty_map() -> Value { Value::Map(PMap::new()) }

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(PMap::from_iter(entries.into_iter().map(|(key, value)| (Value::Keyword(Keyword::from(key)), value))))
}

fn require_arity(type_name: &str, method: &str, values: &[Value], expected: usize) -> Result<(), String> {
    if values.len() == expected { Ok(()) } else { Err(failure(type_name, method, &format!("expects {expected} argument{}", if expected == 1 { "" } else { "s" }))) }
}

fn failure(type_name: &str, method: &str, message: &str) -> String {
    format!("std.lang.{type_name}/{method} {message}")
}
