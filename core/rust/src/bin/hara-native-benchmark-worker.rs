//! One feature-built measurement worker for the native benchmark coordinator.
//!
//! It owns a single prepared runtime artifact for its full process lifetime.
//! The coordinator compares measurements only when all workers were invoked in
//! the same run, on the same host, from recorded worker identities.

use std::env;
use std::time::Instant;

use hara_native::{vm, Runtime};
use serde_json::{json, Value};

#[cfg(feature = "whole-wasm")]
use hara_native::whole_wasm::{compile_artifact, decode_artifact, NativeModule};

fn usage() -> &'static str {
    "hara-native-benchmark-worker TIER WORKLOAD SOURCE_HEX EXPECTED WINDOWS CALLS"
}

fn decode_hex(input: &str) -> Result<String, String> {
    if input.len() % 2 != 0 {
        return Err("source hex must contain an even number of characters".to_owned());
    }
    let bytes = (0..input.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&input[index..index + 2], 16).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn sample_calls(
    call: &mut impl FnMut() -> Result<String, String>,
    expected: &str,
    windows: usize,
    calls: usize,
) -> Result<Vec<u64>, String> {
    for _ in 0..3 {
        verify_result(call()?, expected)?;
    }

    let mut samples = Vec::with_capacity(windows);
    for _ in 0..windows {
        let started = Instant::now();
        for _ in 0..calls {
            verify_result(call()?, expected)?;
        }
        samples
            .push(elapsed_ns(started) / u64::try_from(calls).map_err(|error| error.to_string())?);
    }
    Ok(samples)
}

fn verify_result(actual: String, expected: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "result mismatch: expected {expected}, received {actual}"
        ))
    }
}

#[cfg(feature = "tracing-jit")]
fn telemetry_json(program: &std::rc::Rc<vm::Program>) -> Value {
    let telemetry = hara_native::bytecode_jit_telemetry(program);
    json!({
        "backend": if cfg!(feature = "native-jit") { "native" } else { "checked" },
        "backedges": telemetry.backedges,
        "compile_attempts": telemetry.compile_attempts,
        "compiled": telemetry.compiled,
        "rejected": telemetry.rejected,
        "entries": telemetry.entries,
        "completed_iterations": telemetry.completed_iterations,
        "side_exits": telemetry.side_exits,
        "recording_starts": telemetry.recording_starts,
        "recording_completed": telemetry.recording_completed,
        "recording_aborts": telemetry.recording_aborts,
        "trace_paths": telemetry.trace_paths,
        "branch_exits": telemetry.branch_exits,
        "type_exits": telemetry.type_exits,
        "error_exits": telemetry.error_exits,
        "disabled_loops": telemetry.disabled_loops,
    })
}

#[cfg(not(feature = "tracing-jit"))]
fn telemetry_json(_program: &std::rc::Rc<vm::Program>) -> Value {
    Value::Null
}

fn measure_bytecode(
    tier: &str,
    workload: &str,
    source: &str,
    expected: &str,
    windows: usize,
    calls: usize,
) -> Result<Value, String> {
    let mut runtime = Runtime::core();
    let started = Instant::now();
    let program = runtime.compile_bytecode(source)?;
    let artifact_bytes = vm::encode_program(program.as_ref())?.len();
    let prepare_ns = elapsed_ns(started);

    let mut call = || {
        runtime
            .execute_compiled_bytecode_registry_value(program.clone())
            .map(|value| value.display())
    };
    let started = Instant::now();
    let first_result = call()?;
    let first_ns = elapsed_ns(started);
    verify_result(first_result, expected)?;
    let samples_ns = sample_calls(&mut call, expected, windows, calls)?;

    Ok(json!({
        "status": "ok",
        "tier": tier,
        "workload": workload,
        "result": expected,
        "prepare_ns": prepare_ns,
        "first_ns": first_ns,
        "samples_ns": samples_ns,
        "calls_per_window": calls,
        "artifact_bytes": artifact_bytes,
        "native_entry": Value::Null,
        "jit": telemetry_json(&program),
    }))
}

#[cfg(feature = "whole-wasm")]
fn measure_whole_wasm(
    tier: &str,
    workload: &str,
    source: &str,
    expected: &str,
    windows: usize,
    calls: usize,
) -> Result<Value, String> {
    let started = Instant::now();
    let program = vm::compile_source(source).map_err(|error| error.to_string())?;
    let artifact = compile_artifact(&program)?;
    let decoded = decode_artifact(&artifact)?;
    let native_entry = decoded
        .capabilities
        .get(usize::from(decoded.program.entry))
        .copied()
        .unwrap_or(false);
    let mut module = NativeModule::load(&artifact)?;
    let prepare_ns = elapsed_ns(started);

    let mut call = || module.call_entry_i64().map(|value| value.to_string());
    let started = Instant::now();
    let first_result = call()?;
    let first_ns = elapsed_ns(started);
    verify_result(first_result, expected)?;
    let samples_ns = sample_calls(&mut call, expected, windows, calls)?;

    Ok(json!({
        "status": "ok",
        "tier": tier,
        "workload": workload,
        "result": expected,
        "prepare_ns": prepare_ns,
        "first_ns": first_ns,
        "samples_ns": samples_ns,
        "calls_per_window": calls,
        "artifact_bytes": artifact.len(),
        "native_entry": native_entry,
        "jit": Value::Null,
    }))
}

#[cfg(not(feature = "whole-wasm"))]
fn measure_whole_wasm(
    _tier: &str,
    _workload: &str,
    _source: &str,
    _expected: &str,
    _windows: usize,
    _calls: usize,
) -> Result<Value, String> {
    Err("whole-wasm worker was built without the whole-wasm feature".to_owned())
}

fn run(arguments: impl IntoIterator<Item = String>) -> Result<Value, String> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    if arguments.len() != 6 {
        return Err(usage().to_owned());
    }
    let tier = &arguments[0];
    let workload = &arguments[1];
    let source = decode_hex(&arguments[2])?;
    let expected = &arguments[3];
    let windows = arguments[4]
        .parse::<usize>()
        .map_err(|error| format!("windows must be a positive integer: {error}"))?;
    let calls = arguments[5]
        .parse::<usize>()
        .map_err(|error| format!("calls must be a positive integer: {error}"))?;
    if windows == 0 || calls == 0 {
        return Err("windows and calls must be positive".to_owned());
    }
    if tier == "whole-wasm" {
        measure_whole_wasm(tier, workload, &source, expected, windows, calls)
    } else {
        measure_bytecode(tier, workload, &source, expected, windows, calls)
    }
}

fn main() {
    match run(env::args().skip(1)) {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode_hex;

    #[test]
    fn decodes_utf8_source() {
        assert_eq!(decode_hex("282b20313920323329"), Ok("(+ 19 23)".to_owned()));
    }

    #[test]
    fn rejects_odd_length_source() {
        assert!(decode_hex("0").is_err());
    }
}
