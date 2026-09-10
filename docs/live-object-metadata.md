# Live object metadata

`IObjType/meta` and `IObjType/with-meta` retain native Struct and Function
values inside metadata maps, including nested collections. Record types,
record fields, function identity and captured environments are preserved.
No Hara API or global handle registry is introduced.

Rust stores these leaves in an owned, process-local `RuntimeMetadata` payload.
Cloning metadata retains the payload; releasing its last owner releases it.
Replacing metadata or clearing it with nil continues to use the existing
`IObjType` behavior. The original receiver is unchanged for persistent values.
Existing portable metadata conversion preserves its original representation.

Live payloads are not portable artifacts: HBC serialization, component-value
export, and conversion to source forms reject them explicitly rather than
silently flattening or omitting them. Documentation displays an opaque marker
for a live payload, without changing the retained metadata. This change does
not add support for other previously unsupported metadata leaves.

The JVM already stores records and callbacks through its existing `IMetadata`
path; no JVM implementation change was necessary.

## Validation

Final local results (2026-09-09): 19 Rust host tests, 6 Rust test-runner tests,
8 JVM host tests, 42 native emitter facts (`emit-assign`, `emit-rewrite`,
`emit`), 3 metadata migration facts, and 3 Snapshot namespace-scope facts pass.
The consuming release executable was rebuilt and the native checks repeated.

- `cargo test --test native-lang --test native-test-registry`: regression
  coverage for records, captured callbacks, list-form assignment metadata,
  metadata replacement/removal, ownership release, and serialization rejection.
  The existing socket test requires loopback access.
- `mvn -q -Djacoco.skip=true -Dtest=HaraNativeLangTest test`: JVM parity.
- Removing the Rust Struct/Function conversion branch makes the new behavioral
  regression fail with `value cannot be stored in runtime-neutral metadata`;
  restoring it makes the same test pass.
- Foundation migration probes exercise parent Snapshot metadata, a nested
  grammar callback, and the original `emit-assign/assign-value` callback result.

This resolves the live-metadata migration boundary, not the entire
`lang.base` or Snapshot/Library migration.
