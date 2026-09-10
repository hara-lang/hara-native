# Live macro Vars

The original Foundation script installation path interns runtime callbacks
with `:macro true` on the name symbol. It does not require a separate grammar
registration API or a `Base/def` workaround.

Rust macro resolution now treats a mapped Var's current root and metadata as
authoritative. An ordinary callback marked as a macro expands; resetting its
Var to an ordinary value with `:macro false` disables expansion. Ordinary
referrals follow the same live Var. Non-function values remain valid interned
values even when their metadata contains `:macro true`; they do not expand.

Macro-only imports without a mapped Var retain the existing macro-registry
fallback. This change does not establish live referral semantics for that
separate import mode.

`Runtime/macroexpand-1` observes the dynamic evaluation namespace, including
inside `Runtime/eval-in`, and restores the caller namespace after success or
an expansion error. Stateful callers must restore both root and metadata;
an unannotated intern name intentionally preserves existing metadata.

## Verification (2026-09-10)

- Before the fix, the focused regressions exposed ordinary callback arity
  dispatch and an unexpanded macro in the scoped namespace.
- `cargo test --offline --release --test native-lang -- --nocapture`:
  26 passed with loopback socket access. The sandboxed run passed 25 and
  failed only the socket test with `Operation not permitted`.
- After adding the expansion-error restoration assertion,
  `cargo test --offline --release --test native-lang runtime_macro_vars -- --nocapture`:
  both focused tests passed, exercising interpreter and direct-native backends.
- `cargo build --offline --release --bin hara-native` succeeded.
- The rebuilt executable ran the installed `lang.base.grammar-macro` test
  file: 46 passed, no failures, errors, or skips.
- The saved, dependency-ordered Python callback/emitter migration pipeline
  was replayed with the rebuilt executable: 244 passed, no failures, errors,
  timeouts, or skips (verification ID
  `1bf75482-c59d-46fc-b3bf-de73e338aac8`). This is staged pipeline evidence,
  not installation of the Python model or completion of `script/install`.
- A fresh native Foundation probe using ordinary `intern`, unqualified
  `macroexpand-1`, and `eval` passed both checks, including restoration to
  a predeclared nil root with `:macro false` metadata (verification ID
  `74909dad-e0d6-4b59-bc95-5bcfaa401cfa`).

These results cover the Rust host, not JVM or browser parity. They do not
establish completion of `script/install` or removal of `grammar-api` consumers.
