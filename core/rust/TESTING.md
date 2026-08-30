# Native host test stacks

Run commands from the repository root. Every stack is intentionally independent
of a sibling `hara` checkout. `make test` runs every layer serially; individual
targets below are exactly the targets used by CI.

## Rust CLI and raw Wasm

```text
make test-rust
make test-raw
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- eval "(+ 20 22)"
```

The CLI suite checks generic command parsing, a nonempty serial test selection,
unknown-group rejection, failure summaries, and verified archive installation.
The raw-Wasm suite checks the host and HARP boundary without an embedded
Foundation package.

## JVM host

```text
make test-jvm
java -jar core/java/target/hara-native-jvm-0.1.0.jar eval "(+ 20 22)"
```

The Maven suite is explicitly scoped to the native CLI serial runner and HARP
manifest/install integrity checks. Source-library conformance is owned by the
Hara package repository.

## Native conformance

```text
make test-conformance
```

This is the serial first-layer conformance layer. Rust, JVM, and browser each
decode the checked-in `HNC1` artifact and run its native suite followed by its
protocol suite in one host runtime. The artifact is generated from
`specs/native-protocol-v1.edn`, uses only direct `std.native.*` and
`std.protocol.*` operations, and is rejected if it requires Foundation.

The source contains the exact deterministic behavior cases. At generation time
the native registry contributes independent resolver cases for every portable
type and method, while the protocol registry contributes resolver cases for
every guest-visible protocol type and method plus guest dispatch and arity
cases for each portable protocol method. Do not replace these generated cases
with a hard-coded total or an aggregate "catalog passes" assertion.

`specs/native-protocol-v1.edn` also owns the grouped
`:coverage :native/portable` list: the 145 deterministic methods promoted from
the prior registry fixture. Generation verifies each name is a live portable
declaration and appears as a direct call in a native program; a test proves
that removing one call fails generation. The remaining portable native methods
are inventory/resolver-covered until a stable direct fixture is promoted.
Every portable protocol method has generated extension-success, missing-arity,
and unsupported-receiver cases; `IEncodable/encode-with` is excluded only
because its universal default dispatch is specified behavior.

To extend it, add a structured `:program` form and exact `:expect` display to
the appropriate suite, then regenerate and verify the artifact:

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- generate
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- check
make test-conformance
make test-conformance-full
```

Do not add raw source strings, `.hal` fixtures, registry paths, or Foundation
calls. New native and protocol semantics belong in this artifact; parser,
evaluator, standard-library, and source-level corpora remain outside the native
host repository.

Use `test-conformance-full` when a capability-bound operation needs its
deterministic trusted-provider profile. The strict portable artifact must still
be runnable in every host without a real filesystem, process, network, or
registry service.

When publishing the registry mirror, keep the native EDN file authoritative:

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- mirror /path/to/hara-specs-registry/01-lang/010-bytecode/draft/conformance/native-protocol-v1.edn
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- check-mirror /path/to/hara-specs-registry/01-lang/010-bytecode/draft/conformance/native-protocol-v1.edn
```

The mirror is a publication artifact. Do not edit it directly or feed it back
into HNC generation.

This is not the Hara language conformance suite. Parser, evaluator, standard
library, and source-level behavioral corpora remain versioned with the
canonical HAL and run from the Hara source/package repository.

## Browser, HTA, and provider hosts

```text
make test-browser-integrity
make test-provider-hosts
```

For a generated browser SDK test, use a Rust toolchain with
`wasm32-unknown-unknown` and `wasm-bindgen-cli` installed. The profile build,
SDK test, and native-only Chromium smoke test are:

```text
make test-browser-sdk
make test-browser-playwright
```

Both commands build `native-vm` and `native-full` from the current checkout;
they do not reuse a sibling Hara repository or a pre-generated Wasm directory.

`test:provider-hosts` covers trusted filesystem adapters only. Provider package
manifests and prebuilt Wasm façades are fetched as verified HARP packages from
`packages.hara-lang.org`.

## Source-free boundary

```text
test -z "$(find . -type f -name '*.hal' -print -quit)"
test -z "$(find providers -type f \( -name 'project.edn' -o -name 'extension.edn' -o -name 'provider.sha256' \) -print -quit)"
```

Both commands must succeed. The CI workflow also scans native build inputs for
the old source-root and embedded-bundle paths.
