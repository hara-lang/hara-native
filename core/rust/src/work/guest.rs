use super::plan::{target_name, WorkOperation, WorkPlan, WorkRegistry, WorkRuntime};
use super::*;
use crate::core::{ExtensionValue, ProtocolRegistry};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const PROVIDER: &str = "std.native.Work";
const HOST_TYPE: &str = "WorkHost";
const RUN_TYPE: &str = "WorkRun";
const REGISTRY_TYPE: &str = "WorkRegistry";
const RUNTIME_TYPE: &str = "WorkRuntime";
const HOST_HANDLE: u64 = 1;

#[derive(Default)]
struct GuestHandles {
    next: u64,
    runs: HashMap<u64, WorkRun>,
    ids: HashMap<String, u64>,
    registries: HashMap<u64, WorkRegistry>,
    registry_ids: HashMap<usize, u64>,
    targets: HashMap<(u64, String), Value>,
    runtimes: HashMap<u64, WorkRuntime>,
}

thread_local! {
    static HANDLES: RefCell<GuestHandles> = RefCell::new(GuestHandles::default());
}

fn extension(type_name: &str, handle: u64) -> Value {
    Value::Extension(ExtensionValue {
        provider: PROVIDER.into(),
        type_name: type_name.into(),
        handle,
    })
}

pub(crate) fn default_host_value() -> Value {
    extension(HOST_TYPE, HOST_HANDLE)
}

pub(crate) fn values() -> Vec<(&'static str, Value)> {
    vec![
        (
            "default-host",
            crate::core::native_function("std.native.Work/default-host", 0, |_| {
                Ok(default_host_value())
            }),
        ),
        (
            "reset-host",
            crate::core::native_function("std.native.Work/reset-host", 1, |arguments| {
                host(&arguments, 1)?.reset();
                HANDLES.with(|handles| {
                    let mut handles = handles.borrow_mut();
                    handles.runs.clear();
                    handles.ids.clear();
                });
                Ok(arguments[0].clone())
            }),
        ),
        (
            "current-run",
            crate::core::native_function("std.native.Work/current-run", 0, |_| {
                Ok(current_work_context()
                    .map(|context| register_run(context.run))
                    .unwrap_or(Value::Nil))
            }),
        ),
        (
            "cancelled?",
            crate::core::native_function("std.native.Work/cancelled?", 0, |_| {
                current_work_context()
                    .map(|context| Value::Bool(context.cancelled()))
                    .ok_or_else(|| "cancelled? requires an active native work context".into())
            }),
        ),
        (
            "check-cancelled",
            crate::core::native_function("std.native.Work/check-cancelled", 0, |_| {
                current_work_context()
                    .ok_or_else(|| {
                        "check-cancelled requires an active native work context".to_string()
                    })?
                    .check_cancelled()
                    .map(|_| Value::Nil)
                    .map_err(|error| error.message())
            }),
        ),
        (
            "deadline-nanos",
            crate::core::native_function("std.native.Work/deadline-nanos", 0, |_| {
                let context = current_work_context().ok_or_else(|| {
                    "deadline-nanos requires an active native work context".to_string()
                })?;
                Ok(context
                    .deadline_nanos()
                    .map(|value| Value::Number(value as i64))
                    .unwrap_or(Value::Nil))
            }),
        ),
        (
            "emit",
            crate::core::native_function("std.native.Work/emit", 2, |arguments| {
                let context = current_work_context()
                    .ok_or_else(|| "emit requires an active native work context".to_string())?;
                Ok(Value::Bool(
                    context.emit(arguments[0].clone(), arguments[1].clone()),
                ))
            }),
        ),
        (
            "submit-child",
            crate::core::native_fixed_variadic_function(
                "std.native.Work/submit-child",
                2,
                |arguments| {
                    if !(2..=3).contains(&arguments.len()) {
                        return Err("submit-child expects 2 or 3 arguments".into());
                    }
                    let context = current_work_context().ok_or_else(|| {
                        "submit-child requires an active native work context".to_string()
                    })?;
                    let work = arguments[0].clone();
                    let input = arguments[1].clone();
                    let options = arguments
                        .get(2)
                        .cloned()
                        .unwrap_or_else(|| Value::Map([].into_iter().collect()));
                    let executor = option(&options, "work/execute").unwrap_or_else(|| work.clone());
                    let Value::Function(function) = executor else {
                        return Err(
                            "submit-child requires callable work or a :work/execute adapter".into(),
                        );
                    };
                    let run =
                        context.submit_child(work_options(&options)?, move |child_context| {
                            crate::core::invoke_function_sync(
                                function,
                                vec![
                                    work,
                                    input,
                                    options,
                                    Value::String(child_context.work_id().to_string()),
                                ],
                            )
                        })?;
                    Ok(register_run(run))
                },
            ),
        ),
        (
            "on-close",
            crate::core::native_function("std.native.Work/on-close", 1, |arguments| {
                let context = current_work_context()
                    .ok_or_else(|| "on-close requires an active native work context".to_string())?;
                let Value::Function(function) = arguments[0].clone() else {
                    return Err("on-close requires a callable finalizer".into());
                };
                Ok(Value::Bool(context.on_close(move |context| {
                    crate::core::invoke_function_sync(function, vec![register_run(context.run)])
                        .map(|_| ())
                        .map_err(work_failure)
                })))
            }),
        ),
        (
            "plan?",
            crate::core::native_function("std.native.Work/plan?", 1, |arguments| {
                Ok(Value::Bool(
                    WorkPlan::from_value(arguments[0].clone()).is_ok(),
                ))
            }),
        ),
        (
            "configured",
            crate::core::native_function("std.native.Work/configured", 2, |arguments| {
                Ok(
                    WorkPlan::generic(WorkOperation::parse(&arguments[0])?, arguments[1].clone())?
                        .value(),
                )
            }),
        ),
        (
            "pure",
            crate::core::native_function("std.native.Work/pure", 1, |arguments| {
                Ok(WorkPlan::pure(target_name(arguments[0].clone())?)?.value())
            }),
        ),
        (
            "step",
            crate::core::native_function("std.native.Work/step", 1, |arguments| {
                Ok(WorkPlan::step(target_name(arguments[0].clone())?)?.value())
            }),
        ),
        (
            "chain",
            crate::core::native_function("std.native.Work/chain", 1, |arguments| {
                Ok(WorkPlan::chain(plan_children(arguments[0].clone())?)?.value())
            }),
        ),
        (
            "all",
            crate::core::native_function("std.native.Work/all", 1, |arguments| {
                Ok(WorkPlan::all(plan_children(arguments[0].clone())?)?.value())
            }),
        ),
        (
            "each",
            crate::core::native_function("std.native.Work/each", 1, |arguments| {
                Ok(WorkPlan::each(work_plan(arguments[0].clone())?)?.value())
            }),
        ),
        (
            "filter",
            crate::core::native_function("std.native.Work/filter", 1, |arguments| {
                Ok(WorkPlan::filter(work_plan(arguments[0].clone())?)?.value())
            }),
        ),
        (
            "fold",
            crate::core::native_function("std.native.Work/fold", 2, |arguments| {
                Ok(WorkPlan::fold(arguments[0].clone(), work_plan(arguments[1].clone())?)?.value())
            }),
        ),
        (
            "choose",
            crate::core::native_function("std.native.Work/choose", 2, |arguments| {
                Ok(
                    WorkPlan::choose(work_plan(arguments[0].clone())?, arguments[1].clone())?
                        .value(),
                )
            }),
        ),
        (
            "graph",
            crate::core::native_function("std.native.Work/graph", 2, |arguments| {
                Ok(WorkPlan::generic(
                    WorkOperation::Graph,
                    work_map([
                        ("work/nodes", arguments[0].clone()),
                        ("work/order", arguments[1].clone()),
                    ]),
                )?
                .value())
            }),
        ),
        (
            "batch",
            crate::core::native_function("std.native.Work/batch", 1, |arguments| {
                Ok(WorkPlan::generic(
                    WorkOperation::Batch,
                    work_map([("work/process", arguments[0].clone())]),
                )?
                .value())
            }),
        ),
        (
            "bind",
            crate::core::native_function("std.native.Work/bind", 2, |arguments| {
                Ok(WorkPlan::bind(
                    work_plan(arguments[0].clone())?,
                    target_name(arguments[1].clone())?,
                )?
                .value())
            }),
        ),
        (
            "ensure",
            crate::core::native_function("std.native.Work/ensure", 2, |arguments| {
                Ok(WorkPlan::ensure(
                    work_plan(arguments[0].clone())?,
                    work_plan(arguments[1].clone())?,
                )?
                .value())
            }),
        ),
        (
            "await",
            crate::core::native_function("std.native.Work/await", 1, |arguments| {
                Ok(WorkPlan::await_(arguments[0].clone())?.value())
            }),
        ),
        (
            "encode-hta",
            crate::core::native_function("std.native.Work/encode-hta", 1, |arguments| {
                Ok(Value::Bytes(work_plan(arguments[0].clone())?.encode_hta()?))
            }),
        ),
        (
            "decode-hta",
            crate::core::native_function("std.native.Work/decode-hta", 1, |arguments| {
                let Value::Bytes(bytes) = &arguments[0] else {
                    return Err("decode-hta expects bytes".into());
                };
                Ok(WorkPlan::decode_hta(bytes)?.value())
            }),
        ),
        (
            "new-registry",
            crate::core::native_function("std.native.Work/new-registry", 0, |_| {
                Ok(register_registry(WorkRegistry::default()))
            }),
        ),
        (
            "bind-target",
            crate::core::native_function("std.native.Work/bind-target", 3, |arguments| {
                let registry = registry(&arguments, 3)?;
                let handle = registry_handle(&arguments[0])?;
                let name = target_name(arguments[1].clone())?;
                let Value::Function(function) = arguments[2].clone() else {
                    return Err("bind-target requires a callable target".into());
                };
                registry.bind(
                    name.clone(),
                    Rc::new(move |input, _| {
                        crate::core::invoke_function_sync(function.clone(), vec![input])
                            .map_err(work_failure)
                    }),
                )?;
                HANDLES.with(|handles| {
                    handles
                        .borrow_mut()
                        .targets
                        .insert((handle, name), arguments[2].clone());
                });
                Ok(arguments[0].clone())
            }),
        ),
        (
            "unbind-target",
            crate::core::native_function("std.native.Work/unbind-target", 2, |arguments| {
                let handle = registry_handle(&arguments[0])?;
                let name = target_name(arguments[1].clone())?;
                let removed = registry(&arguments, 2)?.unbind(&name);
                if removed {
                    HANDLES.with(|handles| {
                        handles.borrow_mut().targets.remove(&(handle, name));
                    });
                }
                Ok(Value::Bool(removed))
            }),
        ),
        (
            "target",
            crate::core::native_function("std.native.Work/target", 2, |arguments| {
                let handle = registry_handle(&arguments[0])?;
                let name = target_name(arguments[1].clone())?;
                Ok(HANDLES.with(|handles| {
                    handles
                        .borrow()
                        .targets
                        .get(&(handle, name))
                        .cloned()
                        .unwrap_or(Value::Nil)
                }))
            }),
        ),
        (
            "target-names",
            crate::core::native_function("std.native.Work/target-names", 1, |arguments| {
                Ok(Value::Vector(
                    registry(&arguments, 1)?
                        .target_names()
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ))
            }),
        ),
        (
            "reset-registry",
            crate::core::native_function("std.native.Work/reset-registry", 1, |arguments| {
                registry(&arguments, 1)?.reset();
                let handle = registry_handle(&arguments[0])?;
                HANDLES.with(|handles| {
                    handles
                        .borrow_mut()
                        .targets
                        .retain(|(target_handle, _), _| *target_handle != handle)
                });
                Ok(arguments[0].clone())
            }),
        ),
        (
            "new-runtime",
            crate::core::native_fixed_variadic_function(
                "std.native.Work/new-runtime",
                1,
                |arguments| {
                    if !(1..=2).contains(&arguments.len()) {
                        return Err(
                            "new-runtime expects a registry and optional suspension target".into(),
                        );
                    }
                    let registry = registry(&arguments[..1], 1)?;
                    let runtime = match arguments.get(1).cloned() {
                        None | Some(Value::Nil) => WorkRuntime::new(registry),
                        Some(Value::Function(function)) => WorkRuntime::new(registry)
                            .with_suspension(Rc::new(move |wait, _| {
                                crate::core::invoke_function_sync(function.clone(), vec![wait])
                                    .map_err(work_failure)
                            })),
                        _ => return Err("new-runtime suspension target must be callable".into()),
                    };
                    Ok(register_runtime(runtime))
                },
            ),
        ),
        (
            "runtime-registry",
            crate::core::native_function("std.native.Work/runtime-registry", 1, |arguments| {
                Ok(register_registry(runtime(&arguments, 1)?.registry()))
            }),
        ),
        (
            "evaluate",
            crate::core::native_function("std.native.Work/evaluate", 3, |arguments| {
                let run = process_work_host().submit_plan(
                    runtime(&arguments, 3)?,
                    work_plan(arguments[1].clone())?,
                    arguments[2].clone(),
                    WorkOptions::default(),
                )?;
                Ok(Value::Promise(run.work_result()))
            }),
        ),
        (
            "reset-runtime",
            crate::core::native_function("std.native.Work/reset-runtime", 1, |arguments| {
                let runtime = runtime(&arguments, 1)?;
                let registry = runtime.registry();
                runtime.reset();
                clear_registry_target_values(&registry);
                Ok(arguments[0].clone())
            }),
        ),
        (
            "submit-plan",
            crate::core::native_fixed_variadic_function(
                "std.native.Work/submit-plan",
                4,
                |arguments| {
                    if !(4..=5).contains(&arguments.len()) {
                        return Err(
                            "submit-plan expects host, runtime, plan, input, and optional options"
                                .into(),
                        );
                    }
                    let options = arguments.get(4).cloned().unwrap_or(Value::Nil);
                    let run = host(&arguments[..1], 1)?.submit_plan(
                        runtime(&arguments[1..2], 1)?,
                        work_plan(arguments[2].clone())?,
                        arguments[3].clone(),
                        work_options(&options)?,
                    )?;
                    Ok(register_run(run))
                },
            ),
        ),
    ]
}

fn work_plan(value: Value) -> Result<WorkPlan, String> {
    WorkPlan::from_value(value)
}

fn plan_children(value: Value) -> Result<Vec<WorkPlan>, String> {
    let values = match value {
        Value::Vector(values) => values.into_iter().collect::<Vec<_>>(),
        Value::Tuple(values) => values.into_iter().collect::<Vec<_>>(),
        _ => return Err("work/plan-invalid: work children must be a vector".into()),
    };
    values.into_iter().map(work_plan).collect()
}

fn work_map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(name, value)| (Value::Keyword(name.into()), value))
            .collect(),
    )
}

fn extension_handle(value: &Value, type_name: &str) -> Result<u64, String> {
    match value {
        Value::Extension(extension)
            if extension.provider == PROVIDER && extension.type_name == type_name =>
        {
            Ok(extension.handle)
        }
        _ => Err(format!(
            "native {type_name} operation requires a native {type_name}"
        )),
    }
}

fn registry_handle(value: &Value) -> Result<u64, String> {
    extension_handle(value, REGISTRY_TYPE)
}

fn registry(arguments: &[Value], arity: usize) -> Result<WorkRegistry, String> {
    if arguments.len() != arity {
        return Err(format!(
            "native WorkRegistry operation expects {arity} arguments"
        ));
    }
    let handle = registry_handle(&arguments[0])?;
    HANDLES.with(|handles| {
        handles
            .borrow()
            .registries
            .get(&handle)
            .cloned()
            .ok_or_else(|| "native work registry handle is no longer available".into())
    })
}

fn register_registry(registry: WorkRegistry) -> Value {
    HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        if let Some(handle) = handles.registry_ids.get(&registry.identity()).copied() {
            handles.registries.insert(handle, registry);
            return extension(REGISTRY_TYPE, handle);
        }
        handles.next += 1;
        let handle = handles.next;
        handles.registry_ids.insert(registry.identity(), handle);
        handles.registries.insert(handle, registry);
        extension(REGISTRY_TYPE, handle)
    })
}

fn clear_registry_target_values(registry: &WorkRegistry) {
    HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        let Some(handle) = handles.registry_ids.get(&registry.identity()).copied() else {
            return;
        };
        handles
            .targets
            .retain(|(target_handle, _), _| *target_handle != handle);
    });
}

fn runtime(arguments: &[Value], arity: usize) -> Result<WorkRuntime, String> {
    if arguments.len() != arity {
        return Err(format!(
            "native WorkRuntime operation expects {arity} arguments"
        ));
    }
    let handle = extension_handle(&arguments[0], RUNTIME_TYPE)?;
    HANDLES.with(|handles| {
        handles
            .borrow()
            .runtimes
            .get(&handle)
            .cloned()
            .ok_or_else(|| "native work runtime handle is no longer available".into())
    })
}

fn register_runtime(runtime: WorkRuntime) -> Value {
    HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        handles.next += 1;
        let handle = handles.next;
        handles.runtimes.insert(handle, runtime);
        extension(RUNTIME_TYPE, handle)
    })
}

fn host(arguments: &[Value], arity: usize) -> Result<WorkHost, String> {
    if arguments.len() != arity {
        return Err(format!(
            "native WorkHost operation expects {arity} arguments"
        ));
    }
    if arguments.first().is_some_and(is_work_host) {
        Ok(process_work_host())
    } else {
        Err("native WorkHost operation requires a native work host".into())
    }
}

fn is_work_host(value: &Value) -> bool {
    matches!(
        value,
        Value::Extension(value)
            if value.provider == PROVIDER
                && value.type_name == HOST_TYPE
                && value.handle == HOST_HANDLE
    )
}

fn is_work_run(value: &Value) -> bool {
    matches!(
        value,
        Value::Extension(value) if value.provider == PROVIDER && value.type_name == RUN_TYPE
    )
}

fn run(arguments: &[Value], arity: usize) -> Result<WorkRun, String> {
    if arguments.len() != arity {
        return Err(format!(
            "native WorkRun operation expects {arity} arguments"
        ));
    }
    let handle = match arguments.first() {
        Some(Value::Extension(value)) if is_work_run(&arguments[0]) => value.handle,
        _ => return Err("native WorkRun operation requires a native work run".into()),
    };
    HANDLES.with(|handles| {
        handles
            .borrow()
            .runs
            .get(&handle)
            .cloned()
            .ok_or_else(|| "native work run handle is no longer available".into())
    })
}

fn register_run(run: WorkRun) -> Value {
    HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        if let Some(handle) = handles.ids.get(run.work_id().as_str()).copied() {
            handles.runs.insert(handle, run);
            return extension(RUN_TYPE, handle);
        }
        handles.next += 1;
        let handle = handles.next;
        handles.ids.insert(run.work_id().to_string(), handle);
        handles.runs.insert(handle, run);
        extension(RUN_TYPE, handle)
    })
}

fn option(options: &Value, name: &str) -> Option<Value> {
    let key = Value::Keyword(name.into());
    match options {
        Value::Map(values) => values.get(&key).cloned(),
        Value::OrderedMap(values) => values.get(&key).cloned(),
        Value::SortedMap(values) => values.get(&key).cloned(),
        _ => None,
    }
}

fn id_text(value: Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value),
        Value::Keyword(value) => Ok(value.to_string()),
        Value::Symbol(value) => Ok(value.to_string()),
        _ => Err("work run ID must be a string, keyword, or symbol".into()),
    }
}

fn reference_id(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Keyword(value) => Ok(value.to_string()),
        Value::Symbol(value) => Ok(value.to_string()),
        Value::Extension(_) => Ok(run(std::slice::from_ref(value), 1)?.work_id().to_string()),
        Value::Struct(value) => value
            .get("id")
            .cloned()
            .ok_or_else(|| "work reference does not contain :id".to_string())
            .and_then(id_text),
        _ => Err("work-resolve requires a work reference or run ID".into()),
    }
}

fn work_options(options: &Value) -> Result<WorkOptions, String> {
    let id = option(options, "id")
        .or_else(|| option(options, "run/id"))
        .or_else(|| option(options, "work/id"))
        .map(id_text)
        .transpose()?
        .map(WorkId::new)
        .transpose()?;
    let timeout = option(options, "timeout-ms")
        .map(|value| match value {
            Value::Number(value) if value >= 0 => Ok(Duration::from_millis(value as u64)),
            _ => Err(":timeout-ms must be a non-negative integer".to_string()),
        })
        .transpose()?;
    let deadline = option(options, "deadline-nanos")
        .map(|value| match value {
            Value::Number(value) if value > 0 => Ok(WorkDeadline::at_monotonic_nanos(value as u64)),
            _ => Err(":deadline-nanos must be a positive integer".to_string()),
        })
        .transpose()?;
    let detached = matches!(option(options, "detached"), Some(Value::Bool(true)));
    Ok(WorkOptions {
        id,
        timeout,
        deadline,
        detached,
    })
}

fn work_submit(arguments: &[Value]) -> Result<Value, String> {
    let host = host(arguments, 4)?;
    let work = arguments[1].clone();
    let input = arguments[2].clone();
    let options = arguments[3].clone();
    let executor = option(&options, "work/execute").unwrap_or_else(|| work.clone());
    let function = match executor {
        Value::Function(function) => function,
        _ => return Err("work-submit requires callable work or a :work/execute adapter".into()),
    };
    let run = host.submit_scoped(work_options(&options)?, move |context| {
        crate::core::invoke_function_sync(
            function,
            vec![
                work,
                input,
                options,
                Value::String(context.work_id().to_string()),
            ],
        )
    })?;
    Ok(register_run(run))
}

fn work_resolve(arguments: &[Value]) -> Result<Value, String> {
    let host = host(arguments, 2)?;
    let id = reference_id(&arguments[1])?;
    Ok(register_run(host.resolve(&id)?))
}

fn work_id(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::String(run(arguments, 1)?.work_id().to_string()))
}

fn work_status(arguments: &[Value]) -> Result<Value, String> {
    let state = run(arguments, 1)?.work_status().state;
    let keyword = match state {
        WorkRunState::Queued => "queued",
        WorkRunState::Running => "running",
        WorkRunState::Waiting => "waiting",
        WorkRunState::Cancelling => "cancelling",
        WorkRunState::Completed => "completed",
        WorkRunState::Failed => "failed",
        WorkRunState::Cancelled => "cancelled",
    };
    Ok(Value::Keyword(keyword.into()))
}

fn work_result(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::Promise(run(arguments, 1)?.work_result()))
}

fn work_events(arguments: &[Value]) -> Result<Value, String> {
    let run = run(arguments, 2)?;
    let after = option(&arguments[1], "after")
        .map(|value| match value {
            Value::Number(value) if value >= 0 => Ok(value as usize),
            _ => Err(":after must be a non-negative integer".to_string()),
        })
        .transpose()?
        .unwrap_or(0);
    Ok(run.work_events(after))
}

fn work_cancel(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::Promise(
        run(arguments, 2)?.work_cancel(arguments[1].clone()),
    ))
}

fn closed(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(run(arguments, 1)?.closed()))
}

pub(crate) fn install(protocols: &mut ProtocolRegistry) {
    protocols.register_when("IWorkHost", "work-submit", is_work_host, work_submit);
    protocols.register_when("IWorkHost", "work-resolve", is_work_host, work_resolve);
    protocols.register_when("IWorkRef", "work-id", is_work_run, work_id);
    protocols.register_when("IWorkRun", "work-status", is_work_run, work_status);
    protocols.register_when("IWorkRun", "work-result", is_work_run, work_result);
    protocols.register_when("IWorkRun", "work-events", is_work_run, work_events);
    protocols.register_when("IWorkRun", "work-cancel", is_work_run, work_cancel);
    protocols.register_when("IClosed", "closed?", is_work_run, closed);

    protocols.register_when("IComponent", "props", is_work_host, |_| {
        Ok(Value::Map(
            [(
                Value::Keyword("work/host".into()),
                Value::Keyword("ephemeral".into()),
            )]
            .into_iter()
            .collect(),
        ))
    });
    protocols.register_when("IComponent", "status", is_work_host, |arguments| {
        let status = host(arguments, 1)?.status();
        Ok(Value::Keyword(status.state.into()))
    });
    protocols.register_when("IComponent", "started?", is_work_host, |arguments| {
        Ok(Value::Bool(host(arguments, 1)?.started()))
    });
    protocols.register_when("IComponent", "stopped?", is_work_host, |arguments| {
        Ok(Value::Bool(!host(arguments, 1)?.started()))
    });
    protocols.register_when("IComponent", "start", is_work_host, |arguments| {
        let host = host(arguments, 1)?;
        host.start();
        Ok(default_host_value())
    });
    protocols.register_when("IComponent", "stop", is_work_host, |arguments| {
        let host = host(arguments, 1)?;
        host.stop();
        Ok(default_host_value())
    });
    protocols.register_when("IComponent", "kill", is_work_host, |arguments| {
        let host = host(arguments, 1)?;
        host.kill();
        Ok(default_host_value())
    });
    protocols.register_when("IComponent", "remote?", is_work_host, |arguments| {
        host(arguments, 1)?;
        Ok(Value::Bool(false))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ProcessHostReset;

    impl Drop for ProcessHostReset {
        fn drop(&mut self) {
            process_work_host().reset();
            HANDLES.with(|handles| {
                let mut handles = handles.borrow_mut();
                handles.runs.clear();
                handles.ids.clear();
            });
        }
    }

    fn native(name: &str, arguments: Vec<Value>) -> Value {
        let Value::Function(function) = values()
            .into_iter()
            .find(|(candidate, _)| *candidate == name)
            .expect("native Work method must be registered")
            .1
        else {
            panic!("native Work method must be a function");
        };
        crate::core::invoke_function_sync(function, arguments)
            .unwrap_or_else(|error| panic!("native Work/{name} failed: {error}"))
    }

    #[test]
    fn plan_registry_runtime_and_host_are_exercised_through_direct_native_methods() {
        let registry = native("new-registry", vec![]);
        let target =
            crate::core::native_function("fixture/value", 1, |arguments| Ok(arguments[0].clone()));
        native(
            "bind-target",
            vec![
                registry.clone(),
                Value::String("fixture/value".into()),
                target,
            ],
        );
        let plan = native("pure", vec![Value::String("fixture/value".into())]);
        assert_eq!(native("plan?", vec![plan.clone()]), Value::Bool(true));
        assert!(matches!(
            native("encode-hta", vec![plan.clone()]),
            Value::Bytes(_)
        ));
        let runtime = native("new-runtime", vec![registry.clone()]);
        assert_eq!(native("runtime-registry", vec![runtime.clone()]), registry);
        let suspension =
            crate::core::native_function(
                "fixture/suspend",
                1,
                |arguments| Ok(arguments[0].clone()),
            );
        assert!(matches!(
            native("new-runtime", vec![registry.clone(), suspension]),
            Value::Extension(value)
                if value.provider == PROVIDER && value.type_name == RUNTIME_TYPE
        ));
        let promise = native(
            "evaluate",
            vec![runtime.clone(), plan.clone(), Value::Number(7)],
        );
        process_work_host().drain();
        let Value::Promise(promise) = promise else {
            panic!("Work/evaluate must return a promise");
        };
        assert_eq!(
            promise.wait_state(),
            PromiseState::Fulfilled(Value::Number(7))
        );

        let run = native(
            "submit-plan",
            vec![
                default_host_value(),
                runtime,
                plan,
                Value::Number(9),
                Value::Map([].into_iter().collect()),
            ],
        );
        assert!(matches!(
            &run,
            Value::Extension(value)
                if value.provider == PROVIDER && value.type_name == RUN_TYPE
        ));
        process_work_host().drain();
        let result = work_result(&[run]).expect("native work result must resolve");
        let Value::Promise(result) = result else {
            panic!("IWorkRun/work-result must return a promise");
        };
        assert_eq!(
            result.wait_state(),
            PromiseState::Fulfilled(Value::Number(9))
        );
    }

    #[test]
    fn guest_handles_are_opaque_and_stable_per_run() {
        let host = process_work_host();
        let run = host
            .submit(Some("guest-stable"), || Ok(Value::Number(42)))
            .unwrap();
        let first = register_run(run.clone());
        let second = register_run(run);
        assert_eq!(first, second);
        assert_eq!(reference_id(&first).unwrap(), "guest-stable");
    }

    #[test]
    fn reset_host_is_idempotent_and_forgets_guest_run_handles() {
        let _reset = ProcessHostReset;
        let host = process_work_host();
        host.reset();

        let run = host
            .submit(Some("guest-reset"), || Ok(Value::Number(42)))
            .expect("fixture work must be admitted");
        let handle = register_run(run.clone());

        assert_eq!(
            native("reset-host", vec![default_host_value()]),
            default_host_value()
        );
        assert_eq!(
            native("reset-host", vec![default_host_value()]),
            default_host_value()
        );
        assert!(host.started());
        assert_eq!(host.status().run_count, 0);
        assert_eq!(
            run.work_status().state,
            WorkRunState::Cancelled,
            "reset must cancel admitted work before restoring the host"
        );
        assert!(work_result(&[handle])
            .expect_err("reset must discard guest run handles")
            .contains("native work run handle is no longer available"));

        let replacement = host
            .submit(None, || Ok(Value::Number(7)))
            .expect("reset host must accept new work");
        assert_eq!(replacement.work_id().as_str(), "run-1");
        host.drain();
        assert_eq!(
            replacement.work_result().wait_state(),
            PromiseState::Fulfilled(Value::Number(7))
        );
    }

    #[test]
    fn scope_native_methods_execute_inside_admitted_work() {
        let _reset = ProcessHostReset;
        let host = process_work_host();
        host.reset();
        let finalizers = Rc::new(std::cell::Cell::new(0));
        let finalizer_count = finalizers.clone();

        let run = host
            .submit_scoped(
                WorkOptions::with_id("guest-native-scope").unwrap(),
                move |context| {
                    let current = native("current-run", vec![]);
                    assert_eq!(reference_id(&current).unwrap(), context.work_id().as_str());
                    assert_eq!(native("cancelled?", vec![]), Value::Bool(false));
                    assert_eq!(native("check-cancelled", vec![]), Value::Nil);
                    assert_eq!(native("deadline-nanos", vec![]), Value::Nil);
                    assert_eq!(
                        native(
                            "emit",
                            vec![Value::Keyword("fixture/progress".into()), Value::Number(7)]
                        ),
                        Value::Bool(true)
                    );

                    let finalizer_count = finalizer_count.clone();
                    let finalizer =
                        crate::core::native_function("fixture/on-close", 1, move |_| {
                            finalizer_count.set(finalizer_count.get() + 1);
                            Ok(Value::Nil)
                        });
                    assert_eq!(native("on-close", vec![finalizer]), Value::Bool(true));

                    let child = native(
                        "submit-child",
                        vec![
                            crate::core::native_function("fixture/child", 4, |arguments| {
                                Ok(arguments[1].clone())
                            }),
                            Value::Number(7),
                        ],
                    );
                    assert!(matches!(
                        child,
                        Value::Extension(value)
                            if value.provider == PROVIDER && value.type_name == RUN_TYPE
                    ));
                    Ok(Value::Number(7))
                },
            )
            .expect("fixture work must be admitted");
        host.drain();

        assert_eq!(
            run.work_result().wait_state(),
            PromiseState::Fulfilled(Value::Number(7))
        );
        assert_eq!(finalizers.get(), 1);
    }

    #[test]
    fn native_run_events_are_ordered_and_close_after_terminal() {
        let host = WorkHost::new();
        let run = host
            .submit(Some("guest-events"), || Ok(Value::Number(42)))
            .unwrap();
        let stream = run.work_events(0);
        host.drain();
        let values = (0..3)
            .map(|_| {
                crate::core::stream_next_value(&stream)
                    .unwrap()
                    .wait_state()
            })
            .map(|state| match state {
                PromiseState::Fulfilled(value) => value,
                state => panic!("event pull did not fulfil: {state:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values
                .iter()
                .map(|value| option(value, "event/type").unwrap())
                .collect::<Vec<_>>(),
            vec![
                Value::Keyword("work/run-queued".into()),
                Value::Keyword("work/run-running".into()),
                Value::Keyword("work/run-completed".into()),
            ]
        );
        assert_eq!(
            crate::core::stream_next_value(&stream)
                .unwrap()
                .wait_state(),
            PromiseState::Fulfilled(Value::Nil)
        );
    }
}
