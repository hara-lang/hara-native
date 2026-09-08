# Host-local delayed evaluation

`Base/delay` accepts one zero-argument function without invoking it.
`IDeref/deref` and `IRealize/realize` invoke it on first realization and cache
the returned value or language failure. `IRealize/realized?` remains false
until that outcome has been stored, including for nil/false values and errors.
Recursive realization reports `delay realization is recursive` rather than
deadlocking or overflowing the stack. A supplier may catch that failure itself;
otherwise the outer realization caches it.

The JVM uses the existing synchronized Delay primitive. Rust uses a host-local
reference-counted value shared by its evaluator and compiled execution paths.
These are not timed promises and do not start background work. Delays have
identity semantics and are not transferable serialized values. They are
single-assignment: no reset API is added. Releasing the final reference releases
the pending thunk or cached outcome; JVM context/GC lifetime owns that boundary
on the JVM. Rust releases captured thunk references after realization.

## Validation checkpoint

- Rust native language suite: 16 passing tests, including five delay tests.
  The existing loopback socket check required a sandbox-exempt rerun.
- JVM HaraDelayTest: four passing tests, including concurrent realization.
- JVM HaraNativeLangTest: six passing adjacent regression tests.
- A deliberately incorrect Rust cached-outcome expectation failed; restoring
  the correct expectation returned all five focused tests to green.
- Rust library compilation passed.
- Wasm32 library compilation passed with the rustup stable compiler and
  `--no-default-features --features bytecode-vm,browser-wasm`. The Homebrew
  compiler lacked that target; selecting the installed rustup compiler resolved
  the build-environment failure. This is not browser execution evidence.
- The release `hara-native` binary was rebuilt successfully.

## Foundation integration and structured failure correction

The canonical `std.foundation/delay` macro is now installed in the sibling
Hara source repository, with four handwritten checks in its path-matched
`test/std/foundation_test.hal`. It wraps all body forms in a zero-argument
function passed to `Base/delay`; an empty body returns nil. It uses no timers,
promises, or eager sequence adapters.

The source-level structured-error check exposed a host defect not covered by
the original string-throw test: Rust's catch boundary consumes the dynamic
thrown value. Caching only its error string therefore lost the exception data
on the second dereference. `RuntimeDelay` now retains the original thrown value
alongside the outcome and restores that value on every cached failure. The
old release returned false for repeated caught exception equality; the rebuilt
release returns true. The permanent Rust test now throws a structured exception.

All four written macro checks pass twice in fresh native project-test processes.
The complete source/test candidate also evaluates as top-level bootstrap source;
the written Foundation source loads in a fresh core runner. The changed delay
seam has a real corresponding block, no pending/missing obligation, and a stable
second scaffold generation. Existing Foundation placeholder tests are not
claimed as covered. Full-file scaffold diagnostics were superseded by this
changed-seam inventory and their owned processes were cancelled/cleaned up.

The existing full Foundation project-test file fails before the delay checks
with `Base/method expects an existing multimethod`, both before and after this
edit. The adjacent bootstrap suite passes six checks. A core `run` exit status
alone is not a test pass: individual Result values were inspected, and final
delay checks were verified through the counted native test runner. Compiled
macro expansion qualifies Base's symbol, so its test compares resolved
constructor ownership while retaining the exact generated function body.

Recovery of the Foundation files is additive: remove the new delay macro
immediately before defonce and the appended delay Test/check block. All previous
source and test bytes were preserved; installed files matched evaluated
candidates exactly. No pinned tahto source/test or Clojure tooling was edited.

Rust's 16 native-language tests and Wasm compilation pass after the correction.
Browser execution/conformance and the full host regression train remain
separate validation obligations. Typed migration remains deferred behind
lang.base and lang.core at the user's request.
