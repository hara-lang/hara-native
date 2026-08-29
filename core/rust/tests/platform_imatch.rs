use hara_wasm::Runtime;

#[test]
fn imatch_is_installed_before_any_hal_resource_is_loaded() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "(defstruct PlatformMatcher [expected]) \
                 (extend-type PlatformMatcher IMatch \
                   (match-value [matcher actual] \
                     (= (:expected matcher) actual))) \
                 [(boolean IMatch) \
                  (IMatch/match-value (PlatformMatcher 42) 42)]"
            )
            .unwrap(),
        "[true true]"
    );
}
