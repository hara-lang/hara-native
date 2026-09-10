# Process-local quoted runtime values

Foundation's script macros quote grammar data containing live hydration
callbacks. Converting that generated value back to a reader-only form rejected
the callback (`cannot use function as code`). A quoted record also needs to
retain its type, metadata, and identity rather than becoming a plain map.

`value_to_form` now recognizes a two-element quoted list/cons. Portable quoted
data keeps its existing representation. If its payload cannot be represented
as portable forms, an internal `Form::RuntimeLiteral` owns the original value
through `RuntimeMetadata`. This is not a reader tag or a new Hara API. Unquoted
functions are still rejected as code.

The interpreter and VM recover the owned value without copying its callable
or record identity. Reading a global now returns the actual Var root; optimized
VM callable lookup occurs when that value is invoked, not when it is placed in
data. Equality and hashing implementations are unchanged.

The literal has process-local lifetime. Clones share ownership, and dropping
the last form releases its captures. The kernel encoder rejects it explicitly
with `cannot serialize process-local literal`; portable quoted forms continue
to encode/decode normally. No global literal table or reset mechanism is added.

Validation (Rust, release, offline):

- `cargo test --offline --release --test native-lang -- --nocapture`: 30 pass.
  Quoted callbacks retain identity, remain callable after their source Var is
  reset, and survive macro expansion and returned closures. Quoted records
  retain exact type, identity, and field value on interpreter and direct-native.
  The macro is installed before compiling its consumer; same-compilation-unit
  macro installation is not established by this test.
- `cargo test --offline --release --lib vm:: -- --nocapture`: 169 pass.
- `cargo test --offline --release --lib kernel:: -- --nocapture`: 78 pass,
  including process-local rejection and portable quote round trips.
- `cargo build --offline --release --bin hara-native`: successful.

The regression failed before this change with the callback conversion error,
then exposed a direct-native identity mismatch before the global-read fix.
These results do not establish JVM/browser parity or full language migration
completion.
