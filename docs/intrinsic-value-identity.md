# Intrinsic callable identity

The faithful Foundation `script-macro/call-thunk` implementation compares its
`:tag` against the function values `*` and `-`. These are identity comparisons,
not comparisons against the symbols `'*` and `'-`.

The interpreter already reused the Foundation Var's callable. The bytecode VM's
`IntrinsicValue` instruction allocated a new native wrapper on every read, so
even `(= * *)` returned false. This made the original script tag branches
unreachable through the direct-native backend.

`IntrinsicValue` now reads the existing `std.foundation` Var when a namespace
registry supplies one. This preserves pointer identity without changing function
equality, hashing, or allocating another cache. Registry-free VM execution keeps
its previous primitive fallback; stable identity for that fallback is not claimed.
Ordinary separately allocated closures still compare unequal. No API was added.

Validation:

- `cargo test --offline --release --test native-lang intrinsic_values_preserve_foundation_function_identity -- --nocapture`
  failed before the fix on direct-native (`[false false false]` instead of
  `[true true false]`) and passed afterward on both backends.
- The regression checks cross-namespace tags, equality with the actual
  Foundation Var's root, and inequality of distinct ordinary closures.
- `cargo test --offline --release --test native-lang -- --nocapture`: 28 passed.
- `cargo test --offline --release --lib vm:: -- --nocapture`: 169 passed.
- `cargo build --offline --release --bin hara-native`: succeeded.

The change relies on the existing runtime-owned namespace lifecycle; there is no
additional mutable state to reset. JVM/browser parity was not run for this fix.
