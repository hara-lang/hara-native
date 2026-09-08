# Quoted constant metadata

The pinned Lua C-FFI tests exposed metadata loss in the bytecode constant pool.
Three equal quoted vectors containing `x` with `{:tag :int}`, `{:- :double}`,
and no metadata all reused the first vector.
Consequently, C declarations incorrectly retained `int` for the later `double`
and default `void*` arguments.

Value equality deliberately ignores metadata. Quoted literals now use the
existing non-interned constant path, also used for literal vectors, so metadata
and concrete quoted collection representations are not merged by that equality.
This does not change language equality or introduce a public operation.

The permanent nested-symbol metadata regression fails before the correction
with `[{:tag :int} {:tag :int} {:tag :int}]`, and passes afterward with
`[{:tag :int} {:- :double} nil]`. All 84 VM execution tests pass. The release
native host was rebuilt; the migrated Lua C-FFI file then passes all eight
historical facts (31 assertions) plus one sanitizer provenance/idempotence fact.
Those original declaration expectations were not weakened or renamed.

The nine native tests pass from the written file in fresh processes and all nine
fail under deliberately wrong expectations. This is Rust native execution
evidence, not a new JVM or browser conformance claim.
