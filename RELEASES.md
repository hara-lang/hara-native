# Native releases

A native release ships host implementations only: the Rust CLI, JVM host,
browser packages, and trusted provider adapters. Canonical HAL, provider package
manifests, HARP archives, and the user-facing `hara` CLI remain in the Hara
source/package repositories.

1. Make every release change on `main`, including the single canonical version
   in `release/version.json`. It must agree with every public Cargo, Maven,
   npm, compatibility, and JVM version declaration.
2. Promote that reviewed commit with a pull request from `main` to the protected
   `release` branch. The release preflight runs the full native-host suite,
   validates the version contract, packages public artifacts, and checks the
   OCI build without publishing anything.
3. After the `release` preflight succeeds, dispatch **Native release promotion**
   on the current `release` branch. It refuses another ref, retests the commit,
   creates immutable `v<version>` draft-release intent, and serializes releases
   for that branch.
4. The promotion publishes the repository-owned Rust dependency chain
   (`hara-abi`, `hara-hta`, `hara-protocol-macros`, then `hara-native`) to
   crates.io using `CRATES_IO_TOKEN` from the protected `crates-io` environment. Other
   `core/rust/crates/*` packages remain internal or separate product surfaces.
5. It publishes `hara-native-jvm` to Maven and the browser/HTA packages to npm
   at `packages.hara-lang.org`, using the protected `packages` environment
   secrets `PACKAGES_MAVEN_USERNAME`, `PACKAGES_MAVEN_PASSWORD`, and
   `PACKAGES_NPM_TOKEN`.
6. It also publishes and pull-runs the multi-platform image
   `ghcr.io/hara-lang/hara-native:<version>`. The workflow creates Linux and
   macOS CLI archives, verifies fresh crates.io/Maven/npm consumers, records
   registry integrities and the immutable OCI digest, then finalizes the GitHub
   release with `SHA256SUMS` and `release-manifest.json`.

If a registry publication fails after the draft intent is created, correct only
the delivery problem and rerun the same `release` commit. The workflow permits
an existing artifact only for that exact draft/tag recovery path; a source or
version change requires a new version on `main`.

Repository administrators bootstrap this once: create `release` from the
current `main` head; require the **Native release preflight / release ready**
check and pull requests on `release`; and restrict the `crates-io` and
`packages` environments to protected branches. Add release-maintainer
environment reviewers if a second approval is required. `crates-io` stores
`CRATES_IO_TOKEN`; `packages` stores `PACKAGES_MAVEN_USERNAME`,
`PACKAGES_MAVEN_PASSWORD`, and `PACKAGES_NPM_TOKEN`. Publication stays a
manual workflow dispatch from `release`; pushing a tag cannot publish.

Maven and npm host packages are configured for
`packages.hara-lang.org` (`/maven/releases` and `/npm/`). Browser HARP package
resolution uses the registry root directly. Publishing credentials are supplied
only by the release environment; this repository never substitutes an unchecked
local source checkout.
