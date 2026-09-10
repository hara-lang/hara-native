# Cache object identity

The approved migration adaptation compares actual grammar and entry objects,
not their identity hashes. `Base/identical?` accepts two persistent hash maps
or native records and returns whether they are the same runtime object.
Mixed map/record arguments return false. Other values are explicitly rejected;
this boundary does not define scalar boxing identity or identity for every
collection type. No value equality or hashing behavior changes.

Rust records compare their existing shared references. Rust persistent maps
carry an owned reference-counted identity token: ordinary runtime clones retain
it, while construction, association, successful removal and metadata replacement
produce new identities. A missing-key removal may return the existing object.
Sharing a persistent tree root alone does not establish object identity.
JVM maps and records compare their actual object references after unboxing.

There is no identity registry, numeric ID or serialization of identity. Tokens
are released with their last owner. This adds one reference-counted token to
each Rust map object; no claim of zero allocation overhead is made. Rebuilt or
deserialized objects have fresh identity. Cache entries must retain the actual
objects and release them on replacement/reset. They must compare ordinary
language/options fields by value and grammar/entry fields with `Base/identical?`.
Putting retained objects into a value-equality comparison vector is insufficient.

The host regression covers aliases, equal-valued replacements, cache-state
retrieval, changed metadata, persistent updates, mixed types and invalid calls,
on Rust interpreter/direct-native and JVM. Map unit coverage also distinguishes
shared-root metadata wrappers and verifies token release after the last alias.
The initial host regression failed on the old runtime with an unbound identity
method. This is a host capability, not completed emitter-cache integration.

Validation: `cargo test --test native-lang -- --nocapture` passes 20 tests
(loopback access is required by the existing socket test);
`cargo test --lib lang::data::map::tests -- --nocapture` passes 10 tests;
`mvn -q -Djacoco.skip=true -Dtest=HaraNativeLangTest test` passes 9 tests.
Coverage instrumentation was disabled because the installed JaCoCo version
does not support the current JDK class-file version. These are focused host
checks, not the full migration pipeline suite.
