use crate::{eval_bytecode_native, Runtime};

fn agrees(source: &str) {
    let expected = Runtime::new().eval_native(source).unwrap();
    assert_eq!(eval_bytecode_native(source).unwrap(), expected, "{source}");
}

#[test]
fn hot_arithmetic_branch_and_nested_loops_match_the_evaluator() {
    for source in [
        "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))",
        "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (if (< (mod i 2) 1) 3 7))) acc))",
        "(loop [i 0 acc 1] (if (< i 5000) (recur (+ i 1) (mod (+ acc i) 1000003)) acc))",
        "(loop [i 0 total 0] (if (< i 100) (recur (+ i 1) (+ total (loop [j 0 subtotal 0] (if (< j 100) (recur (+ j 1) (+ subtotal j)) subtotal)))) total))",
    ] {
        agrees(source);
    }
}

#[test]
fn hot_loop_overflow_deopts_to_promoted_integer_arithmetic() {
    let source = "(loop [i 0 x 1] (if (< i 30) (recur (+ i 1) (* x 1000000000)) x))";
    agrees(source);
    let value = eval_bytecode_native(source).unwrap();
    assert!(value.len() > 200, "{value}");
}

#[test]
fn compiled_traces_survive_repeated_execution_of_one_program() {
    let program =
        crate::compile_bytecode("(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))")
            .unwrap();
    assert_eq!(crate::execute_bytecode(&program).unwrap(), "12497500");
    assert!(crate::vm::machine::cached_trace_count(&program) > 0);
    assert_eq!(crate::execute_bytecode(&program).unwrap(), "12497500");
    assert!(crate::vm::machine::cached_trace_count(&program) > 0);
}

#[test]
fn telemetry_distinguishes_hot_compilation_and_execution() {
    let program =
        crate::compile_bytecode("(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))")
            .unwrap();
    assert_eq!(crate::execute_bytecode(&program).unwrap(), "12497500");
    let telemetry = crate::bytecode_jit_telemetry(&program);
    assert!(telemetry.backedges >= 16, "{telemetry:?}");
    assert_eq!(telemetry.compile_attempts, 1, "{telemetry:?}");
    assert_eq!(telemetry.compiled, 1, "{telemetry:?}");
    assert_eq!(telemetry.rejected, 0, "{telemetry:?}");
    assert!(telemetry.entries > 0, "{telemetry:?}");
}

#[test]
fn indexed_numeric_vectors_trace_from_constants_and_locals() {
    for source in [
        "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (std.protocol.inth.INth/nth [3 5 7 11] (mod i 4)))) acc))",
        "(let [values [3 5 7 11]] (loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (std.protocol.inth.INth/nth values (mod i 4)))) acc)))",
    ] {
        agrees(source);
        let program = crate::compile_bytecode(source).unwrap();
        let function = &program.functions[usize::from(program.entry)];
        let (backedge, header) = function
            .code
            .iter()
            .enumerate()
            .rev()
            .find_map(|(ip, instruction)| match instruction {
                crate::vm::Instruction::Jump(target) if usize::try_from(*target).ok()? <= ip => {
                    Some((ip as u32, *target))
                }
                _ => None,
            })
            .unwrap();
        let mut locals = vec![crate::jit::TraceValue::I64(64); usize::from(function.local_count)];
        if source.starts_with("(let") {
            locals[0] = crate::jit::TraceValue::Indexed(Box::new(crate::core::Value::Vector(
                [3, 5, 7, 11]
                    .into_iter()
                    .map(crate::core::Value::Number)
                    .collect(),
            )));
        }
        let recorded = crate::jit::TraceRecorder::new(4096).record_loop(
            &program,
            program.entry,
            header,
            backedge,
            &locals,
        );
        assert!(
            recorded.is_ok(),
            "vector loop was rejected: {recorded:?}; code: {:?}; constants: {:?}",
            function.code, program.constants
        );
        assert_eq!(crate::execute_bytecode(&program).unwrap(), "32500");
        assert!(
            crate::vm::machine::cached_trace_count(&program) > 0,
            "vector loop did not compile: {source}"
        );
    }
}

#[test]
fn unsupported_vectors_and_late_bounds_errors_fall_back_to_vm_semantics() {
    agrees(
        "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (std.protocol.icount.ICount/count (std.protocol.inth.INth/nth [\"ab\"] 0)))) acc))",
    );

    let values = (0..256)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!("(loop [i 0] (if (< i 5000) (do (std.protocol.inth.INth/nth [{values}] i) (recur (+ i 1))) i))");
    let evaluator = Runtime::new().eval_native(&source).unwrap_err();
    let vm = eval_bytecode_native(&source).unwrap_err();
    assert!(evaluator.contains("nth index out of bounds"), "{evaluator}");
    assert!(vm.contains("nth index out of bounds"), "{vm}");
}

#[test]
fn unsupported_primary_path_disables_repeated_trace_collection() {
    let source = "(loop [i 0 value {}] (if (< i 500) (recur (+ i 1) (std.protocol.iassoc.IAssoc/assoc value i (+ i 1))) (std.protocol.ilookup.ILookup/lookup value 499)))";
    let program = crate::compile_bytecode(source).unwrap();

    assert_eq!(crate::execute_bytecode(&program).unwrap(), "500");
    let first = crate::bytecode_jit_telemetry(&program);
    assert_eq!(first.compile_attempts, 1, "{first:?}");
    assert_eq!(first.compiled, 0, "{first:?}");
    assert_eq!(first.rejected, 1, "{first:?}");
    assert_eq!(first.disabled_loops, 1, "{first:?}");

    assert_eq!(crate::execute_bytecode(&program).unwrap(), "500");
    let second = crate::bytecode_jit_telemetry(&program);
    assert_eq!(
        second.compile_attempts, first.compile_attempts,
        "{second:?}"
    );
    assert_eq!(second.rejected, first.rejected, "{second:?}");
    assert_eq!(second.disabled_loops, first.disabled_loops, "{second:?}");
}

#[test]
fn dynamic_paths_compile_both_directions_of_an_alternating_branch() {
    let source = "(loop [i 0 flag true acc 0] (if (< i 5000) (if flag (recur (+ i 1) false (+ acc 3)) (recur (+ i 1) true (+ acc 7))) acc))";
    agrees(source);
    let program = crate::compile_bytecode(source).unwrap();
    assert_eq!(crate::execute_bytecode(&program).unwrap(), "25000");
    let telemetry = crate::bytecode_jit_telemetry(&program);
    assert!(
        telemetry.trace_paths >= 2,
        "{telemetry:?}\n{}",
        crate::vm::disassemble(&program)
    );
    assert!(telemetry.branch_exits > 0, "{telemetry:?}");
    assert_eq!(telemetry.disabled_loops, 1, "{telemetry:?}");
}

#[test]
fn division_and_numeric_sequence_navigation_trace() {
    for (source, expected) in [
        (
            "(loop [i 1 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (/ i 3))) acc))",
            "4164167",
        ),
        (
            "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (std.protocol.icount.ICount/count [3 5 7 11]))) acc))",
            "20000",
        ),
        (
            "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (std.protocol.inth.INth/nth [3 5 7 11] 1))) acc))",
            "25000",
        ),
    ] {
        agrees(source);
        let program = crate::compile_bytecode(source).unwrap();
        assert_eq!(crate::execute_bytecode(&program).unwrap(), expected);
        assert!(
            crate::bytecode_jit_telemetry(&program).compiled > 0,
            "loop did not compile: {source}"
        );
    }
}

#[test]
fn divide_and_remainder_edges_deopt_to_exact_semantics() {
    let division = "(loop [i 0 x -9223372036854775808] (if (< i 100) (recur (+ i 1) (/ x -1)) x))";
    agrees(division);
    assert_eq!(
        eval_bytecode_native(division).unwrap(),
        "-9223372036854775808"
    );

    let modulo = "(loop [i 0 x -9223372036854775808] (if (< i 100) (recur (+ i 1) (mod x -1)) x))";
    agrees(modulo);
    assert_eq!(eval_bytecode_native(modulo).unwrap(), "0");
}
