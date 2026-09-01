//! End-to-end compiler and synchronous-machine execution tests, including
//! control flow, arithmetic, loop/recur behavior, and source diagnostics.

use super::error::CompileErrorKind;
use super::{
    compile_source, compile_source_with, disassemble, eval_source, execute_program,
    execute_program_with_globals,
};
use crate::core::{Promise, PromiseState, Value};
use crate::kernel::NamespaceRegistry;
use crate::Runtime;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[path = "execution_tests/bindings.rs"]
mod bindings;

fn eval(source: &str) -> String {
    eval_native(source)
}

fn eval_error(source: &str) -> String {
    let registry = crate::embedding_namespace_registry();
    match compile_source_with(source, &registry) {
        Ok(program) => execute_program_with_globals(Rc::new(program), &registry)
            .expect_err("evaluation must fail")
            .to_string(),
        Err(error) => error.to_string(),
    }
}

/// The standalone native host has no embedded HAL prelude. Test expressions
/// therefore use explicit std.native and std.protocol paths against its
/// embedding registry.
fn eval_native(source: &str) -> String {
    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with(source, &registry).expect("evaluation must compile");
    execute_program_with_globals(Rc::new(program), &registry)
        .map(|value| value.display())
        .expect("evaluation must succeed")
}

fn native_struct_source(body: &str) -> String {
    format!(
        "(let [Point (std.native.Base/struct \
                         (std.native.Base/current-namespace) \
                         (std.native.Base/symbol \"Point\") \
                         (std.native.Base/vector \
                           (std.native.Base/symbol \"x\") \
                           (std.native.Base/symbol \"y\"))) \
                map->Point (std.protocol.ideref.IDeref/deref \
                             (std.native.Base/resolve \
                               (std.native.Base/symbol \"map->Point\")))] \
           {body})"
    )
}

fn eval_native_struct(body: &str) -> String {
    eval(&native_struct_source(body))
}

fn native_mutable_source(body: &str) -> String {
    format!(
        "(let [Cursor (std.native.Base/mutable \
                          (std.native.Base/current-namespace) \
                          (std.native.Base/symbol \"Cursor\") \
                          (std.native.Base/vector \
                            (std.native.Base/symbol \"x\") \
                            (std.native.Base/symbol \"y\"))) \
                map->Cursor (std.protocol.ideref.IDeref/deref \
                              (std.native.Base/resolve \
                                (std.native.Base/symbol \"map->Cursor\")))] \
           {body})"
    )
}

fn eval_native_mutable(body: &str) -> String {
    eval(&native_mutable_source(body))
}

#[test]
fn protocol_count_executes_in_bytecode() {
    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with(
        "(std.protocol.icount.ICount/count [1 2 3 4])",
        &registry,
    )
    .expect("protocol count compiles against the native registry");
    assert_eq!(
        execute_program_with_globals(Rc::new(program), &registry).unwrap(),
        Value::Number(4)
    );
}

#[test]
fn native_string_method_compiles_without_a_source_alias() {
    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with("(std.native.String/length \"hara\")", &registry)
        .expect("native String methods compile against the registry");
    assert_eq!(
        execute_program_with_globals(Rc::new(program), &registry)
            .unwrap()
            .display(),
        "4"
    );
}

#[test]
fn assoc_accepts_a_bytecode_closure_as_the_replacement() {
    assert_eq!(
        eval("(let [f (fn [value] (+ value 1)) m (std.protocol.iassoc.IAssoc/assoc {} :f f)] ((std.protocol.ilookup.ILookup/lookup m :f) 41))"),
        "42"
    );
}

/// Runtime errors append `(instruction NNNN)` to the display; compare the
/// stable message-and-position prefix.
fn assert_eval_error(source: &str, expected_prefix: &str) {
    let message = eval_error(source);
    assert!(
        message.starts_with(expected_prefix),
        "{source}: {message} does not start with {expected_prefix}"
    );
}

fn compile_error(source: &str) -> (CompileErrorKind, String) {
    match compile_source(source) {
        Ok(program) => panic!("expected compile error, got {}", disassemble(&program)),
        Err(error) => (error.kind(), error.to_string()),
    }
}

#[test]
fn literals() {
    assert_eq!(eval("nil"), "nil");
    assert_eq!(eval("true"), "true");
    assert_eq!(eval("false"), "false");
    assert_eq!(eval("42"), "42");
    assert_eq!(eval("-7"), "-7");
    assert_eq!(eval("1.5"), "(double 1.5)");
    assert_eq!(eval("2.0"), "(double 2)");
    assert_eq!(eval("\"hello\""), "\"hello\"");
    assert_eq!(eval(":hara/name"), ":hara/name");
    assert_eq!(eval("\\a"), "\\a");
    assert!(compile_source("42N").is_err());
    assert!(compile_source("1.25M").is_err());
    assert_eq!(eval("#\"\\d+\""), "#\"\\d+\"");
    assert_eq!(eval("()"), "()");
    assert_eq!(eval("^:private (+ 1 2)"), "3");
}

#[test]
fn mutable_reader_tags_construct_lookup_collections() {
    assert_eq!(
        eval(
            "(let [array #arr[1 (+ 1 1)]\
                   object #obj{\"answer\" (+ 40 2)}]\
               [(std.protocol.ilookup.ILookup/lookup array 1)\
                (std.protocol.ilookup.ILookup/lookup array 9 :missing)\
                (std.protocol.ilookup.ILookup/lookup object \"answer\")\
                (std.protocol.ilookup.ILookup/lookup object \"missing\" :missing)\
                (do (std.native.Arr/set array 0 7)\
                    (std.protocol.ilookup.ILookup/lookup array 0))\
                (do (std.native.Obj/set object \"answer\" 43)\
                    (std.protocol.ilookup.ILookup/lookup object \"answer\"))])"
        ),
        "[2 :missing 42 :missing 7 43]"
    );
}

#[test]
fn mutable_reader_tags_round_trip_through_display() {
    assert_eq!(
        eval("[#arr[1 (+ 1 1)] #obj {\"answer\" (+ 40 2)} #obj {}]"),
        "[#arr[1 2] #obj{\"answer\" 42} #obj{}]"
    );
}

#[test]
fn uuid_reader_tag_constructs_and_displays_uuid_values() {
    assert_eq!(
        eval(
            "[(= #uuid \"00000000-0000-0000-0000-000000000000\" \
                 (std.native.Base/uuid \"00000000-0000-0000-0000-000000000000\")) \
              (std.native.Base/type #uuid \"00000000-0000-0000-0000-000000000000\") \
              #uuid \"00000000-0000-0000-0000-000000000000\"]"
        ),
        "[true :std.native.UUID #uuid \"00000000-0000-0000-0000-000000000000\"]"
    );
    assert!(compile_source("#uuid :not-a-string").is_err());
}

#[test]
fn dynamic_collections_and_short_circuit_forms() {
    assert_eq!(eval("(let [x 19 y 23] [x y])"), "[19 23]");
    assert_eq!(
        eval(
            "[(std.native.Base/type []) \
               (= :std.native.Vector (std.native.Base/type [])) \
               (= :std.native.MapEntry (std.native.Base/type [1 2])) \
               (std.native.Base/type [1 2 3 4 5 6 7 8]) \
               (= :std.native.MapEntry (std.native.Base/type (std.native.Base/map-entry 1 2))) \
               (std.native.Base/type [1 2 3 4 5 6 7 8 9]) \
               (= :std.native.Vector (std.native.Base/type [1 2 3 4 5 6 7 8 9])) \
               (= :std.native.MapEntry (std.native.Base/type [1 2 3 4 5 6 7 8 9]))]"
        ),
        "[:std.native.Vector true false :std.native.Vector true :std.native.Vector true false]"
    );
    assert_eq!(
        eval("[(std.protocol.ilookup.ILookup/lookup [1 2] 1) (std.protocol.ilookup.ILookup/lookup [] 0 :missing)]"),
        "[2 :missing]"
    );
    assert_eq!(eval("(let [x 42] {:answer x})"), "{:answer 42}");
    assert_eq!(eval("(let [x 42] #{x 1})"), "#{42 1}");
    assert_eq!(eval("(and true 42)"), "42");
    assert_eq!(eval("(and 19 false (/ 1 0))"), "false");
    assert_eq!(eval("(or nil false 42)"), "42");
    assert_eq!(eval("(or 42 (/ 1 0))"), "42");
    assert_eq!(eval("(cond false 1 (= 1 1) 42 :else 0)"), "42");
    assert_eq!(eval("'(a [1 2])"), "(a [1 2])");
}

#[test]
fn compiled_execution_can_return_an_immutable_value_directly() {
    let mut runtime = Runtime::core();
    let program = runtime
        .compile_bytecode("{:answer 42}")
        .expect("map must compile");
    let result = runtime
        .execute_compiled_bytecode_value(program)
        .expect("map must execute");

    assert!(matches!(
        result,
        Value::Map(_) | Value::OrderedMap(_) | Value::SortedMap(_) | Value::Trie(_)
    ));
    assert_eq!(result.display(), "{:answer 42}");
}

#[test]
fn registry_only_execution_remains_visible_to_later_interpreter_entries() {
    let mut runtime = Runtime::core();
    let program = runtime
        .compile_bytecode("(def prepared-answer 42)")
        .expect("definition must compile");
    let definition = runtime
        .execute_compiled_bytecode_registry_value(program)
        .expect("definition must execute");
    assert_eq!(definition.display(), "#'user/prepared-answer");

    // eval_native refreshes from the authoritative namespace registry, so
    // omitting the eager compatibility copy does not make definitions stale.
    assert_eq!(runtime.eval_native("prepared-answer"), Ok("42".into()));
}

#[test]
fn runtime_native_array_and_object_calls_lower_to_vm_primitives() {
    let mut runtime = Runtime::core();
    let source = "(let [a (std.native.Arr/new 1 2) \
                        o (std.native.Obj/new \"count\" 3)] \
                    (std.native.Arr/set a 0 7) \
                    (std.native.Obj/set o \"count\" 11) \
                    [(std.native.Arr/get a 0) \
                     (std.native.Obj/get o \"count\") \
                     (std.native.Base/number? (std.native.Arr/get a 0))])";
    let program = runtime
        .compile_bytecode(source)
        .expect("native calls must compile");
    let disassembly = crate::vm::disassemble(&program);
    assert!(disassembly.matches("IntrinsicCall target").count() >= 7, "{disassembly}");
    assert!(!disassembly.contains("GetGlobal"), "{disassembly}");
    assert_eq!(
        runtime
            .execute_compiled_bytecode_registry_value(program)
            .map(|value| value.display()),
        Ok("[7 11 true]".into())
    );
}

#[test]
fn runtime_bytecode_defmacro_registers_and_expands() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime.eval_bytecode_native("(defmacro unless [test body] `(if ~test nil ~body))"),
        Ok("<fn>".into())
    );
    assert_eq!(
        runtime.eval_bytecode_native("(unless false 42)"),
        Ok("42".into())
    );
    assert_eq!(runtime.eval_native("(unless false 42)"), Ok("42".into()));
}

#[test]
fn base_def_macro_registers_and_expands_from_its_explicit_namespace() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime.eval_bytecode_native(
            "(let [target (std.native.Base/namespace 'example.macro)] \
             (std.native.Base/def target 'identity-form \
               (fn [form environment value] value) {:macro true}))",
        ),
        Ok("#'example.macro/identity-form".into())
    );
    assert_eq!(
        runtime.eval_bytecode_native("(example.macro/identity-form 42)"),
        Ok("42".into())
    );
}

#[test]
fn bytecode_variadic_macro_forwards_rest_to_helper_in_order() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime.eval_bytecode_native(
            "(defn third-form [forms] \
               (std.protocol.ipeekfirst.IPeekFirst/peek-first \
                 (std.protocol.ipopfirst.IPopFirst/pop-first \
                   (std.protocol.ipopfirst.IPopFirst/pop-first forms))))",
        ),
        Ok("#'user/third-form".into())
    );
    assert_eq!(
        runtime.eval_bytecode_native("(defmacro choose-third [& forms] (third-form forms))"),
        Ok("<fn>".into())
    );
    assert_eq!(
        runtime.eval_bytecode_native("(choose-third nil nil (+ 19 23))"),
        Ok("42".into())
    );
}

#[test]
fn multiple_top_level_forms() {
    assert_eq!(eval("1 2 3"), "3");
    assert_eq!(eval("(+ 1 2) (+ 3 4)"), "7");
}

#[test]
fn if_branches() {
    assert_eq!(eval("(if true 1 2)"), "1");
    assert_eq!(eval("(if false 1 2)"), "2");
    assert_eq!(eval("(if nil 1 2)"), "2");
    // Everything except nil and false is truthy, including 0 and "".
    assert_eq!(eval("(if 0 1 2)"), "1");
    assert_eq!(eval("(if \"\" 1 2)"), "1");
    assert_eq!(eval("(if false 1)"), "nil");
    assert_eq!(eval("(if (< 19 20) 42 0)"), "42");
}

#[test]
fn do_sequences() {
    assert_eq!(eval("(do)"), "nil");
    assert_eq!(eval("(do 1)"), "1");
    assert_eq!(eval("(do 1 2 3)"), "3");
    assert_eq!(eval("(do (do 1 2) (do 3 4))"), "4");
}

#[test]
fn arithmetic() {
    assert_eval_error("(+)", "+ expects arguments [line 1, column 1]");
    assert_eq!(eval("(+ 19 23)"), "42");
    assert_eq!(eval("(+ 1 2 3 4)"), "10");
    assert_eq!(eval("(+ 5)"), "5");
    assert_eq!(eval("(- 10 3)"), "7");
    assert_eq!(eval("(* 6 7)"), "42");
    assert_eval_error("(*)", "* expects arguments [line 1, column 1]");
    assert_eq!(eval("(/ 2)"), "0");
    assert_eq!(eval("(/ 17 5)"), "3");
    assert_eq!(eval("(/ -17 5)"), "-3");
    assert_eq!(eval("(mod 17 5)"), "2");
    assert_eq!(eval("(mod -7 3)"), "-1");
    assert_eq!(eval("(mod 7 -3)"), "1");
}

#[test]
fn arithmetic_errors() {
    assert_eval_error("(-)", "- expects arguments [line 1, column 1]");
    assert_eval_error("(/)", "/ expects arguments [line 1, column 1]");
    assert_eval_error("(/ 1 0)", "division by zero [line 1, column 1]");
    assert_eval_error("(% 1 0)", "unbound symbol: % [line 1, column 1]");
    assert_eval_error("(mod 1 0)", "division by zero [line 1, column 1]");
    assert_eq!(eval("(+ 9223372036854775807 1)"), "9223372036854775808");
    assert_eq!(eval("(- -9223372036854775808 1)"), "-9223372036854775809");
    assert_eq!(eval("(* 9223372036854775807 2)"), "18446744073709551614");
    assert_eval_error("(+ 1 \"a\")", "+ expects numbers [line 1, column 1]");
    assert_eq!(eval("(+ 1 1.5)"), "(double 2.5)");
    assert_eval_error("(mod \"a\" 1)", "mod expects numbers [line 1, column 1]");
}

#[test]
fn comparisons() {
    assert_eq!(eval("(< 1 2)"), "true");
    assert_eq!(eval("(< 2 1)"), "false");
    assert_eq!(eval("(< 1 2 3)"), "true");
    assert_eq!(eval("(< 1 3 2)"), "false");
    assert_eq!(eval("(<= 1 1)"), "true");
    assert_eq!(eval("(> 2 1)"), "true");
    assert_eq!(eval("(>= 2 3)"), "false");
}

#[test]
fn comparison_errors() {
    assert_eval_error(
        "(< 1)",
        "< expects at least two arguments [line 1, column 1]",
    );
    assert_eval_error("(< 1 \"a\")", "< expects numbers [line 1, column 1]");
    assert_eval_error("(= 1)", "= expects at least 2 arguments [line 1, column 1]");
}

#[test]
fn equality() {
    assert_eq!(eval("(= 1 1)"), "true");
    assert_eq!(eval("(= 1 2)"), "false");
    assert_eq!(eval("(= 1 1 1 1)"), "true");
    assert_eq!(eval("(= nil nil)"), "true");
    assert_eq!(eval("(= nil false)"), "false");
    assert_eq!(eval("(= \"a\" \"a\")"), "true");
    assert_eq!(eval("(= :a :a)"), "true");
    assert_eq!(eval("(= \\a \\a)"), "true");
    assert_eq!(eval("(= 1.5 1.5)"), "true");
    assert_eq!(eval("(= 1 1.0)"), "true");
    assert_eq!(eval("(= 1 9223372036854775808)"), "false");
    assert_eq!(
        eval("(= 9223372036854775808 9223372036854775808.0)"),
        "true"
    );
}

#[test]
fn loop_zero_iterations() {
    assert_eq!(eval("(loop [i 0] (if (< i 0) (recur (+ i 1)) i))"), "0");
}

#[test]
fn loop_iterations() {
    assert_eq!(eval("(loop [i 0] (if (< i 1) (recur (+ i 1)) i))"), "1");
    assert_eq!(eval("(loop [i 0] (if (< i 100) (recur (+ i 1)) i))"), "100");
}

#[test]
fn loop_multiple_bindings() {
    assert_eq!(
        eval("(loop [i 0 acc 0] (if (< i 10) (recur (+ i 1) (+ acc i)) acc))"),
        "45"
    );
}

#[test]
fn recur_updates_are_simultaneous() {
    // Each iteration must compute both new values from the old bindings.
    assert_eq!(
        eval("(loop [x 0 y 1] (if (< x 3) (recur (+ x 1) (+ x y)) y))"),
        "4"
    );
    // Swapping two bindings through recur: one swap exchanges them.
    assert_eq!(
        eval("(loop [x 1 y 2 n 0] (if (< n 1) (recur y x (+ n 1)) (- x y)))"),
        "1"
    );
}

#[test]
fn nested_loops() {
    // Inner loop sums i*j for j in 0..3 per outer step: 3i; total 18.
    assert_eq!(
        eval("(loop [i 0 t 0] (if (< i 4) (recur (+ i 1) (+ t (loop [j 0 s 0] (if (< j 3) (recur (+ j 1) (+ s (* i j))) s)))) t))"),
        "18"
    );
}

#[test]
fn loop_body_sequences_like_do() {
    assert_eq!(eval("(loop [i 0] 1 2)"), "2");
    assert_eq!(eval("(loop [i 0] (+ i 1) i)"), "0");
    assert_eq!(eval("(loop [] 7)"), "7");
}

#[test]
fn recur_through_tail_positions() {
    // Tail `let` and `do` bodies and `if` branches are recur positions.
    assert_eq!(
        eval("(loop [i 0] (let [next (+ i 1)] (if (< i 5) (do (recur next)) i)))"),
        "5"
    );
}

#[test]
fn recur_errors() {
    let (kind, message) = compile_error("(recur 1)");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(message.contains("recur must be inside loop"), "{message}");
    assert!(message.contains("[line 1, column 1]"), "{message}");

    let (kind, message) = compile_error("(recur)");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(message.contains("recur must be inside loop"), "{message}");

    assert_eq!(eval("(loop [] 42)"), "42");

    let (kind, message) = compile_error("(loop [i 0] (recur 1 2))");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(message.contains("loop recur arity mismatch"), "{message}");

    let (kind, message) = compile_error("(loop [i 0] (+ 1 (recur 2)))");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(
        message.contains("recur must be in tail position"),
        "{message}"
    );

    let (kind, message) = compile_error("(loop [i 0] (if (recur 1) 2 3))");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(
        message.contains("recur must be in tail position"),
        "{message}"
    );

    let (kind, message) = compile_error("(loop [i 0] (do (recur 1) i))");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(
        message.contains("recur must be in tail position"),
        "{message}"
    );
}

#[test]
fn compile_arity_errors_match_evaluator_messages() {
    for (source, expected) in [
        ("(if)", "if expects 2 or 3 arguments [line 1, column 1]"),
        (
            "(if 1 2 3 4)",
            "if expects 2 or 3 arguments [line 1, column 1]",
        ),
        (
            "(let)",
            "let expects bindings and a body [line 1, column 1]",
        ),
        (
            "(let [x 1])",
            "let expects bindings and a body [line 1, column 1]",
        ),
        (
            "(let 1 x)",
            "let expects a binding list or vector [line 1, column 6]",
        ),
        (
            "(let [x] x)",
            "let bindings require name/value pairs [line 1, column 6]",
        ),
        (
            "(loop [i 0])",
            "loop expects bindings and a body [line 1, column 1]",
        ),
        (
            "(loop 1 2)",
            "loop expects a binding list or vector [line 1, column 7]",
        ),
        (
            "(loop [i] i)",
            "loop bindings require name/value pairs [line 1, column 7]",
        ),
    ] {
        let (kind, message) = compile_error(source);
        assert_eq!(kind, CompileErrorKind::Arity, "{source}");
        assert_eq!(message, expected, "{source}");
    }
}

#[test]
fn unbound_symbols_are_compile_errors_with_positions() {
    let (kind, message) = compile_error("unknown");
    assert_eq!(kind, CompileErrorKind::UnboundSymbol);
    assert_eq!(message, "unbound symbol: unknown [line 1, column 1]");
    let (kind, message) = compile_error("(let [x 1] (+ x y))");
    assert_eq!(kind, CompileErrorKind::UnboundSymbol);
    assert_eq!(message, "unbound symbol: y [line 1, column 17]");
    assert_eq!(
        eval("(std.protocol.ipeekfirst.IPeekFirst/peek-first [1 2])"),
        "1"
    );
}

#[test]
fn literal_collections_and_collection_primitives() {
    assert_eq!(eval("[1 2 3]"), "[1 2 3]");
    assert_eq!(eval("{:a 1}"), "{:a 1}");
    assert_eq!(eval("#{1 2}"), "#{1 2}");
    assert_eq!(eval("(std.protocol.inth.INth/nth [10 20 30] 1)"), "20");
    assert_eq!(
        eval("(std.protocol.iassoc.IAssoc/assoc {} :answer 42)"),
        "{:answer 42}"
    );
    assert_eq!(
        eval("(let [before {:a 1} after (std.protocol.iassoc.IAssoc/assoc before :b 2)] (+ (if (= nil (std.protocol.ilookup.ILookup/lookup before :b)) 40 0) (std.protocol.ilookup.ILookup/lookup after :b)))"),
        "42"
    );
    assert_eq!(
        eval("(std.protocol.ipeekfirst.IPeekFirst/peek-first (std.protocol.ipopfirst.IPopFirst/pop-first [1 2]))"),
        "2"
    );
}

#[test]
fn tail_recur_assoc_moves_dead_local_without_mutating_persistent_aliases() {
    assert_eq!(
        eval(
            "(let [original {:seed 1}
                   built (loop [i 0 value original]
                           (if (< i 500)
                             (recur (+ i 1) (std.protocol.iassoc.IAssoc/assoc value i (+ i 1)))
                             value))]
               [(std.protocol.icount.ICount/count original) (std.protocol.ilookup.ILookup/lookup original 499) (std.protocol.icount.ICount/count built) (std.protocol.ilookup.ILookup/lookup built 499)])"
        ),
        "[1 nil 501 500]"
    );
}

#[test]
fn mutable_collections_build_in_place_and_freeze_once() {
    assert_eq!(
        eval(
            "(let [m (std.protocol.itomutable.IToMutable/to-mutable {})]
                (do
                  (loop [i 0]
                    (if (< i 500)
                      (do (std.protocol.iassoc.IAssoc/assoc m i (+ i 1)) (recur (+ i 1)))
                      nil))
                  (let [p (std.protocol.itopersistent.IToPersistent/to-persistent m)]
                    (+ (std.protocol.icount.ICount/count p) (std.protocol.ilookup.ILookup/lookup p 499)))))"
        ),
        "1000"
    );
    assert_eval_error(
        "(let [m (std.protocol.itomutable.IToMutable/to-mutable {}) p (std.protocol.itopersistent.IToPersistent/to-persistent m)] (do p (std.protocol.iassoc.IAssoc/assoc m :late 1)))",
        "mutable collection used after to-persistent",
    );
}

#[test]
fn mutable_conversion_is_not_constant_folded_across_executions() {
    let program = Rc::new(
        compile_source_with(
            "(loop [i 0 m (std.protocol.itomutable.IToMutable/to-mutable {})]
           (if (< i 10)
             (recur (+ i 1) (std.protocol.iassoc.IAssoc/assoc m i (+ i 1)))
             (std.protocol.ilookup.ILookup/lookup (std.protocol.itopersistent.IToPersistent/to-persistent m) 9)))",
            &crate::embedding_namespace_registry(),
        )
        .unwrap(),
    );
    let registry = crate::embedding_namespace_registry();
    assert_eq!(
        execute_program_with_globals(program.clone(), &registry)
            .unwrap()
            .display(),
        "10"
    );
    assert_eq!(
        execute_program_with_globals(program, &registry).unwrap().display(),
        "10"
    );
}

#[test]
fn fn_values_and_direct_calls() {
    assert_eq!(eval("(fn [x] x)"), "<fn>");
    assert_eq!(eval("((fn [x] x) 1)"), "1");
    assert_eq!(eval("((fn [x y] (+ x y)) 19 23)"), "42");
    assert_eq!(eval("(let [f (fn [x] (+ x 1))] (f 41))"), "42");
    assert_eq!(eval("(let [f (fn [x] x)] (= f f))"), "true");
    assert_eq!(eval("(= (fn [x] x) (fn [x] x))"), "false");
    // Zero-argument functions.
    assert_eq!(eval("((fn [] 42))"), "42");
}

#[test]
fn immediate_fixed_arity_closures_inline_into_lexical_slots() {
    let program = compile_source("((fn [x] (+ x 1)) 41)").expect("compiles");
    let listing = disassemble(&program);
    assert!(!listing.contains("Closure"), "{listing}");
    assert!(!listing.contains("\nCall "), "{listing}");
    assert!(listing.contains("StoreLocal"), "{listing}");
    assert_eq!(eval("((fn [x] (+ x 1)) 41)"), "42");
    assert_eq!(eval("(let [x 40] ((fn [x y] (+ x y)) 19 23))"), "42");
    // Arguments resolve before the inlined parameter scope is introduced.
    assert_eq!(eval("(let [x 20] ((fn [x y] (+ x y)) 19 (+ x 3)))"), "42");
    // A recur nested in the function body retains its own call boundary.
    assert_eq!(
        eval("((fn [n] (loop [i n] (if (< i 1) 42 (recur (- i 1))))) 10000)"),
        "42"
    );
}

#[test]
fn closures_capture_lexical_environment() {
    assert_eq!(eval("(let [x 19] ((fn [y] (+ x y)) 23))"), "42");
    // Captures are by value at closure-creation time.
    assert_eq!(eval("(let [x 1 f (fn [] x)] (let [x 2] (+ (f) x)))"), "3");
    // Nested closures capture through intermediate scopes.
    assert_eq!(eval("(((fn [x] (fn [y] (+ x y))) 19) 23)"), "42");
    // Loop bindings are capturable.
    assert_eq!(
        eval("(loop [i 0 acc 0] (if (< i 5) (recur (+ i 1) ((fn [x] (+ x i)) acc)) acc))"),
        "10"
    );
}

#[test]
fn defn_lowering_binds_direct_calls() {
    assert_eq!(eval("(do (defn f [x] (+ x 1)) (f 41))"), "42");
    // Later defns shadow earlier ones under early binding.
    assert_eq!(
        eval("(do (defn f [x] (+ x 1)) (defn f [x] (+ x 2)) (f 40))"),
        "42"
    );
    // A defn body sees earlier defns.
    assert_eq!(
        eval("(do (defn g [x] (* x 2)) (defn h [x] (+ (g x) 1)) (h 20))"),
        "41"
    );
    // Self-recursion compiles to a direct static call.
    assert_eq!(
        eval("(do (defn countdown [n] (if (< n 1) 0 (+ 1 (countdown (- n 1))))) (countdown 100))"),
        "100"
    );
    let program = compile_source(
        "(do (defn countdown [n] (if (< n 1) 0 (countdown (- n 1)))) (countdown 10))",
    )
    .unwrap();
    let listing = disassemble(&program);
    assert!(listing.contains("CallStatic 0001 1"), "{listing}");
}

#[test]
fn inline_metadata_lowers_forwarding_calls_to_the_declared_target() {
    let program = compile_source(
        "(do (defn target [x] (+ x 1)) (defn ^:inline shim [x] (target x)) (shim 41))",
    )
    .unwrap();
    let listing = disassemble(&program);
    assert_eq!(listing.matches("GetGlobal 2").count(), 2, "{listing}");
    assert!(!listing.contains("GetGlobal 3"), "{listing}");
    assert_eq!(
        eval("(do (defn target [x] (+ x 1)) (defn ^:inline shim [x] (target x)) (shim 41))"),
        "42"
    );
}

#[test]
fn comment_compiles_to_nil_without_compiling_its_contents() {
    assert_eq!(
        eval("(comment missing-symbol (throw (ex :generic {} :ex/message \"boom\")) (def leaked 1))"),
        "nil"
    );
    assert!(compile_source("(do (comment (def leaked 1)) leaked)").is_err());
}

#[test]
fn native_result_calls_execute_in_bytecode() {
    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with(
        "[(= (std.native.Base/type (std.native.Result/create :success 42)) :std.native.Result) \
          (std.native.Result/status (std.native.Result/create :success 42)) \
          (std.native.Result/data (std.native.Result/create :success 42))]",
        &registry,
    )
    .expect("Result native methods compile against the embedded registry");
    assert_eq!(
        execute_program_with_globals(Rc::new(program), &registry)
            .unwrap()
            .display(),
        "[true :success 42]",
    );
}

#[test]
fn vm_global_recursion_uses_stackless_frames() {
    assert_eq!(
        eval("(do (defn countdown [n] (if (< n 1) 0 (countdown (- n 1)))) (countdown 10000))"),
        "0"
    );
}

#[test]
fn call_errors() {
    // Arity mismatch reports through the shared native-function boundary.
    assert_eval_error(
        "((fn [x] x) 1 2)",
        "function expects 1 arguments [line 1, column 1]",
    );
    // Calling a non-function value.
    assert_eval_error("(1 2)", "value is not callable [line 1, column 1]");
}

#[test]
fn fn_shape_errors_are_compile_errors() {
    let (kind, message) = compile_error("(fn x x)");
    assert_eq!(kind, CompileErrorKind::UnsupportedForm);
    assert!(
        message.contains("function parameters must be a vector"),
        "{message}"
    );
}

#[test]
fn parse_errors_are_compile_errors() {
    let (kind, message) = compile_error("(+ 1");
    assert_eq!(kind, CompileErrorKind::Parse);
    assert!(message.contains("EOF while reading list"), "{message}");
}

#[test]
fn runtime_errors_carry_instruction_and_position() {
    let program =
        compile_source("(+ 1 2) (loop [i 0] (if (< i 3) (recur (/ 1 0)) i))").expect("compiles");
    let error = execute_program(std::rc::Rc::new(program)).expect_err("division by zero");
    let text = error.to_string();
    // The runtime error points at the failing primitive call, not the
    // enclosing `recur`.
    assert!(
        text.starts_with("division by zero [line 1, column 40]"),
        "{text}"
    );
    assert!(text.contains("(instruction"), "{text}");
    let position = error.position.expect("source position");
    assert_eq!((position.line, position.column), (1, 40));
}

#[test]
fn loop_workload_executes() {
    assert_eq!(
        eval("(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (mod i 17))) acc))"),
        "39985"
    );
}

#[test]
fn multiline_source_positions() {
    let (_, message) = compile_error("(let [x 1]\n  (+ x y))");
    assert!(message.contains("[line 2, column 8]"), "{message}");
}

#[test]
fn compiled_programs_are_reusable() {
    let program = std::rc::Rc::new(compile_source("(let [x 19 y 23] (+ x y))").expect("compiles"));
    for _ in 0..3 {
        let value = execute_program(program.clone()).expect("executes");
        assert!(matches!(value, Value::Number(42)));
    }
}

#[test]
fn declare_supplies_forward_visibility_only() {
    assert_eq!(eval("(declare answer)"), "nil");
    assert_eq!(
        eval("(declare answer) (defn answer [n] (+ n 1)) (answer 41)"),
        "42"
    );
    // declare is top-level only and takes name symbols.
    let (_, message) = compile_error("(let [x 1] (declare y) x)");
    assert!(
        message.contains("declare is only supported as a top-level statement"),
        "{message}"
    );
    let (_, message) = compile_error("(declare 1)");
    assert!(
        message.contains("declare expects name symbols"),
        "{message}"
    );
}

#[test]
fn workload_disassembly_is_deterministic() {
    let source = "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (mod i 17))) acc))";
    let first = disassemble(&compile_source(source).expect("compiles"));
    let second = disassemble(&compile_source(source).expect("compiles"));
    assert_eq!(first, second);
    assert!(first.contains("JumpIfFalse ->"), "{first}");
    assert!(first.contains("StoreLocal 1"), "{first}");
    assert!(first.contains("IntrinsicCall target"), "{first}");
}

// ------------------------------------------------------------------
// Exceptions (issue #203): try/catch/finally and guest throw.
// ------------------------------------------------------------------

#[test]
fn throw_and_catch_basics() {
    assert_eq!(
        eval("(try (throw (ex :test/failed {:value 41})) (catch error (+ (:value (ex-data error)) 1)))"),
        "42"
    );
    // Hara catch binds one error name and evaluates one handler form.
    assert_eq!(
        eval("(try (throw (ex :test/failed {})) (catch error (:ex/code (ex-data error))))"),
        ":test/failed"
    );
    // First matching catch wins; later clauses do not run.
    assert_eq!(
        eval("(try (throw (ex :test/failed {:value 41})) (catch a 41) (catch b 42))"),
        "41"
    );
    // A body value passes through a catch-bearing try unchanged.
    assert_eq!(eval("(try 7 (catch e 0))"), "7");
}

#[test]
fn catch_binds_runtime_error_messages() {
    // Runtime errors bind the message string.
    assert_eq!(
        eval("(try (/ 1 0) (catch error error))"),
        "\"division by zero\""
    );
    // Errors crossing a closure call bind the bare message string, not a
    // rendered composite.
    assert_eq!(
        eval("(try ((fn [] (/ 1 0))) (catch e e))"),
        "\"division by zero\""
    );
}

#[test]
fn uncaught_throws_propagate() {
    assert_eval_error("(throw (ex :test/failed {}))", "thrown:");
}

#[test]
fn finally_semantics() {
    // Finally results are discarded on the success path.
    assert_eq!(eval("(try 42 (finally 0))"), "42");
    assert_eq!(eval("(try 42 43 (finally 0 1))"), "43");
    // Finally runs after a caught error without changing the outcome.
    assert_eq!(
        eval("(try (throw (ex :test/failed {:value 41})) (catch error (+ (:value (ex-data error)) 1)) (finally 0))"),
        "42"
    );
    // An in-flight error rethrows with its identity after finally.
    assert_eq!(
        eval("(try (try (throw (ex :test/original {})) (finally 0)) (catch e (:ex/code (ex-data e))))"),
        ":test/original"
    );
    // An error in finally replaces the in-flight outcome (first error
    // short-circuits, matching the fiber).
    assert_eval_error("(try 1 (finally (throw (ex :test/finally {}))))", "thrown:");
    assert_eval_error(
        "(try (throw (ex :test/body {})) (catch e (throw (ex :test/catch {}))))",
        "thrown:",
    );
    assert_eval_error(
        "(try (throw (ex :test/body {})) (finally (throw (ex :test/finally {}))))",
        "thrown:",
    );
}

#[test]
fn exceptions_cross_function_boundaries() {
    // try inside a function body.
    assert_eq!(
        eval("((fn [] (try (throw (ex :test/failed {})) (catch e 42))))"),
        "42"
    );
    // A throw inside a called function unwinds to the caller's catch.
    assert_eq!(
        eval("(try ((fn [] (throw (ex :test/failed {:value 41})))) (catch e (+ (:value (ex-data e)) 1)))"),
        "42"
    );
}

#[test]
fn recur_through_catch_only_try() {
    // recur in the body of a catch-only try stays in tail position.
    assert_eq!(
        eval("(loop [i 0] (try (if (< i 3) (recur (+ i 1)) i) (catch e -1)))"),
        "3"
    );
    // recur in a catch body of a catch-only try.
    assert_eq!(
        eval("(loop [i 0] (try (throw (ex :test/failed {})) (catch e (if (< i 3) (recur (+ i 1)) i))))"),
        "3"
    );
}

#[test]
fn try_compile_errors() {
    // Body forms cannot follow catch/finally clauses.
    let (kind, message) = compile_error("(try 1 (catch e 2) 3)");
    assert_eq!(kind, CompileErrorKind::Arity);
    assert!(
        message.contains("try clauses must follow body"),
        "{message}"
    );
    // Malformed catch clauses are compile errors.
    let (_, message) = compile_error("(try 1 (catch 42 0))");
    assert!(message.contains("catch binding must be symbol"), "{message}");
    let (_, message) = compile_error("(try 1 (catch))");
    assert!(
        message.contains("catch expects a binding symbol and one handler form"),
        "{message}"
    );
    // Hara rejects Clojure's optional class slot rather than guessing
    // whether the first symbol is a class or the binding.
    let (_, message) = compile_error("(try 1 (catch Throwable error 0))");
    assert!(
        message.contains("catch expects a binding symbol and one handler form"),
        "{message}"
    );
    // throw takes exactly one value.
    let (kind, message) = compile_error("(throw)");
    assert_eq!(kind, CompileErrorKind::Arity);
    assert!(message.contains("throw expects one value"), "{message}");
    // recur cannot cross a finally boundary (checked before the tail
    // check, because the try itself suppresses tail propagation).
    let (kind, message) =
        compile_error("(loop [i 0] (try (if (< i 3) (recur (+ i 1)) i) (finally 0)))");
    assert_eq!(kind, CompileErrorKind::Recur);
    assert!(
        message.contains("recur cannot cross a finally boundary"),
        "{message}"
    );
}

#[test]
fn uncaught_throw_carries_position() {
    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with(
        "(try 1 (finally 0)) (throw (std.native.Exception/new \"failed\" {}))",
        &registry,
    )
    .expect("compiles");
    let error = execute_program_with_globals(std::rc::Rc::new(program), &registry)
        .expect_err("uncaught throw");
    let text = error.to_string();
    assert!(text.contains("[line 1, column 21]"), "{text}");
    assert!(text.contains("(instruction"), "{text}");
}

#[test]
fn global_forms_issue_223() {
    let mut runtime = Runtime::new();
    assert_eq!(runtime.eval_bytecode_native("(ns+)"), Ok("nil".into()));
    assert_eq!(eval("(def player 1)"), "#'user/player");
    assert_eq!(eval("(= (def player 1) #'player)"), "true");
    assert_eq!(eval("(do (def answer 42) answer)"), "42");
    assert_eq!(
        eval("(do (def answer 19) (def answer (+ answer 23)) answer)"),
        "42"
    );
    // defn interns a real var and evaluates to it; display is qualified.
    assert_eq!(eval("(defn f [x] x)"), "#'user/f");
    assert_eq!(eval("(do (defn f [x] (+ x 1)) (f 41))"), "42");
    // Late binding: redefinition resets the shared cell.
    assert_eq!(
        eval("(do (defn f [x] 1) (defn g [] (f 0)) (defn f [x] 2) (g))"),
        "2"
    );
    assert_eq!(
        eval("(do (defn f [x] 1) (def v (var f)) (defn f [x] 2) (= v (var f)))"),
        "true"
    );
    // var / #' reads the var itself.
    assert_eq!(eval("(do (defn f [x] x) #'f)"), "#'user/f");
    assert_eq!(eval("(do (defn f [x] x) (var f))"), "#'user/f");
    // set! resets a global root and evaluates to the value.
    assert_eq!(eval("(do (def c 0) (set! c (+ c 42)) c)"), "42");
    assert_eq!(eval("(do (def c 0) (set! c 42))"), "42");
    // declare interns a nil var and evaluates to nil.
    assert_eq!(eval("(declare future)"), "nil");
    assert_eq!(eval("(declare a b)"), "nil");
    assert_eval_error("(defn- p [] 42)", "unbound symbol: defn-");
}

#[test]
fn native_struct_forms_issue_223() {
    assert_eq!(
        eval_native_struct("(std.native.Base/type Point)"),
        ":std.native.StructType"
    );
    assert_eq!(eval_native_struct("(:y (Point 19 23))"), "23");
    assert_eq!(
        eval_native_struct(
            "(let [point (map->Point {:x 1 :extra 9})] \
               [(:x point) (:missing point 7) (:extra point) (std.native.Base/type point)])",
        ),
        "[1 7 nil :user.Point]"
    );
    assert_eq!(
        eval_native_struct(
            "[(let [{:keys [x y missing] :or {missing 7} :as point} (Point 1 2)] \
                [x y missing (std.native.Base/type point)]) \
              ((fn [{:keys [x y]}] [x y]) (Point 3 4))]",
        ),
        "[[1 2 7 :user.Point] [3 4]]"
    );
    assert_eq!(
        eval_native_struct(
            "[(std.protocol.ilookup.ILookup/lookup (map->Point {:x 1 :extra 9}) :x) \
              (std.protocol.ilookup.ILookup/lookup (map->Point {:x 1 :extra 9}) :y)]",
        ),
        "[1 nil]"
    );
    assert_eq!(
        eval_native_struct(
            "(let [original (Point 1 2) \
                   updated (std.protocol.iassoc.IAssoc/assoc original :x 10)] \
               [(:x original) (:x updated) (std.native.Base/instance? Point updated)])",
        ),
        "[1 10 true]"
    );
    assert_eq!(
        eval_native_struct("(std.native.Base/instance? Point (Point 1 2))"),
        "true"
    );
    assert_eq!(
        eval_native_struct("(std.native.Base/instance? Point 42)"),
        "false"
    );
    assert_eq!(eval_native_struct("(:x (Point 1 2))"), "1");
}

#[test]
fn native_mutable_forms_use_reference_identity_and_settable_fields() {
    assert_eq!(
        eval_native_mutable("(std.native.Base/type Cursor)"),
        ":std.native.MutableType"
    );
    assert_eq!(
        eval_native_mutable("(std.native.Base/field (Cursor 19 23) :y)"),
        "23"
    );
    assert_eq!(
        eval_native_mutable(
            "(let [cursor (map->Cursor {:x 1 :extra 9})] \
               [(std.protocol.ilookup.ILookup/lookup cursor :x) \
                (:y cursor) \
                (std.protocol.icount.ICount/count cursor)])",
        ),
        "[1 nil 2]"
    );
    assert_eq!(
        eval_native_mutable(
            "(let [cursor (Cursor 1 0) alias cursor \
                   result (set! (field cursor :x) 10)] \
               [result (std.native.Base/field alias :x) (= cursor alias) (= cursor (Cursor 10 0))])",
        ),
        "[10 10 true false]"
    );
    assert_eq!(
        eval_native_mutable(
            "(do (def order []) \
                 (def cursor (Cursor 1 0)) \
                 (set! (field (do (set! order (std.protocol.iconj.IConj/conj order :receiver)) cursor) :x) \
                       (do (set! order (std.protocol.iconj.IConj/conj order :replacement)) 10)) \
                 [order (std.native.Base/field cursor :x)])",
        ),
        "[[:receiver :replacement] 10]"
    );
    assert_eq!(
        eval_native_mutable("(std.native.Base/instance? Cursor (Cursor 1 0))"),
        "true"
    );
}

#[test]
fn mutable_field_set_inside_function_preserves_order_identity_and_result() {
    assert_eq!(
        eval_native_mutable(
            "(do (def order []) \
                 (defn replace! [cursor] \
                   (set! (field (do (set! order (std.protocol.iconj.IConj/conj order :receiver)) cursor) :x) \
                         (do (set! order (std.protocol.iconj.IConj/conj order :replacement)) 10))) \
                 (def cursor (Cursor 1 0)) \
                 [(replace! cursor) order (std.native.Base/field cursor :x)])",
        ),
        "[10 [:receiver :replacement] 10]"
    );
    assert_eq!(
        eval_native_mutable("((fn [cursor] (field cursor x)) (Cursor 7 0))"),
        "7"
    );
}

#[test]
fn variadic_and_multi_arity_issue_223() {
    assert_eq!(eval("((fn [left & more] left) 42 1 2)"), "42");
    assert_eq!(eval("((fn [left & more] more) 42 1 2)"), "(1 2)");
    assert_eq!(eval("((fn [left & more] more) 42)"), "()");
    assert_eq!(
        eval("((fn ([value] value) ([left right] (+ left right))) 19)"),
        "19"
    );
    assert_eq!(
        eval("((fn ([value] value) ([left right] (+ left right))) 19 23)"),
        "42"
    );
    assert_eval_error(
        "((fn [l r & more] l) 1)",
        "function expects at least 2 arguments",
    );
    assert_eq!(
        eval("(do (defn choose ([v] v) ([l r] (+ l r))) (+ (choose 19) (choose 20 3)))"),
        "42"
    );
    assert_eq!(
        eval("(do (defn sum3 ([a b] (+ a b)) ([a b c & more] (+ a b c))) (sum3 19 20 3))"),
        "42"
    );
    assert_eq!(
        eval("(do (defn rest-args [f & r] r) (rest-args 42 1 2))"),
        "(1 2)"
    );
}

#[test]
fn def_is_predeclared_for_later_top_level_definitions() {
    let registry = crate::embedding_namespace_registry();
    compile_source_with(
        "(def bytecode-registry (std.native.Base/atom {}))\n(defn bytecode-value [] bytecode-registry)\n(= bytecode-registry (bytecode-value))",
        &registry,
    )
    .expect("def is visible to later definitions during compilation");
}

#[test]
fn global_form_errors_issue_223() {
    let (kind, message) = compile_error("(set! missing 1)");
    assert_eq!(kind, CompileErrorKind::UnboundSymbol);
    assert!(message.contains("unbound var: missing"), "{message}");
    let (kind, message) = compile_error("(var missing)");
    assert_eq!(kind, CompileErrorKind::UnboundSymbol);
    assert!(message.contains("unbound var: missing"), "{message}");
    let (_, message) = compile_error("(let [x 1] (set! x 2))");
    assert!(message.contains("set! targets a global var"), "{message}");
    // Mutable-field errors surface at runtime, not compile time.
    assert_eval_error(
        &native_mutable_source("(std.native.Base/field (Cursor 1 0) :z)"),
        "unknown mutable field: z",
    );
    assert_eval_error(
        &native_struct_source("(std.native.Base/field (Point 1 0) :x)"),
        "field expects a mutable value",
    );
    assert_eval_error(
        &native_mutable_source("(std.native.Base/field 42 :x)"),
        "field expects a mutable value",
    );
    assert_eval_error(
        &native_mutable_source("(set! (field (Cursor 1 0) :z) 2)"),
        "unknown mutable field: z",
    );
    assert_eval_error(
        &native_mutable_source(
            "(std.protocol.iassoc.IAssoc/assoc (Cursor 1 0) :x 2)",
        ),
        "protocol/unsupported-receiver: missing protocol implementation: std.protocol.iassoc.IAssoc/assoc",
    );
    assert_eval_error(
        &native_mutable_source(
            "(std.protocol.idissoc.IDissoc/dissoc (Cursor 1 0) :x)",
        ),
        "protocol/unsupported-receiver: missing protocol implementation: std.protocol.idissoc.IDissoc/dissoc",
    );
    assert_eval_error(
        "(std.native.Base/instance? 42 1)",
        "Base/instance? expects a type descriptor and value",
    );
    // The standalone host has no referred Foundation Vars, so a local
    // definition of identity compiles and executes normally.
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime.eval_bytecode_native("(do (defn identity [n] 42) (identity 5))"),
        Ok("42".into())
    );
    // Uninitialized let-style errors keep their shape.
    let (_, message) = compile_error("(fn [a &] a)");
    assert!(
        message.contains("rest parameter must be the last"),
        "{message}"
    );
}

#[test]
fn async_metadata_and_await_lowering_are_explicit() {
    let program = compile_source("(defn ^:async delayed [p] (std.native.Coroutine/await p))")
        .expect("async function must compile");
    let async_proto = program
        .functions
        .iter()
        .find(|function| function.name.as_deref() == Some("delayed"))
        .expect("named async prototype");
    assert!(async_proto.async_function);
    assert!(async_proto.code.contains(&super::Instruction::Await));
}

#[test]
fn await_infers_a_suspending_synchronous_function() {
    let program = compile_source("(defn delayed [p] (std.native.Coroutine/await p))")
        .expect("await should infer suspension support");
    let prototype = program
        .functions
        .iter()
        .find(|function| function.name.as_deref() == Some("delayed"))
        .expect("named prototype");
    assert!(
        !prototype.async_function,
        "inferred await must not force a promise wrapper"
    );
    assert!(prototype.code.contains(&super::Instruction::Await));

    compile_source("(defn outer [p] (fn [] (std.native.Coroutine/await p)))")
        .expect("nested functions infer their own suspension support");
}

#[test]
fn inferred_await_returns_directly_until_it_really_suspends() {
    let registry = NamespaceRegistry::new("user");
    let source = Promise::new();
    registry
        .find_or_create("user")
        .intern("source", Value::Promise(source.clone()));
    let program = compile_source_with(
        "(do (defn delayed [] (std.native.Coroutine/await source)) (delayed))",
        &registry,
    )
    .unwrap();
    let mut fiber =
        crate::core::with_namespace_registry(&registry, || super::VmFiber::start(Rc::new(program)));
    assert!(matches!(fiber.state(), super::VmFiberState::Suspended));
    source.resolve(Value::Number(9));
    assert!(matches!(
        fiber.poll(),
        super::VmFiberState::Completed(Value::Number(9))
    ));

    let registry = NamespaceRegistry::new("user");
    let ready = Promise::new();
    ready.resolve(Value::Number(42));
    registry
        .find_or_create("user")
        .intern("source", Value::Promise(ready));
    let program = compile_source_with(
        "(do (defn immediate [] (std.native.Coroutine/await source)) (immediate))",
        &registry,
    )
    .unwrap();
    assert_eq!(
        execute_program_with_globals(Rc::new(program), &registry).unwrap(),
        Value::Number(42)
    );
}

#[test]
fn async_calls_return_promises_and_adopt_direct_values() {
    let program = compile_source("(do (defn ^:async answer [] 42) (answer))").unwrap();
    let Value::Promise(result) = execute_program(Rc::new(program)).unwrap() else {
        panic!("async call must return a promise")
    };
    assert_eq!(result.state(), PromiseState::Fulfilled(Value::Number(42)));
}

#[test]
fn pending_async_child_is_resumed_only_when_the_scheduler_is_polled() {
    let registry = NamespaceRegistry::new("user");
    let source = Promise::new();
    registry
        .find_or_create("user")
        .intern("source", Value::Promise(source.clone()));
    let program = compile_source_with(
        "(do (defn ^:async delayed [] (std.native.Coroutine/await source)) (delayed))",
        &registry,
    )
    .unwrap();
    let Value::Promise(result) = execute_program_with_globals(Rc::new(program), &registry).unwrap()
    else {
        panic!("async call must return a promise")
    };
    assert_eq!(result.state(), PromiseState::Pending);
    source.resolve(Value::Number(9));
    assert_eq!(result.state(), PromiseState::Fulfilled(Value::Number(9)));
}

#[test]
fn cancelling_async_result_propagates_to_the_pending_host_promise() {
    let registry = NamespaceRegistry::new("user");
    let source = Promise::new();
    let cancelled = Rc::new(Cell::new(false));
    let observed = cancelled.clone();
    source.set_cancel_hook(Rc::new(move || observed.set(true)));
    registry
        .find_or_create("user")
        .intern("source", Value::Promise(source));
    let program = compile_source_with(
        "(do (defn ^:async delayed [] (std.native.Coroutine/await source)) (delayed))",
        &registry,
    )
    .unwrap();
    let Value::Promise(result) = execute_program_with_globals(Rc::new(program), &registry).unwrap()
    else {
        panic!("async call must return a promise")
    };
    assert!(result.cancel());
    assert!(cancelled.get());
    assert!(matches!(
        result.state(),
        PromiseState::Rejected(error) if error.is_cancelled()
    ));
}

#[test]
fn async_calls_always_return_settled_promises_on_the_fast_path() {
    let value = eval_source("(do (defn ^:async answer [] 42) (answer))")
        .expect("async call must return normally");
    let Value::Promise(promise) = value else {
        panic!("async call returned {value:?}");
    };
    assert_eq!(
        promise.state(),
        crate::core::PromiseState::Fulfilled(Value::Number(42))
    );

    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with(
        "(do (defn ^:async fail [] (throw (std.native.Exception/new \"boom\" {}))) (fail))",
        &registry,
    )
    .expect("async failure source compiles");
    let value = execute_program_with_globals(Rc::new(program), &registry)
        .expect("async throw rejects rather than escaping");
    let Value::Promise(promise) = value else {
        panic!("async call returned {value:?}");
    };
    assert!(matches!(
        promise.state(),
        crate::core::PromiseState::Rejected(ref error) if error.message().contains("thrown:")
    ));
}

#[test]
fn async_calls_retain_and_resume_pending_child_fibers() {
    let registry = crate::kernel::NamespaceRegistry::new("user");
    let pending = crate::core::Promise::new();
    registry
        .current()
        .intern("pending", Value::Promise(pending.clone()));
    let program = super::compile_source_with(
        "(do (defn ^:async delayed [] (std.native.Coroutine/await pending)) (delayed))",
        &registry,
    )
    .expect("async source must compile");
    let value = super::execute_program_with_globals(std::rc::Rc::new(program), &registry)
        .expect("async call returns its result promise");
    let Value::Promise(result) = value else {
        panic!("async call returned {value:?}");
    };
    assert_eq!(result.state(), crate::core::PromiseState::Pending);
    pending.resolve(Value::Number(42));
    assert_eq!(
        result.state(),
        crate::core::PromiseState::Fulfilled(Value::Number(42))
    );
}

#[test]
fn vm_host_call_returns_a_native_promise_and_resumes_through_await() {
    let pending = Promise::new();
    let provider_promise = pending.clone();
    let observed = Rc::new(RefCell::new(Vec::new()));
    let provider_observed = observed.clone();
    let provider = Rc::new(
        move |service: String, method: String, arguments: Vec<Value>| {
            provider_observed
                .borrow_mut()
                .push((service, method, arguments));
            Ok(Value::Promise(provider_promise.clone()))
        },
    );
    let program = compile_source(
        "(do (defn ^:async delayed [] (std.native.Coroutine/await (std.native.Host/call \"nginx\" \"sleep\" [25]))) (delayed))",
    )
    .unwrap();
    let value = crate::core::with_host_calls(provider, || execute_program(Rc::new(program)))
        .expect("host call returns its promise");
    let Value::Promise(result) = value else {
        panic!("async host call returned {value:?}");
    };
    assert_eq!(result.state(), PromiseState::Pending);
    assert_eq!(
        observed.borrow().as_slice(),
        &[("nginx".into(), "sleep".into(), vec![Value::Number(25)])]
    );
    pending.resolve(Value::String("done".into()));
    assert_eq!(
        result.state(),
        PromiseState::Fulfilled(Value::String("done".into()))
    );
}

#[cfg(feature = "tracing-jit")]
#[test]
fn typed_numeric_functions_start_guarded_tracing_on_the_first_backedge() {
    use crate::kernel::{FunctionSchema, SchemaType};

    let mut program = compile_source(
        "(do (defn sum-to [n] \
           (loop [i 0 total 0] \
             (if (< i n) (recur (+ i 1) (+ total i)) total))) \
         (sum-to 10))",
    )
    .unwrap();
    program.namespace = Some("user".into());
    program.function_types.insert(
        "user/sum-to".into(),
        SchemaType::Function(vec![FunctionSchema {
            fixed: vec![SchemaType::Primitive("int".into())],
            rest: None,
            output: Box::new(SchemaType::Primitive("int".into())),
        }]),
    );
    let program = Rc::new(program);

    assert_eq!(execute_program(program.clone()).unwrap(), Value::Number(45));
    assert!(super::machine::cached_trace_count(&program) > 0);
    let telemetry = super::machine::cached_jit_telemetry(&program);
    assert_eq!(telemetry.recording_starts, 1);
    assert!(
        telemetry.backedges < 16,
        "typed trace waited for the generic threshold"
    );
}
