# Native releases

A native release ships host implementations only: the Rust CLI, JVM host,
browser packages, and trusted provider adapters. Canonical HAL, provider package
manifests, HARP archives, and the user-facing `hara` CLI remain in the Hara
source/package repositories.

1. Make every release change on `main`, including the single canonical version
   in `release/version.json`. It must agree with every public Cargo, Maven,
   npm, compatibility, and JVM version declaration.
2. Promote that reviewed commit with a merge-commit pull request from `main` to
   the protected `release` branch. `release` must be an ancestor of `main` when
   the pull request opens; do not squash or rebase this promotion. The release
   preflight runs the full native-host suite, validates the version contract,
   packages public artifacts, and checks the OCI build without publishing
   anything.
3. After the `release` preflight succeeds, dispatch **Native release promotion**
   on the current `release` branch. It refuses another ref, retests the commit,
   creates immutable `v<version>` draft-release intent, and serializes releases
   for that branch.
4. The promotion publishes the repository-owned Rust dependency chain
   (`hara-abi`, `hara-hta`, `hara-protocol-macros`, then `hara-native`) to
   crates.io using `CRATES_IO_TOKEN` from the protected `crates-io` environment. Other
   `core/rust/crates/*` packages remain internal or separate product surfaces.
5. It publishes `hara-native-jvm` through the Maven Central Portal and verifies
   the public `repo.maven.apache.org` coordinate. It publishes the exact packed
   browser/HTA tarballs to npmjs and verifies their public SRI integrities.
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
current `main` head; require the **release ready** check and pull requests on
`release`; and restrict the `crates-io` and `packages` environments to that
branch. `release` deliberately permits merge commits so every promotion retains
the corresponding `main` ancestry. Publication stays a manual workflow dispatch
from `release`; pushing a tag cannot publish.

`crates-io` stores `CRATES_IO_TOKEN`. `packages` stores the Maven Central Portal
user token as `MAVEN_CENTRAL_USERNAME` and `MAVEN_CENTRAL_PASSWORD`, the release
signing key as `MAVEN_GPG_PRIVATE_KEY` (and `MAVEN_GPG_PASSPHRASE` when the key
is encrypted), and the scoped, bypass-2FA npm token as `NPM_TOKEN`. Maven Central requires ownership of
the `org.hara-lang` namespace; npm requires publication permission for the
`@hara-lang` scope. Browser HARP package resolution continues to use the Hara
registry root, while Maven and npm consumers use Maven Central and npmjs.
Publishing credentials are supplied only by the release environment; this
repository never substitutes an unchecked local source checkout.

## Recovery and branch lineage

An existing tag or registry coordinate is accepted only when it belongs to the
same draft release, exact version, and exact artifact checksum. Otherwise stop,
correct the source on `main`, and release a new version. Do not overwrite or
retag a public artifact.

The partial `v0.1.6` attempt is retained only as audit evidence: its immutable
tag remains pinned to the abandoned release commit and the draft remains
unpublished. `archive/release-v0.1.6-aborted` preserves that branch head. The active
`release` branch is re-anchored once to the reviewed `0.1.7` main commit; future
promotions are merge commits from `main` so they stay conflict-free.
