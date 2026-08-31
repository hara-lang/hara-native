# Hara Native

`hara-native` is the host-runtime repository for Hara. It contains the Rust,
JVM/Truffle, browser/Wasm, and provider implementations that execute Hara
packages. It intentionally contains no canonical HAL source, embedded
Foundation bundle, or built-in end-user `hara` command.

The Hara source repository publishes signed HARP packages to
`packages.hara-lang.org`. `hara-native` provides the integrated local signer,
publisher-identity commands, and source-package publication request command;
the protected registry rebuilds, attests, and deploys the final release. A Hara
release pins the exact native artifacts it supports and supplies the full
`hara` command as a source-package wrapper. This repository can therefore
release a host fix without republishing Hara libraries, and a Hara library
release without rebuilding every host. For host contributors, start with the
[developer's guide](DEVELOPING.md). For publishing a source HARP package to
the official registry, use [PUBLISHING.md](PUBLISHING.md).

## Supported host surfaces

| Host | Artifact | Package boundary |
| --- | --- | --- |
| Rust | `hara-native` | `bundle verify`, `bundle install`, `bundle run`, `bundle exec`, `distribution build` |
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
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle build project-dir --output app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle verify app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle install app.harp
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle run app.harp --entry app.main/start
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle exec app.harp --entry app.main/start -- --help
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native -- distribution build project-dir --output target/app
```

`eval`, `run`, and `repl` use only the core host language. Library namespaces
are available only after a verified package is mounted.

`distribution build` requires `:project/distribution` in the source project's
`project.edn`. It copies the current host to `bin/<launcher>`, builds the HARP
archive at `lib/hara.harp`, and writes `lib/release.edn` with source and host
identity/version/digests. When the copied host starts under `bin/`, it verifies
that manifest before loading the HARP archive and invokes its declared HAL
entry with argv. It contains no product command behavior itself. A source entry
may return a declared host action; the current supported action is
`{:hara/host-action :resp}`. In a companion distribution only, that action
starts the source-backed RESP broker and prints `HARA RESP 127.0.0.1:<port>`.
The host accepts only the loopback address because RESP has no authentication;
the Hara package remains responsible for choosing the action and its command
semantics.

### Native project tests

`hara-native test` discovers the test paths declared by `project.edn`, creates
a fresh runtime for every `*.hal` file, and accepts a final `Test/run` summary
or `Test/check` Result vector. Limit a run without widening source discovery:

```text
hara-native test --project path/to/project
hara-native test --project path/to/project --file test/example/math_test.hal
```

`Test/register` stores a namespace-local fact with a non-empty `:desc`; `:name`
is retained as a compatibility alias. The registry preserves the supplied
`:meta` map (including `:refer` and `:id`) and adds `:test/namespace` and
`:test/order`. `Test/run` is the registry runner and returns the standard
summary shape; use `Test/check` for an ad hoc vector of comparisons.

The previous JSON host fixture command remains available as
`hara-native test-json SUITE.json [GROUP...]`. It is deliberately separate
from source-project testing.

## Release contract

Release metadata lives in [release/compatibility.json](release/compatibility.json).
Before a native release, run the commands in [core/rust/TESTING.md](core/rust/TESTING.md)
and confirm that no canonical HAL enters a host build or release artifact. The
two projects under [examples/](examples/README.md) are the sole checked-in HAL
fixtures: they are documentation/smoke inputs outside `core/rust`, and the
boundary check prunes that directory. Native artifacts must not contain
The canonical public version lives in [release/version.json](release/version.json);
[release/compatibility.json](release/compatibility.json) records its host ABI.
Changes promote from `main` to the protected `release` branch, where the
release preflight validates every public manifest and artifact before a
maintainer dispatches publication. See [RELEASES.md](RELEASES.md) for the
registry and recovery procedure. Native artifacts must not contain
`HARA_SOURCE_ROOT`, `core/lib`, `hal-src`, `std.foundation.hbx`, or `cli.hbx`
dependencies.

The source repository, not this one, owns the full `hara` CLI, canonical HAL,
package composition, registry publication policy, and end-user documentation.
This repository owns host implementation and contributor documentation.

## Conformance boundary

`make test-conformance` runs two Rust-produced, checksummed artifacts serially
in the Rust, JVM, and browser/Wasm hosts. `HNC1` is the native/protocol ABI
artifact generated from `core/rust/specs/native-protocol-v1.edn`. `HLC1` is the
functional language artifact generated from
`core/rust/specs/language-v1.edn`; each case records parser, evaluator, or
lowered-native-ABI coverage and executes the canonical HBC0 bytes. Both
specifications use structured forms, never HAL files. The imported registry
fixtures under `core/rust/specs/language/registry` are verbatim provenance
references only: they never execute directly, and every adopted case is checked
against its imported fixture or `lowering-v1.edn` before HLC1 generation.

HLC1 deliberately excludes Foundation and source-package behavior. Foundation
wrappers are lowered into direct `std.native.*` and `std.protocol.*` calls where
the ABI is lossless; source-owned wrappers, `require`, namespace aliases, and
package loading remain outside this source-free host boundary.

The corpus has two complementary layers. Hand-authored cases lower every
portable native method into direct operations (numeric, strings, bytes,
collections, codecs, results, Base, iterators, and receiver boundaries). The
artifact generator validates that exact native inventory, then appends
functionally executed extension-dispatch, missing-arity, and
unsupported-receiver cases for every portable protocol method.

The native `:coverage :native/portable` inventory owns every portable native
method. Generation rejects an unknown, non-portable, duplicate, or
no-longer-directly-invoked entry. Every portable protocol method receives
generated extension success, missing-arity, and unsupported-receiver coverage.
`IEncodable/encode-with` is the deliberate exception because it has universal
default dispatch.
Adding or removing a declaration therefore expands the executed corpus rather
than silently changing an aggregate count.

`make test-conformance` is the strict portable layer. It proves the same HNC1
and HLC1 artifacts in Rust, JVM, and browser/Wasm without external providers.
`make test-conformance-full` adds the trusted provider-host profile; use it for
capability-bound provider behavior such as filesystem adapters. Capability
declarations are held in the native registry and their behavior belongs to a
profile; the portable corpus must not depend on a real machine, network,
process, or package registry.

## Declaration-package ABI

Source packages own Hara surface forms and lower them into this host ABI. The
host does not register a Foundation declaration namespace or a second source
interpreter:

| Source form | Native lowering |
| --- | --- |
| `def` | `std.native.Base/def` with `Base/current-namespace`, an unqualified symbol, value, and metadata-or-`nil` |
| `defn` | Construct the function in the source package, then use `Base/def`; schemas remain metadata. `defn-` is unsupported and rejected as an unbound symbol. |
| `defmacro` | Construct the expander with its `form` and `environment` inputs, then use `Base/def` with `{:macro true}` metadata |
| `defstruct` / `defmutable` | `Base/struct` / `Base/mutable` with a native Vector of fields |
| `defprotocol` | `Base/protocol` with a method-to-arity map and a native Vector of parents |
| `extend-type` | `Base/extend` with the declared type, protocol, and method-to-function map |
| `field` | `Base/field` with the mutable value and field keyword or symbol |

`ns`, `ns+`, and `require` intentionally stay host directives. They select or
reuse the compilation namespace and coordinate verified package loading before
ordinary source forms expand; they are not `std.native.Base` calls. A source
package must use the explicit `Base/resolve` + `IDeref/deref` sequence when it
needs to call a dynamically resolved Var. Struct and mutable constructors are
the published `->Type` Vars, not an implicit promise that a type descriptor is
callable.

The registry copy is a read-only mirror, never a second source of truth. When
the registry checkout is available, regenerate and validate it explicitly:

```text
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- mirror /path/to/hara-specs-registry/01-lang/010-bytecode/draft/conformance/native-protocol-v1.edn
cargo run --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- check-mirror /path/to/hara-specs-registry/01-lang/010-bytecode/draft/conformance/native-protocol-v1.edn
```
