# Native Var and Pointer invocation

Vars participate in Rust native IFn dispatch. Invocation follows the current
root or dynamic binding at call time, after argument evaluation. Var chains
are resolved by identity; cycles fail with `cyclic Var invocation` rather than
overflowing. A Var with a non-callable value still implements IFn, but invoking
it reports the underlying non-callable error.

Pointers already supported IFn and retain their context-runtime dispatch.
Vars containing pointers use that same path. Ordinary calls, IFn/invoke, and
Base/apply are covered on interpreter and direct-native backends. No migration
wrapper or new Hara API is needed. The JVM invokeCallable dispatcher already
follows Var roots and handles pointers; no JVM change was made in this slice.

The initial Var regression failed with `value is not callable` before the
runtime edit. Final validation:

- `cargo test --offline --test native-lang`: 44 passed. Run with loopback
  permission; the sandbox alone rejects the existing socket fixtures.
- `cargo build --offline --release --bin hara-native`: passed.
- Rebuilt CLI, original staged compiler: two historical specialization facts
  and five branch/order/failure/batch contracts pass without dereferencing the
  selected compiler Var in migration code.
- Scoped `git diff --check`: passed.

Interpreter coverage also verifies yielding through a Var retains its caller
continuation. Direct-native nested yield is not claimed as verified: a probe
whose coroutine calls a plain `yield-once` function reports `cannot resume a
dead coroutine` on its second resume. The same plain-function probe fails with
explicit async metadata. This is not isolated to Vars and needs a separate
direct-native coroutine investigation. No direct-native coroutine expectation
was weakened or installed as a passing contract.

The runtime edit preserves the separate direct-native/fiber call paths after
resolving the Var, rather than routing them through synchronous invocation.
No commits or publication were performed.
