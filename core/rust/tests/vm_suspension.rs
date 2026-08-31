#![cfg(feature = "bytecode-vm")]

use hara_wasm::core::{with_namespace_registry, Promise, PromiseState, Value};
use hara_wasm::kernel::NamespaceRegistry;
use hara_wasm::lang::protocol::IDisplay;
use hara_wasm::vm::{compile_source_with, execute_program_with_globals, VmFiber, VmFiberState};
use std::cell::Cell;
use std::rc::Rc;

fn compile_with_source(
    source: &str,
    promise: Promise,
) -> (NamespaceRegistry<Value>, Rc<hara_wasm::vm::Program>) {
    let registry = NamespaceRegistry::new("user");
    registry
        .find_or_create("user")
        .intern("source", Value::Promise(promise));
    let program = compile_source_with(source, &registry)
        .unwrap_or_else(|error| panic!("bytecode compilation failed for {source}: {error}"));
    (registry, Rc::new(program))
}

#[test]
fn fulfilled_await_resumes_nested_frames_and_runs_finally() {
    let source = Promise::new();
    let (registry, program) = compile_with_source(
        "(do
           (def marker 0)
           (defn inner [p]
             (try
               (std.native.Coroutine/await p)
               (finally (set! marker 1))))
           (defn outer [p] [(inner p) marker])
           (outer source))",
        source.clone(),
    );

    let mut fiber = with_namespace_registry(&registry, || VmFiber::start(program));
    assert!(matches!(fiber.state(), VmFiberState::Suspended));
    assert!(fiber
        .pending()
        .is_some_and(|pending| pending.same_identity(&source)));

    assert!(source.resolve(Value::Number(41)));
    let state = with_namespace_registry(&registry, || fiber.poll());
    let VmFiberState::Completed(value) = state else {
        panic!("fulfilled await did not complete the VM fiber")
    };
    assert_eq!(value.display(), "[41 1]");
}

#[test]
fn rejected_await_runs_finally_before_the_outer_catch() {
    let source = Promise::new();
    let (registry, program) = compile_with_source(
        "(do
           (def marker 0)
           (defn inner [p]
             (try
               (std.native.Coroutine/await p)
               (finally (set! marker 1))))
           (try
             (inner source)
             (catch error [error marker])))",
        source.clone(),
    );

    let mut fiber = with_namespace_registry(&registry, || VmFiber::start(program));
    assert!(matches!(fiber.state(), VmFiberState::Suspended));

    assert!(source.reject("failed"));
    let state = with_namespace_registry(&registry, || fiber.poll());
    let VmFiberState::Completed(value) = state else {
        panic!("rejected await was not handled by the outer catch")
    };
    assert_eq!(value.display(), "[\"failed\" 1]");
}

#[test]
fn yield_parks_the_machine_and_resume_value_becomes_the_expression_result() {
    let registry = NamespaceRegistry::new("user");
    let program = compile_source_with(
        "(do
           (defn generator [] (std.native.Coroutine/yield 41))
           (generator))",
        &registry,
    )
    .expect("yielding bytecode must compile");

    let mut fiber = with_namespace_registry(&registry, || VmFiber::start(Rc::new(program)));
    assert!(matches!(
        fiber.state(),
        VmFiberState::Yielded(Value::Number(41))
    ));

    let state = with_namespace_registry(&registry, || fiber.resume_yield(Value::Number(42)));
    assert!(matches!(state, VmFiberState::Completed(Value::Number(42))));
}

#[test]
fn async_calls_keep_a_promise_shape_on_the_settled_fast_path() {
    let source = Promise::new();
    assert!(source.resolve(Value::Number(42)));
    let (registry, program) = compile_with_source(
        "(do
           (defn ^:async delayed []
             (std.native.Coroutine/await source))
           (delayed))",
        source,
    );

    let value = execute_program_with_globals(program, &registry)
        .expect("async bytecode call must return its result promise");
    let Value::Promise(result) = value else {
        panic!("async bytecode call did not preserve the Promise return shape")
    };
    assert!(matches!(
        result.state(),
        PromiseState::Fulfilled(Value::Number(42))
    ));
}

#[test]
fn cancelling_an_async_result_notifies_the_pending_host_promise() {
    let source = Promise::new();
    let cancelled = Rc::new(Cell::new(false));
    let observed = cancelled.clone();
    source.set_cancel_hook(Rc::new(move || observed.set(true)));

    let (registry, program) = compile_with_source(
        "(do
           (defn ^:async delayed []
             (std.native.Coroutine/await source))
           (delayed))",
        source,
    );
    let value = execute_program_with_globals(program, &registry)
        .expect("async bytecode call must return its result promise");
    let Value::Promise(result) = value else {
        panic!("async bytecode call did not return a Promise")
    };

    assert!(result.cancel());
    assert!(cancelled.get());
    assert!(matches!(
        result.state(),
        PromiseState::Rejected(error) if error.is_cancelled()
    ));
}
