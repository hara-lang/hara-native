# Macro invocation metadata

Macro `&form` must retain the metadata attached to the invocation. This is
required by Foundation's `%.form` and `%.str` macros, whose options come from
`(meta &form)`, including raw preprocessing mode.

Rust macro lookup unwraps metadata only to inspect the operator and arguments.
The macro receives the complete invocation as `&form`; explicit
`Runtime/macroexpand-1`, interpreted evaluation, and direct-native compilation
use this same boundary. Ordinary metadata-free calls remain metadata-free.
Exception source-site handling remains around interpreted expansion/evaluation.

The regression failed before the change (`nil` instead of `{:- :raw}`) and
passes for interpreter and direct-native backends afterward. It covers direct
calls, nested calls, function bodies, explicit expansion, and no metadata leak
into a later ordinary invocation. JVM parity checks the option value rather
than discarding the JVM's additional source-location metadata; no JVM runtime
change is required.

Validation:

- `cargo test --test native-lang`: 21 passed with loopback permission. The
  sandbox-only run passed 20 and denied the existing socket test.
- `mvn -q -Djacoco.skip=true -Dpolyglot.engine.WarnInterpreterOnly=false -Dtest=HaraNativeLangTest test`:
  10 passed. Coverage instrumentation was disabled because the installed
  JaCoCo version cannot instrument this JDK's class-file version.
- `cargo build --release --bin hara-native`: rebuilt the migration consumer.
- `cargo test --lib macro`: 12 passed, including compiler, bytecode macro,
  bundle, and re-exported macro checks.

Historical migration verification is recorded under Foundation's
`resources/code/migrate/candidates/`; the original raw-mode assertion is not
replaced by a direct call to its implementation helper.
