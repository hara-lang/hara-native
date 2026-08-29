use hara_wasm::Runtime;

#[test]
fn istringlike_is_installed_before_any_hal_resource_is_loaded() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "(require [std.protocol.istringlike :as sl]) \
                 (defstruct WrappedName [value]) \
                 (extend-type WrappedName sl/IStringLike \
                   (to-string [wrapped] (:value wrapped)) \
                   (from-string [wrapped text] (WrappedName text))) \
                 [(satisfies? sl/IStringLike :hello) \
                  (sl/to-string :hello/world) \
                  (sl/from-string :sample \"hello/world\") \
                  (sl/to-string (WrappedName \"custom\")) \
                  (:value (sl/from-string (WrappedName \"\") \"restored\"))]"
            )
            .unwrap(),
        "[true \"hello/world\" :hello/world \"custom\" \"restored\"]"
    );
}
