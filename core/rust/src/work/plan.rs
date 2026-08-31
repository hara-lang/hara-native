//! Portable, closure-free work-plan execution.
//!
//! A [`WorkPlan`] is data that can cross a Hara package boundary.  Leaf
//! behavior is referenced by a stable target name and supplied by a local
//! [`WorkRegistry`]; closures are intentionally never part of the encoded
//! plan.  The implementation is single-threaded because guest [`Value`]s are
//! evaluator-thread values, just like [`super::WorkHost`].

use super::{WorkContext, WorkHost, WorkOptions, WorkRun};
use crate::core::{Promise, PromiseRejection, PromiseState, Value};
use crate::lang::protocol::INamespaced;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const VERSION_KEY: &str = "work/plan-version";
const OP_KEY: &str = "work/op";
const TARGET_KEY: &str = "work/target";
const CHILDREN_KEY: &str = "work/children";
const CHILD_KEY: &str = "work/child";
const SOURCE_KEY: &str = "work/source";
const CONTINUATION_KEY: &str = "work/continuation-target";
const CLEANUP_KEY: &str = "work/cleanup";
const INITIAL_KEY: &str = "work/initial";
const REDUCER_KEY: &str = "work/reducer";
const SELECTOR_KEY: &str = "work/selector";
const CHOICES_KEY: &str = "work/choices";
const WAIT_KEY: &str = "work/wait";
const MAXIMUM_DEPTH_KEY: &str = "work/maximum-depth";
const NODES_KEY: &str = "work/nodes";
const ORDER_KEY: &str = "work/order";
const PROCESS_KEY: &str = "work/process";

fn key(name: &str) -> Value {
    Value::Keyword(name.into())
}

fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(name, value)| (key(name), value))
            .collect(),
    )
}

fn field(value: &Value, name: &str) -> Option<Value> {
    match value {
        Value::Map(fields) => fields.get(&key(name)).cloned(),
        Value::OrderedMap(fields) => fields.get(&key(name)).cloned(),
        Value::SortedMap(fields) => fields.get(&key(name)).cloned(),
        _ => None,
    }
}

fn vector(value: Value, name: &str) -> Result<Vec<Value>, String> {
    match value {
        Value::Vector(values) => Ok(values.into_iter().collect()),
        _ => Err(format!("work/plan-invalid: {name} must be a vector")),
    }
}

pub fn target_name(value: Value) -> Result<String, String> {
    match value {
        Value::String(value) => non_blank_target(value),
        Value::Keyword(value) => non_blank_target(value.to_string()),
        Value::Symbol(value) => non_blank_target(value.to_string()),
        _ => Err("work/plan-invalid: work target must be a string, keyword, or symbol".into()),
    }
}

fn non_blank_target(value: String) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("work/plan-invalid: work target cannot be blank".into())
    } else {
        Ok(value)
    }
}

fn plan_error(message: impl Into<String>) -> PromiseRejection {
    PromiseRejection::Value(map([
        ("code", Value::Keyword("work/plan-error".into())),
        ("message", Value::String(message.into())),
        ("retryable", Value::Bool(false)),
    ]))
}

fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Nil | Value::Bool(false))
}

/// The operations accepted by the portable work-plan v1 envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkOperation {
    Pure,
    Step,
    Chain,
    Each,
    Filter,
    Fold,
    All,
    Choose,
    Graph,
    Batch,
    Bind,
    Ensure,
    Await,
}

impl WorkOperation {
    pub const VERSION: i64 = 1;

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::Step => "step",
            Self::Chain => "chain",
            Self::Each => "each",
            Self::Filter => "filter",
            Self::Fold => "fold",
            Self::All => "all",
            Self::Choose => "choose",
            Self::Graph => "graph",
            Self::Batch => "batch",
            Self::Bind => "bind",
            Self::Ensure => "ensure",
            Self::Await => "await",
        }
    }

    pub fn parse(value: &Value) -> Result<Self, String> {
        let Value::Keyword(value) = value else {
            return Err("work/plan-invalid: work operation must be a keyword".into());
        };
        match value.get_name() {
            "pure" => Ok(Self::Pure),
            "step" => Ok(Self::Step),
            "chain" => Ok(Self::Chain),
            "each" => Ok(Self::Each),
            "filter" => Ok(Self::Filter),
            "fold" => Ok(Self::Fold),
            "all" => Ok(Self::All),
            "choose" => Ok(Self::Choose),
            "graph" => Ok(Self::Graph),
            "batch" => Ok(Self::Batch),
            "bind" => Ok(Self::Bind),
            "ensure" => Ok(Self::Ensure),
            "await" => Ok(Self::Await),
            _ => Err(format!("work/plan-unsupported: {}", value.as_str())),
        }
    }
}

/// A canonical v1 plan backed by a Hara portable value.
#[derive(Clone, Debug)]
pub struct WorkPlan {
    value: Value,
}

impl WorkPlan {
    pub fn from_value(value: Value) -> Result<Self, String> {
        validate_plan(&value)?;
        Ok(Self { value })
    }

    pub fn value(&self) -> Value {
        self.value.clone()
    }

    pub fn operation(&self) -> WorkOperation {
        WorkOperation::parse(&field(&self.value, OP_KEY).expect("validated plan has operation"))
            .expect("validated plan has supported operation")
    }

    pub fn encode_hta(&self) -> Result<Vec<u8>, String> {
        crate::hta::encode(&self.value)
    }

    pub fn decode_hta(bytes: &[u8]) -> Result<Self, String> {
        Self::from_value(crate::hta::decode_canonical(bytes)?)
    }

    pub fn pure(target: impl Into<String>) -> Result<Self, String> {
        Self::leaf(WorkOperation::Pure, target)
    }

    pub fn step(target: impl Into<String>) -> Result<Self, String> {
        Self::leaf(WorkOperation::Step, target)
    }

    pub fn leaf(operation: WorkOperation, target: impl Into<String>) -> Result<Self, String> {
        match operation {
            WorkOperation::Pure | WorkOperation::Step => Self::from_value(map([
                (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
                (OP_KEY, Value::Keyword(operation.as_str().into())),
                (TARGET_KEY, Value::String(non_blank_target(target.into())?)),
            ])),
            _ => Err("work/plan-invalid: only pure and step are leaf operations".into()),
        }
    }

    pub fn chain(children: Vec<Self>) -> Result<Self, String> {
        Self::children(WorkOperation::Chain, children)
    }

    pub fn all(children: Vec<Self>) -> Result<Self, String> {
        Self::children(WorkOperation::All, children)
    }

    pub fn each(child: Self) -> Result<Self, String> {
        Self::child(WorkOperation::Each, child)
    }

    pub fn filter(child: Self) -> Result<Self, String> {
        Self::child(WorkOperation::Filter, child)
    }

    pub fn children(operation: WorkOperation, children: Vec<Self>) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword(operation.as_str().into())),
            (
                CHILDREN_KEY,
                Value::Vector(children.into_iter().map(|child| child.value).collect()),
            ),
        ]))
    }

    pub fn child(operation: WorkOperation, child: Self) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword(operation.as_str().into())),
            (CHILD_KEY, child.value),
        ]))
    }

    pub fn fold(initial: Value, reducer: Self) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword("fold".into())),
            (INITIAL_KEY, initial),
            (REDUCER_KEY, reducer.value),
        ]))
    }

    pub fn choose(selector: Self, choices: Value) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword("choose".into())),
            (SELECTOR_KEY, selector.value),
            (CHOICES_KEY, choices),
        ]))
    }

    pub fn graph(graph: Value) -> Result<Self, String> {
        Self::generic(WorkOperation::Graph, graph)
    }

    pub fn batch(stages: Value) -> Result<Self, String> {
        Self::generic(WorkOperation::Batch, stages)
    }

    pub fn bind(source: Self, continuation_target: impl Into<String>) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword("bind".into())),
            (SOURCE_KEY, source.value),
            (
                CONTINUATION_KEY,
                Value::String(non_blank_target(continuation_target.into())?),
            ),
        ]))
    }

    pub fn ensure(body: Self, cleanup: Self) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword("ensure".into())),
            (CHILD_KEY, body.value),
            (CLEANUP_KEY, cleanup.value),
        ]))
    }

    pub fn await_(wait: Value) -> Result<Self, String> {
        Self::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword("await".into())),
            (WAIT_KEY, wait),
        ]))
    }

    pub fn generic(operation: WorkOperation, fields: Value) -> Result<Self, String> {
        let fields = map_entries(&fields, "operation fields")?;
        let output = fields
            .into_iter()
            .chain([
                (key(VERSION_KEY), Value::Number(WorkOperation::VERSION)),
                (key(OP_KEY), Value::Keyword(operation.as_str().into())),
            ])
            .collect();
        Self::from_value(Value::Map(output))
    }
}

/// A named local implementation for a serializable work target.
pub type WorkTarget = Rc<dyn Fn(Value, WorkContext) -> Result<Value, PromiseRejection>>;

/// Process-local bindings for the otherwise portable target names in a plan.
#[derive(Clone, Default)]
pub struct WorkRegistry {
    targets: Rc<RefCell<HashMap<String, WorkTarget>>>,
}

impl WorkRegistry {
    pub fn bind(&self, name: impl Into<String>, target: WorkTarget) -> Result<(), String> {
        let name = non_blank_target(name.into())?;
        self.targets.borrow_mut().insert(name, target);
        Ok(())
    }

    pub fn unbind(&self, name: &str) -> bool {
        self.targets.borrow_mut().remove(name).is_some()
    }

    pub fn reset(&self) {
        self.targets.borrow_mut().clear();
    }

    pub fn target(&self, name: &str) -> Option<WorkTarget> {
        self.targets.borrow().get(name).cloned()
    }

    pub fn target_names(&self) -> Vec<String> {
        let mut names = self.targets.borrow().keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.targets) as usize
    }
}

pub type SuspensionTarget = Rc<dyn Fn(Value, WorkContext) -> Result<Value, PromiseRejection>>;

/// Evaluates plans against explicit targets and an optional suspension bridge.
#[derive(Clone, Default)]
pub struct WorkRuntime {
    registry: WorkRegistry,
    suspension: Option<SuspensionTarget>,
}

impl WorkRuntime {
    pub fn new(registry: WorkRegistry) -> Self {
        Self {
            registry,
            suspension: None,
        }
    }

    pub fn registry(&self) -> WorkRegistry {
        self.registry.clone()
    }

    pub fn with_suspension(mut self, suspension: SuspensionTarget) -> Self {
        self.suspension = Some(suspension);
        self
    }

    pub fn reset(&self) {
        self.registry.reset();
    }

    pub fn evaluate(
        &self,
        plan: &WorkPlan,
        input: Value,
        context: WorkContext,
    ) -> Result<Value, PromiseRejection> {
        execute(self.clone(), plan.value(), input, context, 0)
    }
}

impl WorkHost {
    /// Submit a portable plan through the existing lifecycle host.
    pub fn submit_plan(
        &self,
        runtime: WorkRuntime,
        plan: WorkPlan,
        input: Value,
        options: WorkOptions,
    ) -> Result<WorkRun, String> {
        self.submit_scoped_rejection(options, move |context| {
            runtime.evaluate(&plan, input, context)
        })
    }

    /// Cancels and forgets all runs, restores admission, and is idempotent.
    pub fn reset(&self) {
        self.kill();
        let mut host = self.inner.borrow_mut();
        host.queue.clear();
        host.runs.clear();
        host.next_id = 1;
        host.started = true;
    }
}

fn validate_plan(value: &Value) -> Result<(), String> {
    let (Value::Map(_) | Value::OrderedMap(_) | Value::SortedMap(_)) = value else {
        return Err("work/plan-invalid: plan must be a map".into());
    };
    if field(value, VERSION_KEY) != Some(Value::Number(WorkOperation::VERSION)) {
        return Err("work/plan-invalid: unsupported plan version".into());
    }
    let operation =
        WorkOperation::parse(&field(value, OP_KEY).ok_or("work/plan-invalid: missing operation")?)?;
    match operation {
        WorkOperation::Pure | WorkOperation::Step => {
            target_name(
                field(value, TARGET_KEY).ok_or("work/plan-invalid: leaf requires target")?,
            )?;
        }
        WorkOperation::Chain | WorkOperation::All => {
            for child in vector(
                field(value, CHILDREN_KEY).ok_or("work/plan-invalid: missing children")?,
                CHILDREN_KEY,
            )? {
                validate_plan(&child)?;
            }
        }
        WorkOperation::Each | WorkOperation::Filter => {
            validate_plan(&field(value, CHILD_KEY).ok_or("work/plan-invalid: missing child")?)?;
        }
        WorkOperation::Fold => {
            validate_plan(&field(value, REDUCER_KEY).ok_or("work/plan-invalid: missing reducer")?)?;
        }
        WorkOperation::Choose => {
            validate_plan(
                &field(value, SELECTOR_KEY).ok_or("work/plan-invalid: missing selector")?,
            )?;
            let choices = field(value, CHOICES_KEY).ok_or("work/plan-invalid: missing choices")?;
            for choice in map_values(&choices, CHOICES_KEY)? {
                validate_plan(&choice)?;
            }
        }
        WorkOperation::Bind => {
            validate_plan(
                &field(value, SOURCE_KEY).ok_or("work/plan-invalid: missing bind source")?,
            )?;
            target_name(
                field(value, CONTINUATION_KEY)
                    .ok_or("work/plan-invalid: missing continuation target")?,
            )?;
        }
        WorkOperation::Ensure => {
            validate_plan(
                &field(value, CHILD_KEY).ok_or("work/plan-invalid: missing ensure body")?,
            )?;
            validate_plan(&field(value, CLEANUP_KEY).ok_or("work/plan-invalid: missing cleanup")?)?;
        }
        WorkOperation::Await => {
            if field(value, WAIT_KEY).is_none() {
                return Err("work/plan-invalid: await requires a wait descriptor".into());
            }
        }
        WorkOperation::Graph => {
            let nodes = field(value, NODES_KEY).ok_or("work/plan-invalid: missing graph nodes")?;
            for child in map_values(&nodes, NODES_KEY)? {
                validate_plan(&child)?;
            }
            for id in vector(
                field(value, ORDER_KEY).ok_or("work/plan-invalid: missing graph order")?,
                ORDER_KEY,
            )? {
                let child = map_lookup(&nodes, &id)
                    .ok_or("work/plan-invalid: graph order refers to an unknown node")?;
                validate_plan(&child)?;
            }
        }
        WorkOperation::Batch => {
            validate_plan(
                &field(value, PROCESS_KEY).ok_or("work/plan-invalid: missing batch process")?,
            )?;
        }
    }
    Ok(())
}

fn map_values(value: &Value, name: &str) -> Result<Vec<Value>, String> {
    match value {
        Value::Map(entries) => Ok(entries.iter().map(|(_, value)| value.clone()).collect()),
        Value::OrderedMap(entries) => Ok(entries.iter().map(|(_, value)| value.clone()).collect()),
        Value::SortedMap(entries) => Ok(entries.iter().map(|(_, value)| value.clone()).collect()),
        _ => Err(format!("work/plan-invalid: {name} must be a map")),
    }
}

fn map_entries(value: &Value, name: &str) -> Result<Vec<(Value, Value)>, String> {
    match value {
        Value::Map(entries) => Ok(entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        Value::OrderedMap(entries) => Ok(entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        Value::SortedMap(entries) => Ok(entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        _ => Err(format!("work/plan-invalid: {name} must be a map")),
    }
}

fn map_lookup(value: &Value, lookup: &Value) -> Option<Value> {
    match value {
        Value::Map(entries) => entries.get(lookup).cloned(),
        Value::OrderedMap(entries) => entries.get(lookup).cloned(),
        Value::SortedMap(entries) => entries.get(lookup).cloned(),
        _ => None,
    }
}

fn then(
    value: Value,
    continuation: Rc<dyn Fn(Value) -> Result<Value, PromiseRejection>>,
) -> Result<Value, PromiseRejection> {
    let Value::Promise(source) = value else {
        return continuation(value);
    };
    let output = Promise::new();
    let destination = output.clone();
    source.on_settle(Rc::new(move |state| match state {
        PromiseState::Fulfilled(value) => match continuation(value) {
            Ok(Value::Promise(value)) => {
                destination.adopt(&value);
            }
            Ok(value) => {
                destination.resolve(value);
            }
            Err(error) => {
                destination.reject_rejection(error);
            }
        },
        PromiseState::Rejected(error) => {
            destination.reject_rejection(error);
        }
        PromiseState::Pending => {}
    }));
    Ok(Value::Promise(output))
}

fn settle(
    value: Value,
    fulfilled: Rc<dyn Fn(Value) -> Result<Value, PromiseRejection>>,
    rejected: Rc<dyn Fn(PromiseRejection) -> Result<Value, PromiseRejection>>,
) -> Result<Value, PromiseRejection> {
    let Value::Promise(source) = value else {
        return fulfilled(value);
    };
    let output = Promise::new();
    let destination = output.clone();
    source.on_settle(Rc::new(move |state| {
        let next = match state {
            PromiseState::Fulfilled(value) => fulfilled(value),
            PromiseState::Rejected(error) => rejected(error),
            PromiseState::Pending => return,
        };
        match next {
            Ok(Value::Promise(value)) => {
                destination.adopt(&value);
            }
            Ok(value) => {
                destination.resolve(value);
            }
            Err(error) => {
                destination.reject_rejection(error);
            }
        }
    }));
    Ok(Value::Promise(output))
}

fn execute(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    bind_depth: usize,
) -> Result<Value, PromiseRejection> {
    validate_plan(&value).map_err(plan_error)?;
    let operation = WorkOperation::parse(&field(&value, OP_KEY).expect("validated plan has op"))
        .map_err(plan_error)?;
    context.check_cancelled()?;
    let _ = context.emit(
        Value::Keyword("work/node-started".into()),
        map([("operation", Value::Keyword(operation.as_str().into()))]),
    );
    let result = match operation {
        WorkOperation::Pure | WorkOperation::Step => {
            execute_target(&runtime, &value, input, context.clone())
        }
        WorkOperation::Chain => execute_chain(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::All => execute_all(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::Each => {
            execute_each(runtime, value, input, context.clone(), bind_depth, false)
        }
        WorkOperation::Filter => {
            execute_each(runtime, value, input, context.clone(), bind_depth, true)
        }
        WorkOperation::Fold => execute_fold(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::Choose => execute_choose(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::Bind => execute_bind(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::Ensure => execute_ensure(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::Await => execute_await(&runtime, value, context.clone()),
        WorkOperation::Graph => execute_graph(runtime, value, input, context.clone(), bind_depth),
        WorkOperation::Batch => execute_batch(runtime, value, input, context.clone(), bind_depth),
    };
    if result.is_ok() {
        let _ = context.emit(
            Value::Keyword("work/node-completed".into()),
            map([("operation", Value::Keyword(operation.as_str().into()))]),
        );
    }
    result
}

fn execute_target(
    runtime: &WorkRuntime,
    value: &Value,
    input: Value,
    context: WorkContext,
) -> Result<Value, PromiseRejection> {
    let target = target_name(field(value, TARGET_KEY).expect("validated leaf has target"))
        .map_err(plan_error)?;
    runtime
        .registry
        .target(&target)
        .ok_or_else(|| plan_error(format!("work/target-unavailable: {target}")))?(input, context)
}

fn execute_chain(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let children = vector(
        field(&value, CHILDREN_KEY).expect("validated chain children"),
        CHILDREN_KEY,
    )
    .map_err(plan_error)?;
    fn next(
        runtime: WorkRuntime,
        children: Rc<Vec<Value>>,
        index: usize,
        input: Value,
        context: WorkContext,
        depth: usize,
    ) -> Result<Value, PromiseRejection> {
        let Some(child) = children.get(index).cloned() else {
            return Ok(input);
        };
        let runtime_next = runtime.clone();
        let children_next = children.clone();
        let context_next = context.clone();
        let value = execute(runtime, child, input, context, depth)?;
        then(
            value,
            Rc::new(move |resolved| {
                next(
                    runtime_next.clone(),
                    children_next.clone(),
                    index + 1,
                    resolved,
                    context_next.clone(),
                    depth,
                )
            }),
        )
    }
    next(runtime, Rc::new(children), 0, input, context, depth)
}

fn execute_all(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let children = vector(
        field(&value, CHILDREN_KEY).expect("validated all children"),
        CHILDREN_KEY,
    )
    .map_err(plan_error)?;
    fn next(
        runtime: WorkRuntime,
        children: Rc<Vec<Value>>,
        index: usize,
        input: Value,
        context: WorkContext,
        depth: usize,
        output: Vec<Value>,
    ) -> Result<Value, PromiseRejection> {
        let Some(child) = children.get(index).cloned() else {
            return Ok(Value::Vector(output.into_iter().collect()));
        };
        let runtime_next = runtime.clone();
        let children_next = children.clone();
        let context_next = context.clone();
        let value = execute(runtime, child, input.clone(), context, depth)?;
        then(
            value,
            Rc::new(move |resolved| {
                let mut next_output = output.clone();
                next_output.push(resolved);
                next(
                    runtime_next.clone(),
                    children_next.clone(),
                    index + 1,
                    input.clone(),
                    context_next.clone(),
                    depth,
                    next_output,
                )
            }),
        )
    }
    next(
        runtime,
        Rc::new(children),
        0,
        input,
        context,
        depth,
        Vec::new(),
    )
}

fn sequence_input(value: Value) -> Result<Vec<Value>, PromiseRejection> {
    vector(value, "work input").map_err(plan_error)
}

fn execute_each(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
    filtering: bool,
) -> Result<Value, PromiseRejection> {
    let child = field(&value, CHILD_KEY).expect("validated child");
    let values = sequence_input(input)?;
    fn next(
        runtime: WorkRuntime,
        child: Value,
        values: Rc<Vec<Value>>,
        index: usize,
        context: WorkContext,
        depth: usize,
        filtering: bool,
        output: Vec<Value>,
    ) -> Result<Value, PromiseRejection> {
        let Some(item) = values.get(index).cloned() else {
            return Ok(Value::Vector(output.into_iter().collect()));
        };
        let runtime_next = runtime.clone();
        let child_next = child.clone();
        let values_next = values.clone();
        let context_next = context.clone();
        let value = execute(runtime, child, item.clone(), context, depth)?;
        then(
            value,
            Rc::new(move |resolved| {
                let mut next_output = output.clone();
                if !filtering || truthy(&resolved) {
                    next_output.push(if filtering { item.clone() } else { resolved });
                }
                next(
                    runtime_next.clone(),
                    child_next.clone(),
                    values_next.clone(),
                    index + 1,
                    context_next.clone(),
                    depth,
                    filtering,
                    next_output,
                )
            }),
        )
    }
    next(
        runtime,
        child,
        Rc::new(values),
        0,
        context,
        depth,
        filtering,
        Vec::new(),
    )
}

fn execute_fold(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let reducer = field(&value, REDUCER_KEY).expect("validated reducer");
    let initial = field(&value, INITIAL_KEY).unwrap_or(Value::Nil);
    let values = sequence_input(input)?;
    fn next(
        runtime: WorkRuntime,
        reducer: Value,
        values: Rc<Vec<Value>>,
        index: usize,
        accumulator: Value,
        context: WorkContext,
        depth: usize,
    ) -> Result<Value, PromiseRejection> {
        let Some(item) = values.get(index).cloned() else {
            return Ok(accumulator);
        };
        let runtime_next = runtime.clone();
        let reducer_next = reducer.clone();
        let values_next = values.clone();
        let context_next = context.clone();
        let request = map([("acc", accumulator), ("item", item)]);
        let value = execute(runtime, reducer, request, context, depth)?;
        then(
            value,
            Rc::new(move |resolved| {
                next(
                    runtime_next.clone(),
                    reducer_next.clone(),
                    values_next.clone(),
                    index + 1,
                    resolved,
                    context_next.clone(),
                    depth,
                )
            }),
        )
    }
    next(
        runtime,
        reducer,
        Rc::new(values),
        0,
        initial,
        context,
        depth,
    )
}

fn execute_choose(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let selector = field(&value, SELECTOR_KEY).expect("validated selector");
    let choices = field(&value, CHOICES_KEY).expect("validated choices");
    let runtime_next = runtime.clone();
    let context_next = context.clone();
    let selected = execute(runtime, selector, input.clone(), context, depth)?;
    then(
        selected,
        Rc::new(move |selected| {
            let child =
                map_lookup(&choices, &selected).ok_or_else(|| plan_error("work/choice-missing"))?;
            execute(
                runtime_next.clone(),
                child,
                input.clone(),
                context_next.clone(),
                depth,
            )
        }),
    )
}

fn execute_graph(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let nodes = field(&value, NODES_KEY).expect("validated graph nodes");
    let order = vector(
        field(&value, ORDER_KEY).expect("validated graph order"),
        ORDER_KEY,
    )
    .map_err(plan_error)?;
    fn next(
        runtime: WorkRuntime,
        nodes: Value,
        order: Rc<Vec<Value>>,
        index: usize,
        input: Value,
        context: WorkContext,
        depth: usize,
        output: Vec<(Value, Value)>,
    ) -> Result<Value, PromiseRejection> {
        let Some(id) = order.get(index).cloned() else {
            return Ok(Value::Map(output.into_iter().collect()));
        };
        let child = map_lookup(&nodes, &id).expect("validated graph node");
        let runtime_next = runtime.clone();
        let nodes_next = nodes.clone();
        let order_next = order.clone();
        let input_next = input.clone();
        let context_next = context.clone();
        let value = execute(runtime, child, input, context, depth)?;
        then(
            value,
            Rc::new(move |resolved| {
                let mut next_output = output.clone();
                next_output.push((id.clone(), resolved));
                next(
                    runtime_next.clone(),
                    nodes_next.clone(),
                    order_next.clone(),
                    index + 1,
                    input_next.clone(),
                    context_next.clone(),
                    depth,
                    next_output,
                )
            }),
        )
    }
    next(
        runtime,
        nodes,
        Rc::new(order),
        0,
        input,
        context,
        depth,
        Vec::new(),
    )
}

fn execute_batch(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let each = map([
        (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
        (OP_KEY, Value::Keyword("each".into())),
        (
            CHILD_KEY,
            field(&value, PROCESS_KEY).expect("validated batch process"),
        ),
    ]);
    execute_each(runtime, each, input, context, depth, false)
}

fn execute_bind(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let maximum = field(&value, MAXIMUM_DEPTH_KEY)
        .and_then(|value| match value {
            Value::Number(value) if value > 0 => Some(value as usize),
            _ => None,
        })
        .unwrap_or(64);
    if depth >= maximum {
        return Err(plan_error("work/bind-depth-exceeded"));
    }
    let source = field(&value, SOURCE_KEY).expect("validated source");
    let target = target_name(field(&value, CONTINUATION_KEY).expect("validated continuation"))
        .map_err(plan_error)?;
    let runtime_next = runtime.clone();
    let context_next = context.clone();
    let source_value = execute(runtime, source, input, context, depth)?;
    then(
        source_value,
        Rc::new(move |resolved| {
            let produced = runtime_next
                .registry
                .target(&target)
                .ok_or_else(|| plan_error(format!("work/target-unavailable: {target}")))?(
                resolved.clone(),
                context_next.clone(),
            )?;
            let plan = WorkPlan::from_value(produced)
                .map_err(|_| plan_error("work/bind-target-returned-non-plan"))?;
            execute(
                runtime_next.clone(),
                plan.value(),
                resolved,
                context_next.clone(),
                depth + 1,
            )
        }),
    )
}

fn finish_ensure(
    runtime: WorkRuntime,
    cleanup: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
    body_status: &'static str,
    body_result: Value,
    body_error: Option<PromiseRejection>,
) -> Result<Value, PromiseRejection> {
    let cleanup_value = map([
        ("work/body-status", Value::Keyword(body_status.into())),
        ("work/body-result", body_result.clone()),
        ("work/input", input),
    ]);
    let cleanup_result = execute(runtime, cleanup, cleanup_value, context, depth)?;
    then(
        cleanup_result,
        Rc::new(move |_| match &body_error {
            Some(error) => Err(error.clone()),
            None => Ok(body_result.clone()),
        }),
    )
}

fn execute_ensure(
    runtime: WorkRuntime,
    value: Value,
    input: Value,
    context: WorkContext,
    depth: usize,
) -> Result<Value, PromiseRejection> {
    let body = field(&value, CHILD_KEY).expect("validated ensure body");
    let cleanup = field(&value, CLEANUP_KEY).expect("validated cleanup");
    let result = execute(runtime.clone(), body, input.clone(), context.clone(), depth);
    match result {
        Ok(result) => {
            let completed_runtime = runtime.clone();
            let completed_cleanup = cleanup.clone();
            let completed_input = input.clone();
            let completed_context = context.clone();
            settle(
                result,
                Rc::new(move |body_result| {
                    finish_ensure(
                        completed_runtime.clone(),
                        completed_cleanup.clone(),
                        completed_input.clone(),
                        completed_context.clone(),
                        depth,
                        "completed",
                        body_result,
                        None,
                    )
                }),
                Rc::new(move |body_error| {
                    finish_ensure(
                        runtime.clone(),
                        cleanup.clone(),
                        input.clone(),
                        context.clone(),
                        depth,
                        "failed",
                        Value::Nil,
                        Some(body_error),
                    )
                }),
            )
        }
        Err(body_error) => finish_ensure(
            runtime,
            cleanup,
            input,
            context,
            depth,
            "failed",
            Value::Nil,
            Some(body_error),
        ),
    }
}

fn execute_await(
    runtime: &WorkRuntime,
    value: Value,
    context: WorkContext,
) -> Result<Value, PromiseRejection> {
    let wait = field(&value, WAIT_KEY).expect("validated await");
    runtime
        .suspension
        .as_ref()
        .ok_or_else(|| plan_error("work/suspension-unavailable"))?(wait, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn registry() -> WorkRegistry {
        let registry = WorkRegistry::default();
        registry
            .bind(
                "fixture/inc",
                Rc::new(|value, _| match value {
                    Value::Number(value) => Ok(Value::Number(value + 1)),
                    _ => Err(plan_error("fixture expects number")),
                }),
            )
            .unwrap();
        registry
            .bind(
                "fixture/double",
                Rc::new(|value, _| match value {
                    Value::Number(value) => Ok(Value::Number(value * 2)),
                    _ => Err(plan_error("fixture expects number")),
                }),
            )
            .unwrap();
        registry
    }

    #[test]
    fn hta_round_trip_is_canonical_for_closure_free_plans() {
        let plan = WorkPlan::chain(vec![
            WorkPlan::pure("fixture/inc").unwrap(),
            WorkPlan::step("fixture/double").unwrap(),
        ])
        .unwrap();
        let bytes = plan.encode_hta().unwrap();
        assert_eq!(
            bytes,
            WorkPlan::decode_hta(&bytes).unwrap().encode_hta().unwrap()
        );
    }

    #[test]
    fn named_targets_execute_through_the_existing_host_lifecycle() {
        let host = WorkHost::new();
        let runtime = WorkRuntime::new(registry());
        let plan = WorkPlan::chain(vec![
            WorkPlan::pure("fixture/inc").unwrap(),
            WorkPlan::step("fixture/double").unwrap(),
        ])
        .unwrap();
        let run = host
            .submit_plan(
                runtime,
                plan,
                Value::Number(4),
                WorkOptions::with_id("plan-run").unwrap(),
            )
            .unwrap();
        assert_eq!(
            run.work_result().wait_state(),
            PromiseState::Fulfilled(Value::Number(10))
        );
        assert_eq!(
            run.work_status().state,
            super::super::WorkRunState::Completed
        );
    }

    #[test]
    fn missing_targets_fail_closed_and_reset_is_idempotent() {
        let host = WorkHost::new();
        let runtime = WorkRuntime::default();
        let run = host
            .submit_plan(
                runtime,
                WorkPlan::pure("missing").unwrap(),
                Value::Nil,
                WorkOptions::default(),
            )
            .unwrap();
        assert!(matches!(
            run.work_result().wait_state(),
            PromiseState::Rejected(_)
        ));
        host.reset();
        host.reset();
        assert_eq!(host.status().run_count, 0);
        assert!(host.started());
    }

    #[test]
    fn keyword_targets_use_their_stable_names() {
        let plan = WorkPlan::from_value(map([
            (VERSION_KEY, Value::Number(WorkOperation::VERSION)),
            (OP_KEY, Value::Keyword("pure".into())),
            (TARGET_KEY, Value::Keyword("fixture/inc".into())),
        ]))
        .unwrap();
        let host = WorkHost::new();
        let run = host
            .submit_plan(
                WorkRuntime::new(registry()),
                plan,
                Value::Number(4),
                WorkOptions::default(),
            )
            .unwrap();
        assert_eq!(
            run.work_result().wait_state(),
            PromiseState::Fulfilled(Value::Number(5))
        );
    }

    #[test]
    fn ensure_runs_cleanup_for_completed_and_failed_bodies() {
        let registry = registry();
        let cleanup_count = Rc::new(Cell::new(0));
        let cleanup_count_next = cleanup_count.clone();
        registry
            .bind(
                "fixture/cleanup",
                Rc::new(move |_, _| {
                    cleanup_count_next.set(cleanup_count_next.get() + 1);
                    Ok(Value::Nil)
                }),
            )
            .unwrap();
        registry
            .bind(
                "fixture/fail",
                Rc::new(|_, _| Err(plan_error("fixture fails"))),
            )
            .unwrap();

        let host = WorkHost::new();
        let runtime = WorkRuntime::new(registry);
        let cleanup = WorkPlan::step("fixture/cleanup").unwrap();
        let completed = host
            .submit_plan(
                runtime.clone(),
                WorkPlan::ensure(WorkPlan::pure("fixture/inc").unwrap(), cleanup.clone()).unwrap(),
                Value::Number(4),
                WorkOptions::with_id("ensure-completed").unwrap(),
            )
            .unwrap();
        assert_eq!(
            completed.work_result().wait_state(),
            PromiseState::Fulfilled(Value::Number(5))
        );

        let failed = host
            .submit_plan(
                runtime,
                WorkPlan::ensure(WorkPlan::pure("fixture/fail").unwrap(), cleanup).unwrap(),
                Value::Number(4),
                WorkOptions::with_id("ensure-failed").unwrap(),
            )
            .unwrap();
        assert!(matches!(
            failed.work_result().wait_state(),
            PromiseState::Rejected(_)
        ));
        assert_eq!(cleanup_count.get(), 2);
    }

    #[test]
    fn graph_and_batch_execute_their_data_owned_children() {
        let runtime = WorkRuntime::new(registry());
        let host = WorkHost::new();
        let graph = WorkPlan::graph(map([
            (
                "work/nodes",
                Value::Map(
                    [(
                        Value::Keyword("increment".into()),
                        WorkPlan::pure("fixture/inc").unwrap().value(),
                    )]
                    .into_iter()
                    .collect(),
                ),
            ),
            (
                "work/order",
                Value::Vector([Value::Keyword("increment".into())].into_iter().collect()),
            ),
        ]))
        .unwrap();
        let graph_result = host
            .submit_plan(
                runtime.clone(),
                graph,
                Value::Number(4),
                WorkOptions::with_id("plan-graph").unwrap(),
            )
            .unwrap()
            .work_result()
            .wait_state();
        let PromiseState::Fulfilled(graph_result) = graph_result else {
            panic!("graph plan should succeed");
        };
        assert_eq!(
            map_lookup(&graph_result, &Value::Keyword("increment".into())),
            Some(Value::Number(5))
        );

        let batch = WorkPlan::batch(map([(
            "work/process",
            WorkPlan::step("fixture/double").unwrap().value(),
        )]))
        .unwrap();
        assert_eq!(
            host.submit_plan(
                runtime,
                batch,
                Value::Vector([Value::Number(2), Value::Number(3)].into_iter().collect()),
                WorkOptions::with_id("plan-batch").unwrap()
            )
            .unwrap()
            .work_result()
            .wait_state(),
            PromiseState::Fulfilled(Value::Vector(
                [Value::Number(4), Value::Number(6)].into_iter().collect()
            ))
        );

        let invalid = WorkPlan::graph(map([
            (
                "work/nodes",
                Value::Map(
                    [(Value::Keyword("unused".into()), Value::Number(1))]
                        .into_iter()
                        .collect(),
                ),
            ),
            (
                "work/order",
                Value::Vector(Vec::new().into_iter().collect()),
            ),
        ]))
        .unwrap_err();
        assert!(invalid.contains("plan must be a map"));
    }
}
