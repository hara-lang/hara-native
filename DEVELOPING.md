# Developing Hara Native

`hara-native` is the portable host for verified Hara packages. It owns the
Rust CLI and runtime, JVM/Truffle host, browser/Wasm host, package integrity,
and provider bridges. It is intentionally not the full Hara source
distribution: canonical `.hal` libraries, source compilation, package
composition, and the end-user `hara` CLI are owned by the Hara source
repository.

This guide is for contributors working on the host boundary, package loading,
providers, or browser profiles.

## Repository map

| Path | Responsibility |
| --- | --- |
| `core/rust/` | generic CLI, package verifier/installer, runtime, bytecode VM, JIT, raw Wasm, and browser build inputs |
| `core/java/` | JVM/Truffle host and HARP loading tests |
| `core/rust/web/` | browser SDK, archive/HTA verification, workers, and browser provider boundary |
| `providers/` | provider contracts and host adapters; providers are selected from verified packages |
| `examples/` | two small source-package smoke fixtures, intentionally outside native build and release inputs |
| `release/compatibility.json` | host/package compatibility information used for a release |

Read [core/rust/ARCHITECTURE.md](core/rust/ARCHITECTURE.md) for the runtime
layers, [core/rust/HNW0_HTA_BOUNDARY.md](core/rust/HNW0_HTA_BOUNDARY.md) for
package and provider lifecycle, and [core/rust/LIVE_SESSION.md](core/rust/LIVE_SESSION.md)
for session and execution-backend details.

## Build and run the generic host

Install the Rust toolchain specified by `core/rust/Cargo.toml`. Build or run
the host from the repository root:

```text
cargo build --manifest-path core/rust/Cargo.toml --bin hara-native
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- eval "(+ 19 23)"
```

The generic host can evaluate only its core language. `eval`, `run`, and `repl`
do not make Foundation or other source libraries ambient. To use library code,
put it in a HARP package, verify it, and run an explicit entry:

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle verify app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle install app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle run app.harp --entry app.main/start
```

Use a throwaway `HARA_DIST_HOME` while testing package installation so the
test does not rely on or alter a developer's package store:

```text
HARA_DIST_HOME="$(mktemp -d)" cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- \
  bundle run app.harp --entry app.main/start
```

## Source-package smoke workflow

`hara-native bundle build` packages a source project into a HARP archive, then
the same executable verifies and runs that archive. The checked-in fixtures in
[examples/](examples/README.md) demonstrate both a single namespace entry
point and a project-local `require`.

Run both fixtures with:

```text
make test-examples
```

The target is deliberately not part of `make test`, because it is a
user-facing source-package smoke flow while the generic-host suite must remain
source-free.

```text
mkdir -p target/smoke-walkthrough
cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- \
  bundle build examples/smoke-answer --output target/smoke-walkthrough/smoke-answer.harp
HARA_DIST_HOME="$(mktemp -d)" cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- \
  bundle verify target/smoke-walkthrough/smoke-answer.harp
HARA_DIST_HOME="$(mktemp -d)" cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- \
  bundle run target/smoke-walkthrough/smoke-answer.harp --entry hara-native.smoke.answer.main/main
```

The last command must print `42`. The source project declares its source paths,
entry namespace, dependencies, and requested capabilities in `project.edn`.
Keep capability declarations minimal: the archive declares what it needs, and a
host/provider profile decides what it can actually receive.

The root `examples/` directory is the sole checked-in `.hal` exception. Its
projects are documentation and smoke inputs only. Do not add canonical source
libraries below `core/`, provider implementations, release inputs, or Cargo
package inputs.

For a separate source-package repository that will publish to
`packages.hara-lang.org`, follow [PUBLISHING.md](PUBLISHING.md). The
`hara-native publish` command signs and submits a tag-bound registry request
with the integrated signer. It is not a local deployment command: the registry
rebuilds, attests, and deploys an accepted package. Publication is not part of
this repository's `make test` workflow.

## Validation layers

Run commands from the repository root. The default fast loop is:

```text
make test-boundary
make test-rust
```

Use the smallest layer that covers a change, then run the broader affected
layer before handoff:

| Change area | Required validation |
| --- | --- |
| Rust runtime or CLI | `make test-rust` |
| raw Wasm boundary | `make test-raw` |
| JVM host | `make test-jvm` |
| portable runtime semantics | `make test-conformance` |
| capability-bound provider semantics | `make test-conformance-full` and `make test-provider-hosts` |
| browser archive/HTA boundary | `make test-browser-integrity` |
| generated browser SDK/profile | `make test-browser-sdk`; use `make test-browser-playwright` for a Chromium smoke test |
| release/package boundary | `make test-boundary` plus the relevant host/package tests |
| local development signer | `make test-signer` |

`make test` runs every layer serially. Browser SDK builds require the
`wasm32-unknown-unknown` Rust target and `wasm-bindgen-cli`; the browser testing
guide explains the profile build in more detail.

The boundary test rejects canonical HAL, source-tree dependencies, embedded
Foundation bundles, and provider manifests in host inputs. It expressly prunes
the root smoke fixtures, which cannot enter the `core/rust` Cargo package or a
release archive. Keep that exception narrow.

## Conformance and execution profiles

`make test-conformance` runs the deterministic `HNC1` native/protocol artifact
and `HLC1` core-language artifact in Rust, JVM, and browser hosts. These are
structured native specifications and generated bytecode; they are not a second
source interpreter and do not cover Foundation packages.

The browser produces two SDK profiles:

| Profile | Role |
| --- | --- |
| `native-vm` | browser wrapper around the bytecode VM with verified package activation |
| `native-full` | browser host with the complete supported native runtime surface, including the whole-Wasm execution path |

Whole-Wasm execution means validated HBC is lowered into a standalone Wasm
module and executed by the Wasm engine. It is a runtime compilation target for
already compiled package bytecode; it does not compile `.hal` source.

## Providers and package authority

Providers are explicit host bridges for capabilities that the portable runtime
does not own, such as filesystem access, transports, async values, or extension
Wasm. A package declaration identifies the provider and ABI. The host verifies
the HARP archive and declaration before starting it, gives it only the selected
capability profile, and owns lifecycle cancellation, release, and shutdown.

Provider work must preserve that ordering: no provider starts from an ambient
namespace lookup or before package verification. See
[providers/README.md](providers/README.md) and
[core/rust/HNW0_HTA_BOUNDARY.md](core/rust/HNW0_HTA_BOUNDARY.md) before changing
a provider or its lifecycle contract.

## Practical contribution rules

- Keep host source source-free. The root examples are the documented smoke-only exception.
- Add behavior-focused tests near the owning Rust, JVM, or browser component.
- Keep package verification before mount, activation, or provider startup.
- Use an isolated package store for install/run tests and clean it on test exit.
- Regenerate conformance artifacts only with their native generator and validate them with its `check` command; never hand-edit generated artifacts.
- Read [core/rust/TESTING.md](core/rust/TESTING.md) for exact test ownership and commands.
