## Hara Native host release

Hara Native is the source-free host/runtime distribution for the Hara language.
This release delivers the Rust CLI, JVM host, browser packages, and trusted
provider adapters; canonical HAL, provider package manifests, HARP archives,
and the user-facing `hara` CLI remain in the Hara source/package repositories.

### Delivered artifacts

- Native CLI archives for Linux x86_64, Linux ARM64, macOS Intel, and Apple
  Silicon.
- Repository-owned Rust crates on crates.io.
- `hara-native-jvm` on Maven Central.
- `@hara-lang/native-browser` and `@hara-lang/hta` on npm.
- `ghcr.io/hara-lang/hara-native` multi-platform OCI image.

### Verification and integrity

- Native conformance is run across Rust, JVM, and browser before promotion.
- Published registries and OCI platform images are checked after publication.
- `SHA256SUMS` and `release-manifest.json` below record the immutable release
  artifacts, package coordinates, and OCI digest.

Use the version displayed by this release for package-manager installation or
select the matching platform archive below. The generated change list that
follows describes what changed since the previous release.
