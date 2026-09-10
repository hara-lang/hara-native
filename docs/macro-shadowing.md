# Ordinary definitions shadow prelude macros

Original language script macros define an ordinary `intern-in` helper. Hara's
prelude also has a publication macro named `intern-in`. These are separate
owners; a local function must win over the prelude fallback.

Rust live macro resolution now stops fallback when the current namespace maps
the name to an ordinary Var, including referrals and nonfunction values.
Qualified prelude calls and absent-name fallback remain available.

Direct-native namespace compilation also tracks ordinary `def` and `defn`
names encountered in the current compilation. Later calls must not expand a
prelude macro while waiting for the ordinary definition's emitted instruction
to execute. `defmacro` clears this ordinary-definition shadow for its name.
This change does not claim a general overhaul of lexical macro shadowing or
forward macro declaration semantics.

## Evidence

`local_values_shadow_foundation_macros_without_falling_back` runs both
interpreter and direct-native backends. Its first assertion reproduced
`:prelude` instead of `42`. After fixing live lookup, an added required-module
case reproduced the same mismatch specifically on direct-native compilation.

After both fixes, the native-lang suite passed 27 tests. The compiler-focused
suite (`cargo test --offline --release --lib vm::compiler::tests -- --nocapture`)
passed 10 tests. The release executable was rebuilt, and the separate installed
`lang.core.script-macro` test file passed 16 facts. The migration's earlier
sequential candidate success alone did not establish dependency-loading parity.

The saved staged Python callback/emitter pipeline was replayed after both host
fixes: 244 passed with no failures, errors, timeouts, or skips (verification
`3d89e35e-c53f-40bf-bee6-6404ef11af0a`). This is staged pipeline evidence, not
completion of the original language installation flow.

JVM and browser parity for this change have not been verified.
