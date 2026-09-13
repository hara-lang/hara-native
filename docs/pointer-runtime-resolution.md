# Pointer runtime resolution

The faithful language migration is authorized to extend native Pointer runtime
resolution, component `info`/`health`, and clipboard support. This change covers
the Pointer evaluator boundary. Component queries are now implemented in the
follow-up documented in [component-queries.md](component-queries.md). The text
clipboard boundary is documented in [clipboard-text.md](clipboard-text.md);
its language runtime wiring and desktop integration checks remain pending.

Authority: Foundation `fe54cc866473a888bfbd4ce2d3de6d8011f09f72`,
`src/std/lib/context/pointer.clj`, `pointer-default` and `pointer-deref`.

Application and display select the first truthy value from:

1. The current binding of `std.lib.context.pointer/*runtime*`, when that Var exists.
2. The descriptor's `:context/rt` field.
3. Calling `:context/fn` with the original Pointer.
4. `std.lib.context.space/space:rt-current` with the Pointer's context.

Nil and false fall through. Resolver exceptions propagate rather than silently
falling back. Looking up the optional override does not load or create its
namespace. The language migration owns declaration of that dynamic Var.

Dereferencing deliberately resolves through Space, independently of the override
and descriptor resolver. This is the original Foundation distinction, not an
accidental omission. Descriptor equality, hashing, and metadata are unchanged.

The JVM also routes intrinsic `IApplicable`, `IDeref`, and `IDisplay` calls on
Pointers through the evaluator, as direct Pointer application already does.
Without that routing, the public protocol method invoked Pointer's evaluator-free
fallback and threw even when an evaluator was present.

Validation:

- The regression fails on the previous JVM implementation with
  `pointer/runtime-unavailable: resolution requires an evaluator` and on the
  previous Rust implementation by falling through to an unavailable Space.
- The regression passes in JVM, Rust interpreter, and Rust direct-native mode.
  It verifies precedence, short-circuiting, nil/false fallback, truthy zero,
  callback failure, dynamic binding restoration after failure, actual invocation,
  and the independent Space-only dereference path.
- `make test-rust` passes: 34 CLI tests, 31 native-language tests, and 6 test-registry
  tests. Its socket test requires loopback permission; the sandbox-denied run is
  not counted as a passing run.
- Maven `-Dtest=HaraNativeLangTest,PointerTest` passes 15 tests.
- The release Rust binary was rebuilt. The installed macro migration test passes
  all 60 facts against it (`34dce136-845e-4d78-8b75-1980650e4baa`). This still uses
  the original model/runtime overlays and does not claim installed-only closure.
- Full Maven test run in the sandbox: 103 tests, 0 failures, 8 socket permission
  errors in `HaraServerTest`. The permitted full Maven rerun passes. Cross-host
  conformance targets have not yet been run.

The validation above records the Pointer stage; component-query validation is
recorded separately. Clipboard boundary validation is also recorded separately.
