# Value interning

`Runtime/intern namespace symbol value` binds a live value and returns the
destination Var. The prelude exposes the same three-argument operation as
`std.foundation/intern`. The target symbol must be unqualified.

Unlike `intern-var`, this operation does not dereference its input or copy
source-Var metadata. A supplied Var remains the stored value. Native pointers,
closures, and nested Var callbacks are retained without serialization.
Metadata on the name symbol replaces the destination Var metadata, as required
by the original script-definition path (including argument lists). Rebinding
with an unannotated name preserves existing metadata. Rebinding an owned Var
preserves its identity. A referred source
Var is shadowed locally, never reset through its referral. Previously compiled
references retain their resolved binding; compile subsequent consumers after
dynamic publication.

Stateful callers should save the previous value and restore it in `finally`.
The focused tests restore predeclared destination slots, including nil. Host
test runtimes own and discard their newly created namespaces.

Validation (2026-09-10):

- Rust `cargo test --test native-lang`: 22 passing, including interpreter and
  direct-native interning assertions.
- JVM `mvn -q -Djacoco.skip=true -Dtest=HaraNativeLangTest test`: 11 passing.
- Complete candidate and written Foundation source evaluated in fresh native
  processes; five new path-matched Foundation checks passed independently.
- Before implementation, the Rust regression failed with unbound
  `Runtime/intern`. A deliberately incorrect Hara wrapper using `intern-var`
  produced four errors; restoring value interning passed all five checks.
- The complete Foundation test file fails during existing multimethod fixture
  initialization: `Base/method expects an existing multimethod`. This was
  reproduced before the Foundation edits and remains outside this change.
- The release native executable was rebuilt. No Pointer type, component hook,
  runtime lifecycle hook, or migration handler was changed by this addition.

Follow-up script-pipeline regression: the original script macro passes a
metadata-bearing symbol into `ptr-intern`. The first implementation of value
interning dropped this metadata. A new host assertion reproduced missing doc,
arglists, and language fields; a new Foundation assertion failed on the old
release with five existing checks still passing. Rust and JVM now retain this
metadata, including replacement and unannotated-name rebinding semantics.
The six focused Foundation checks and the 22 Rust / 11 JVM suites pass. Metadata
tests restore both the root value and the original metadata in `finally`.
