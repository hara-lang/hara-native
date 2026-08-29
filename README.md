# Hara Native

`hara-native` is the host-runtime repository for Hara. It contains the Rust,
JVM/Truffle, browser/Wasm, and provider implementations that execute Hara
packages. It intentionally contains no canonical HAL source, source test
corpus, embedded Foundation bundle, or end-user `hara` command.

The Hara source repository publishes signed HARP packages to
`packages.hara-lang.org`. A Hara release pins the exact native artifacts it
supports and supplies the full `hara` command as a source-package wrapper.
This repository can therefore release a host fix without republishing Hara
libraries, and a Hara library release without rebuilding every host.

## Supported host surfaces

| Host | Artifact | Package boundary |
| --- | --- | --- |
| Rust | `hara-native` | `bundle verify`, `bundle install`, `bundle run` |
| JVM | `org.hara-lang:hara-native-jvm` | verified `package.edn` archive and JVM flavor loader |
| Browser | `@hara-lang/native-browser` | `inspectHarp` and `activateLockedPackages` |
| Providers | `providers/` | host adapters selected by verified HARP extensions |

The browser host verifies a complete archive and its signatures before it
registers a namespace. JVM package installation verifies the package index and
every installed artifact before a JVM flavor is loaded.

## Rust CLI

Build and run the generic host from this checkout:

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- eval "(+ 20 22)"
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle verify app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle install app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle run app.harp --entry app.main/start
```

`eval`, `run`, and `repl` use only the core host language. Library namespaces
are available only after a verified package is mounted. `test` evaluates a
host fixture; the Hara source package supplies the language-level test runner
and the user-facing test selection policy.

## Release contract

Release metadata lives in [release/compatibility.json](release/compatibility.json).
Before a native release, run the commands in [core/rust/TESTING.md](core/rust/TESTING.md)
and confirm the repository has no `*.hal` files. Native artifacts must not
contain `HARA_SOURCE_ROOT`, `core/lib`, `hal-src`, `std.foundation.hbx`, or
`cli.hbx` dependencies.

The source repository, not this one, owns the full `hara` CLI, canonical HAL,
package composition, registry publication policy, and end-user documentation.

## Conformance boundary

`make test-conformance` runs the native compatibility vectors serially: the
Rust code-VM corpus, core-only success-result Rust-produced HBC0 artifacts on
the JVM, and browser HTA canonical-value frames. HBC0 artifacts that name
`std.foundation/*` are package-backed and run with the corresponding HARP in
the source/package repository; legacy `error/*` HBC0 records await the shared
Rust/JVM failure-ownership contract. Language and standard-library conformance
is therefore intentionally not a prerequisite of a standalone native-host
checkout.
