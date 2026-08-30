# Native releases

A native release ships host implementations only: the Rust CLI, JVM host,
browser packages, and trusted provider adapters. Canonical HAL, provider package
manifests, HARP archives, and the user-facing `hara` CLI remain in the Hara
source/package repositories.

1. Update `release/compatibility.json` for the new native version and ABI.
2. Run the native stacks in [core/rust/TESTING.md](core/rust/TESTING.md).
3. Update the Rust, Maven, and npm package versions together, then push a `v*`
   tag or run the **Native release** workflow for an existing tag.
4. The workflow builds `hara-native` for Linux x86_64, macOS x86_64, and macOS
   arm64; builds the JVM JAR; and packs both browser host packages.
5. The tagged release publishes the repository-owned Rust dependency chain
   (`hara-abi`, `hara-protocol-macros`, then `hara-native`) to crates.io. The
   first release requires a `CRATES_IO_TOKEN` in the protected `crates-io`
   environment; later releases may move to crates.io trusted publishing.
   The other `core/rust/crates/*` packages remain internal or separate product
   surfaces. PostgreSQL support is deliberately outside this core release
   train; a core runtime rejects PostgreSQL authority rather than loading an
   extension from another repository.
6. The same release builds and verifies a multi-platform OCI image containing
   the `hara-native` executable, then publishes it as
   `ghcr.io/hara-lang/hara-native:<version>` and records the immutable digest
   in the release payload.
7. The tagged release publishes `hara-native-jvm` to Maven and the browser/HTA
   packages to npm at `packages.hara-lang.org`. It requires the protected
   `packages` environment secrets `PACKAGES_MAVEN_USERNAME`,
   `PACKAGES_MAVEN_PASSWORD`, and `PACKAGES_NPM_TOKEN`.
8. The final publish job rejects source-bearing payloads, computes
   `SHA256SUMS`, and attaches `release-manifest.json` with each asset name,
   size, and exact SHA-256 to the GitHub release.

Maven and npm host packages are configured for
`packages.hara-lang.org` (`/maven/releases` and `/npm/`). Browser HARP package
resolution uses the registry root directly. Publishing credentials are supplied
only by the release environment; this repository never substitutes an unchecked
local source checkout.
