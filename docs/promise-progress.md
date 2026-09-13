# Cooperative Promise progress

The Rust host progresses subscribed promises while waiting on another promise.
This allows a process-completion continuation to run while the caller awaits
an unrelated timer, including the original `lang.runtime.basic.type-bench`
completion callback that removes an exited process from its active registry.

## Boundaries

- `wait_state` and `wait_state_timeout` poll subscribed promises at cooperative
  wait points. Ordinary `state` inspection does not globally drain work.
- The thread-local subscription registry holds weak references. It prunes
  dropped and settled promises, and snapshots before running callbacks so
  callbacks can subscribe or settle promises without a registry borrow conflict.
- Native process completion and stream promises explicitly opt into cooperative
  polling. Ordinary provider and async-wrapper waiters remain authoritative;
  their pollers alone may be unable to drive upstream host work.
- Process polling does not join unfinished output-reader threads. Completion
  retains its existing requirement that captured output has reached EOF.
- This is not a background event loop. Pure CPU work and arbitrary blocking host
  operations do not gain asynchronous scheduling. Recursive global progress is
  guarded; synchronously waiting inside a continuation for another continuation
  in the same drain is not a supported scheduling mechanism.
- No Hara public API, process-tree termination behavior, or JVM implementation
  changes are introduced.

## Regression coverage

`src/task/promise.rs` covers unrelated waits, timed waits remaining pending,
cooperative provider fairness, preservation of essential waiters, cancellation,
dropped subscriptions, and reentrant subscription registration. Existing async
VM tests cover successive waiter-only upstream promises.

`src/native_process.rs` holds an output reader behind a channel to prove that
polling an exited process returns pending without joining the reader, then
releases it and verifies the exact exit code and captured bytes.

`tests/native_lang.rs` verifies a process exit continuation while awaiting an
unrelated timer in both interpreter and direct-native modes.

Run from this repository:

```text
cargo test --manifest-path core/rust/Cargo.toml --lib -- --test-threads=1
cargo test --manifest-path core/rust/Cargo.toml --test native-lang -- --test-threads=1
cargo build --release --manifest-path core/rust/Cargo.toml --bin hara-native
```

Native socket tests require local loopback access.
