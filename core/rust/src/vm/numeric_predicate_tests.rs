use crate::Runtime;

#[test]
fn bytecode_uses_representation_independent_numeric_predicates() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime.eval_bytecode_native(
            "[(std.native.Base/long? 42) \
              (std.native.Base/long? (std.native.Num/double 1.5)) \
              (= (std.native.Base/type (std.native.Num/double 1.5)) :std.native.Float) \
              (= (std.native.Base/type 42) :std.native.Float) \
              (std.native.Base/number? 42) (std.native.Base/number? 1.5) \
              (std.native.Base/long? 9223372036854775808) \
              (= (std.native.Base/type 9223372036854775808) :std.native.BigInteger) \
              (= (std.native.Base/type 42) :std.native.BigInteger) \
              (= (std.native.Base/type 42) :std.native.Long) \
              (= (std.native.Base/type 9223372036854775808) :std.native.BigInteger) \
              (= (std.native.Base/type 1.0) :std.native.Long)]",
        ),
        Ok("[true false true false true true false true false true true false]".into()),
    );
}
