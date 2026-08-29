use hara_wasm::Runtime;

#[test]
fn to_fixed_supports_every_runtime_numeric_representation() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "[(str/to-fixed 1 2) \
                  (str/to-fixed 123.45 3) \
                  (str/to-fixed (double 1.24) 1) \
                  (str/to-fixed 9223372036854775808 2)]"
            )
            .unwrap(),
        "[\"1.00\" \"123.450\" \"1.2\" \"9223372036854775808.00\"]"
    );
}

#[test]
fn to_fixed_rejects_precision_outside_the_portable_bounds() {
    for source in ["(str/to-fixed 1 -1)", "(str/to-fixed 1 101)"] {
        let mut runtime = Runtime::new();
        let error = runtime.eval_native(source).unwrap_err();
        assert!(
            error.contains("str/to-fixed precision must be in the range 0..100"),
            "{source}: {error}"
        );
    }
}
