# Runtime/eval retains form metadata

The direct-native Runtime/eval boundary must compile the Form produced by
value_to_form without printing it. Form's display representation intentionally
omits metadata; printing and reparsing dropped dynamic definition metadata.
The Foundation defonce macro preserves name metadata but calls Runtime/eval,
so its dynamically declared Vars could not be bound under this path.

The source, value and eval-in paths now share direct-native execution of
spanned forms. Text input is still parsed. Value input receives synthetic spans
and retains its structural metadata. Compilation keeps namespace preparation
and unbound-global handling enabled, as in the prior source compiler path.
Captured namespaces, providers and nested multimethod propagation are unchanged.
This adds no Hara API and does not alter Form display or defonce semantics.

Validation (2026-09-10):

- The new direct_native_eval_preserves_dynamic_definition_metadata regression
  failed before the fix with `Var has no dynamic binding`.
- After the fix, the regression checks the dynamic marker, the temporary bound
  value, and restoration of the root value: `[true 11 7]`.
- The corresponding eval-in regression also failed before its repair with
  `Var has no dynamic binding`. It now checks metadata, binding restoration
  and empty-input behavior: `[true 11 7 nil]`.
- `cargo test --test native-lang`: 24 passed. The socket test requires local
  loopback permission; its sandbox-denied run is not a semantic regression.
- `cargo build --release --bin hara-native`: passed.
- The eight staged original runtime.basic.type-common option-state facts pass
  with the rebuilt executable. Before this fix all eight errored on binding.
  Those candidates are not yet an installed type-common port.
- `git diff --check`: passed.

Scope: direct-native Runtime/eval, form-sequence Runtime/eval-in and the shared
text-evaluation execution path. Eval-in retains its original do wrapper and
namespace restoration; its callable path is unchanged. Other host backends
were not modified by this repair.
