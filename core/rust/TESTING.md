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

This is a serial cross-host compatibility layer, run in one CI job. It proves
three native-owned contracts: Rust executes the checked-in code-VM corpus; the
JVM executes every checked-in, core-only HBC0 artifact with a success-result
contract; and the browser HTA codec accepts the same canonical values,
including `MapEntry`, while rejecting malformed frames. HBC0 artifacts that
explicitly name `std.foundation/*` are checked as package-backed programs in
the source/package repository, after the relevant HARP is mounted. The legacy
`error/*` HBC0 records are intentionally held for the separate failure-ownership
lane while Java and Rust complete its shared contract. These vectors are
deliberately checked into this repository so a fresh native checkout needs
neither `hara` nor `hara-specs-registry`.

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
