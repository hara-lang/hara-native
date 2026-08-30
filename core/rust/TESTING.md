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
unknown-group rejection, failure summaries, local source-package archive build,
and verified archive installation. The raw-Wasm suite checks the host and HARP
boundary without an embedded Foundation package.

## Native Test runner

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- test --project /path/to/project
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- test --project /path/to/project --file test/example/math_test.hal
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- test-json fixtures.json smoke
```

The project runner discovers only the `:project/test-paths` from `project.edn`,
loads the project source catalog, and evaluates each selected Test file in a
fresh runtime. A test file must finish with a `Test/run` summary or a
`Test/check` Result vector. The native registry uses `:desc` as the canonical
fact identity, preserves supplied `:meta` such as `:refer` and `:id`, and
retains `:name` only as a compatibility alias. `Test/run` never accepts an ad
hoc case vector; use `Test/check` for that legacy shape.

## Native publication client

```text
make test-signer
make test-publication
```

`hara-native signer` manages a local development key and preserves the legacy
stdin/stdout `HARA_SIGNER` protocol for compatible external clients.
`hara-native id enroll` and `hara-native publish` use that signer directly in
the native process: they do not create a child signer process. The signer tests
prove a generated 0600 private seed derives the reported public key, an emitted
signature verifies against that key, and a publisher key id cannot escape the
EDN response. The publication target covers native command parsing and the
in-process enrollment signer against exact canonical enrollment bytes. It does
not contact the identity or registry services. Publication workflow and
production-key guidance live in [PUBLISHING.md](../../PUBLISHING.md).

## JVM host

```text
make test-jvm
java -jar core/java/target/hara-native-jvm-0.1.6.jar eval "(+ 20 22)"
```

The Maven suite is explicitly scoped to the native CLI serial runner, HARP
manifest/install integrity checks, and the JVM-native Test registry runner.
The JVM test proves that a direct `Test/register` / `Test/run` file preserves
`:desc`, `:refer`, and `:id` metadata through the isolated host result. Source
library conformance is owned by the Hara package repository.

## Native conformance

```text
make test-conformance
```

This is the serial first-layer conformance layer. Rust, JVM, and browser each
decode the checked-in `HNC1` native/protocol artifact and `HLC1` functional
language artifact. HNC1 runs its native suite followed by its protocol suite
in one host runtime. HLC1 executes every Rust-produced HBC0 case in a fresh
runtime and covers parser, evaluator, and lowered-native-ABI behavior.
`specs/native-protocol-v1.edn` and `specs/language-v1.edn` are the editable
sources; both reject Foundation dependencies.

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

To extend HNC1, add a structured `:program` form and exact `:expect` display to
the appropriate suite. To extend HLC1, adopt a bytecode VM fixture from the
verbatim local registry import or a case from `specs/lowering-v1.edn`; the
generator rejects a drifted source program or expectation. Regenerate and
verify the artifacts:

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- generate
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- check
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-language-conformance-artifact -- generate
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-language-conformance-artifact -- check
make test-conformance
make test-conformance-full
```

Do not add `.hal` fixtures or Foundation calls to `core/rust`. Registry material
belongs under `specs/language/registry` as a verbatim, non-executable
provenance import; HLC1 contains only the adopted, generated HBC0 cases. HLC1
covers the core parser/evaluator and ABI lowering; standard-library and
source-package semantics remain outside the source-free host boundary. The
root `examples/` directory is the only exception in the repository: its source
projects are explicit package smoke fixtures and are not Cargo or release
inputs.

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

HLC1 is the source-free host subset of language conformance, not a replacement
for the full Hara language suite. Standard-library and source-package behavioral
corpora remain versioned with canonical HAL and run from the Hara
source/package repository.

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
test -z "$(find . -path './examples' -prune -o -type f -name '*.hal' -print -quit)"
test -z "$(find providers -type f \( -name 'project.edn' -o -name 'extension.edn' -o -name 'provider.sha256' \) -print -quit)"
```

Both commands must succeed. The first intentionally prunes the root smoke
fixtures while rejecting HAL everywhere that can feed the native host. The CI
workflow also scans native build inputs for the old source-root and
embedded-bundle paths.
