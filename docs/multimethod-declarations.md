# Nested native multimethod declarations

The faithful relational-schema migration exposed a native declaration failure:
`defmethod` could not find a multimethod created through a declaration callback.
Both release and debug project runs reproduced it, including the existing
`test/std/foundation_test.hal` file.

Nested direct-native contexts sharing one multimethod registry were exchanging
the live active map with a saved snapshot. Restoring the parent snapshot lost
registrations made by nested callbacks. Such contexts now retain the active map;
contexts with distinct registries still use the existing swap/restore boundary.

The native-lang regression covers interpreter and direct-native execution,
nested declaration callbacks, callbacks invoked in later evaluations, exact
dispatch/default results, and rollback of an aborted declaration. Each backend
runs in its own thread so the thread-local registry is released even on failure.
The regression failed before the implementation change and passes afterward.

Validation:

- `cargo test --offline --test native-lang`: 45 passed with permission for the
  suite's existing loopback socket fixtures.
- `cargo build --offline --release --bin hara-native`: passed.
- Rebuilt native runtime evaluates the real schema-base multimethod and all
  eight historical schema-base facts. Additional schema contracts pass too.
- The full Foundation test file advances past multimethod registration but
  still fails with `unbound symbol: def`. That later failure remains unresolved;
  this change does not claim the whole Foundation file passes.

No migration-only callable wrapper or replacement dispatch table was introduced.
No commits or publication were performed.
