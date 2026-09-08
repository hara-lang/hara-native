# Library migration: immediate calls in captured closures

The faithful Foundation Library candidate exposed a bytecode validation error:
`callstatic capture count differs from current function`. The source-only
candidate failed before its tests ran. This was reproduced independently by:

```clojure
(let [make (fn [captured] (fn [] ((fn [& args] 1) captured)))]
  ((make 7)))
```

The immediate-function optimization selected `CallStatic` for a capture-free
callee inside a caller with captures. That instruction inherits the caller's
capture environment; the bytecode validator correctly rejected the mismatch.

The optimization now requires the caller to be capture-free as well. Function
captures are inventoried before body compilation, so this includes captures
used in arguments and expressions after the call. Other calls retain their
normal closure boundary. The validator and self-recursive direct calls are
unchanged. No Library source adaptation is used to hide the compiler error.

Validation:

- The new `immediate_variadic_calls_do_not_inherit_caller_captures` regression
  failed against the previous compiler with the exact validation error.
- After the fix, `cargo test --offline --lib vm::` passed all 169 tests,
  including capture isolation, direct recursion, and validator rejection tests.
- `cargo build --offline --release --bin hara-native` succeeded.

The original Library candidate now compiles with the rebuilt release. Its seven
state tests report six passing and one error: native `IComponent/start` has no
implementation for Library. Foundation's component implementation supplies an
Object-wide identity default, whereas the native component layer requires an
explicit implementation. This is the next fidelity boundary to reconcile; the
component call has not been bypassed. An invalid unqualified exception code in
the scratch failure test was corrected to `:migration/test` before this result.

This host checkpoint does not establish complete Library migration parity or
JVM/browser execution parity. Library source and tests remain candidates in the
migration REPL, not installed native files.
