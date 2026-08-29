use hara_wasm::core::Value;
use hara_wasm::hta;
use hara_wasm::native_cli::RuntimeBroker;
use hara_wasm::{InvokeHtaError, Runtime};

fn arguments(values: Vec<Value>) -> Vec<u8> {
    hta::encode(&Value::Vector(values.into())).expect("canonical arguments")
}

fn number(bytes: &[u8]) -> i64 {
    let Value::Number(value) = hta::decode(bytes).expect("result HTA") else {
        panic!("expected integer result")
    };
    value
}

#[test]
fn runtime_invokes_only_an_already_loaded_qualified_var() {
    let mut runtime = Runtime::core();
    runtime
        .eval_native("(ns invoke.sample) (defn add [a b] (+ a b)) (def answer 42)")
        .expect("load sample namespace");

    let result = runtime
        .invoke_hta(
            "invoke.sample/add",
            &arguments(vec![Value::Number(20), Value::Number(22)]),
        )
        .expect("invoke add");
    assert_eq!(number(&result), 42);

    assert_eq!(
        runtime.invoke_hta("add", &arguments(vec![])),
        Err(InvokeHtaError::InvalidQualifiedVar)
    );
    assert_eq!(
        runtime.invoke_hta("missing.namespace/add", &arguments(vec![])),
        Err(InvokeHtaError::NamespaceMissing(
            "missing.namespace".to_owned()
        ))
    );
    assert_eq!(
        runtime.invoke_hta("invoke.sample/missing", &arguments(vec![])),
        Err(InvokeHtaError::VarMissing(
            "invoke.sample/missing".to_owned()
        ))
    );
    assert_eq!(
        runtime.invoke_hta("invoke.sample/answer", &arguments(vec![])),
        Err(InvokeHtaError::VarNotCallable(
            "invoke.sample/answer".to_owned()
        ))
    );
}

#[test]
fn runtime_rejects_malformed_noncanonical_and_non_vector_arguments() {
    let mut runtime = Runtime::core();
    runtime
        .eval_native("(ns invoke.input) (defn identity* [value] value)")
        .expect("load input namespace");

    assert!(matches!(
        runtime.invoke_hta("invoke.input/identity*", b"not-hta"),
        Err(InvokeHtaError::MalformedInput(_))
    ));
    assert_eq!(
        runtime.invoke_hta(
            "invoke.input/identity*",
            &hta::encode(&Value::Number(1)).expect("scalar HTA")
        ),
        Err(InvokeHtaError::ArgumentsNotVector)
    );

    let mut noncanonical = b"HTA0".to_vec();
    noncanonical.push(11);
    noncanonical.extend_from_slice(&2_u32.to_be_bytes());
    for (key, value) in [(b'z', 1_i64), (b'a', 2_i64)] {
        noncanonical.push(6);
        noncanonical.extend_from_slice(&1_u32.to_be_bytes());
        noncanonical.push(key);
        noncanonical.push(3);
        noncanonical.extend_from_slice(&value.to_be_bytes());
    }
    assert_eq!(
        runtime.invoke_hta("invoke.input/identity*", &noncanonical),
        Err(InvokeHtaError::NoncanonicalInput)
    );
}

#[test]
fn broker_keeps_invoke_hta_session_isolated() {
    let broker = RuntimeBroker::start_core().expect("broker");
    broker
        .eval("ROOT", "(ns invoke.broker) (defn value [] 1)")
        .expect("root function");
    broker.create("SECOND").expect("second session");
    broker
        .eval("SECOND", "(ns invoke.broker) (defn value [] 2)")
        .expect("second function");

    assert_eq!(
        number(
            &broker
                .invoke_hta("ROOT", "invoke.broker/value", &arguments(vec![]))
                .expect("root invoke")
        ),
        1
    );
    assert_eq!(
        number(
            &broker
                .invoke_hta("SECOND", "invoke.broker/value", &arguments(vec![]))
                .expect("second invoke")
        ),
        2
    );
    assert_eq!(
        broker.invoke_hta("MISSING", "invoke.broker/value", &arguments(vec![])),
        Err(InvokeHtaError::SessionMissing("MISSING".to_owned()))
    );
}
