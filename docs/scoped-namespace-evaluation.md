# Captured namespace evaluation

`Runtime/eval-in` (published by Foundation as `eval-in-ns`) accepts either its
existing collection of quoted forms or a zero-argument native function.

The function form keeps lexical bindings and captured runtime values intact.
During the synchronous call, `Runtime/current` and dynamic `Runtime/eval`
observe the requested existing namespace. Nested scopes override that namespace
until they return. Success, invalid arity, and thrown errors restore the caller's
scope. The operation does not create namespaces or redefine captured globals.
Asynchronous propagation after the function returns is not promised.

The quoted-form form retains its existing lexical isolation. Rust now also
restores the selected namespace when interpretation of a quoted form fails.

Rust reuses the dynamic evaluation namespace guard rather than changing a
function's captured environment or declaring namespace. JVM dynamic evaluation
uses uncached parses: otherwise identical `eval` text could reuse bindings
resolved in an earlier namespace.

## Validation

- `cargo test --test native-lang`: 17 passed (loopback socket test requires
  execution outside the filesystem/network sandbox).
- Both interpreter and direct-native execution run the captured-scope test.
- `mvn -q -Djacoco.skip=true -Dtest=HaraNativeLangTest test`: 7 passed.
- Final combined `HaraNativeLangTest,HaraDelayTest` run: 11 passed, including
  the quoted-form lexical-isolation regression.
- Both hosts' new scope tests reproduced the old function-rejection error.
- Nested JVM evaluation additionally exposed stale parse bindings; its exact
  expected target value passes after disabling dynamic evaluation parse caching.

The release binary was rebuilt as 0.1.27. The adjacent Hara source project still
pins host 0.1.26; its manifest was not changed. Emitter candidate validation uses
an isolated source copy and a matching 0.1.27 manifest, not the canonical project
launcher. Browser/Wasm execution has not been verified for this change.
