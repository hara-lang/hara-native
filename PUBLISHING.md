# Publishing Hara packages

This guide publishes a Hara source package to the official
`https://packages.hara-lang.org` registry. It is deliberately separate from a
native-host release:

- A HARP package publication publishes source-project provenance and a
  reproducible recipe. The registry rebuilds and attests the archive.
- A `hara-native` release publishes host artifacts (the JVM package and browser
  npm packages) through its tagged GitHub workflow. See [RELEASES.md](RELEASES.md).

`hara-native` has an integrated development signer and publication client. It
does not compile `.hal` source itself: it validates the source project and
recipe, binds publication to the signed source tag, signs the request, and
submits it to the registry. The registry performs the deployment boundary: it
rebuilds the HARP archive, verifies the policy and signatures, records the
immutable release, creates an attestation, and makes the verified release
available.

Never upload a `.harp` with `curl`, copy it into a registry bucket, or put a
registry token or private key in this repository. The publisher CLI sends a
canonical signed intent to the registry; the registry verifies the identity
grant, signed source tag, recipe, and rebuilt archive before it records a
release.

## What must be ready

Prepare a real source-package repository. The smoke projects in
[examples/](examples/README.md) prove the local package/host boundary, but are
not release recipes and should not be published at their sample versions.

The project needs an immutable package version and an official-tap coordinate.
A project id without a tap prefix is normalized to the `hara` tap, so this:

```clojure
{:project/id acme/widgets
 :project/version "1.2.3"
 :project/recipe "hara.recipe.edn"}
```

publishes the canonical coordinate `hara:acme/widgets`. An alternate tap must
be explicit (for example, `partner:acme/widgets`) and selected with the matching
`--tap` value.

Official publication also requires `hara.recipe.edn`. It describes a
reproducible build, not a shell script. A minimal HAL-source recipe is:

```clojure
{:recipe/format "0.0.0-alpha"
 :recipe/adapter :hal
 :recipe/toolchain {}
 :recipe/inputs {}
 :recipe/outputs []}
```

The release recipe must contain all five fields above. Official recipes reject
commands, scripts, and shell fragments. Expand the typed toolchain, inputs, and
outputs as the package acquires real build inputs.

## Create a local development signer

For local development, `hara-native` provides signer commands. It stores a
random 32-byte Ed25519 seed in a new Unix `0600` file. The native `id enroll`
and `publish` commands sign in-process, so there is no `HARA_SIGNER` wrapper or
separate publisher executable to configure. This seed-file signer is a local
development facility, not a team or production key-management solution. The
native publisher currently uses only this integrated signer; do not use it for
production until a reviewed keychain, HSM, or signing-service backend is added.

```text
make build-signer
install -d -m 700 "$HOME/.local/state/hara"
core/rust/target/release/hara-native signer generate --key-file "$HOME/.local/state/hara/publisher.ed25519"
export HARA_NATIVE="$PWD/core/rust/target/release/hara-native"
export HARA_SIGNER_KEY_FILE="$HOME/.local/state/hara/publisher.ed25519"
export HARA_SIGNER_KEY_ID="YOUR_GITHUB_OWNER-2026-01"
export HARA_OFFICIAL_ROOT_SHA256="sha256:8861d398c14a53b2fe13f7736310bb2c55624260c84e131452457e8aa69ac3dc"
```

`generate` refuses to overwrite a key file and never prints the seed. The
signer rejects symlinks, non-regular files, relative paths, and key files that
are group- or world-readable. Test the compatible output shape with:

```text
printf '%s' '{:intent/format "0.0.0-alpha"}' | "$HARA_NATIVE" signer sign
```

The emitted `:key/id` must be the exact id that is enrolled and granted below.
The no-argument executable mode preserves the stdin/stdout protocol for legacy
clients that configure `HARA_SIGNER`; the native `id enroll` and `publish`
commands do not use that compatibility path.

`HARA_OFFICIAL_ROOT_SHA256` is a public, pinned trust anchor for the official
identity policy, not a secret. It is the SHA-256 fingerprint of the official
identity root public key published by [Hara Identity](https://id.hara-lang.org/).
The native client uses it to reject a policy whose signed root key is not the
one expected for the official tap.

## Establish a publisher identity

The private key stays outside the repository. `hara-native id enroll` derives
the corresponding public key from `HARA_SIGNER_KEY_FILE`, obtains or accepts a
one-time enrollment challenge, and signs that canonical enrollment proof. It
sends the public key, key id, owner, challenge, and signature to the identity
service; it never sends the private key.

Authenticate and enroll the public key:

```text
"$HARA_NATIVE" id login
"$HARA_NATIVE" id enroll --tap hara --owner YOUR_GITHUB_OWNER
"$HARA_NATIVE" id status
```

Enrollment alone is not authorization to publish. An identity-policy maintainer
must grant the enrolled key the exact coordinate—for example,
`hara:acme/widgets`. A dry run below proves that the key, policy revision, and
coordinate grant all agree.

## Validate and tag the source release

Run the project’s required checks from the package root. The publication dry
run below performs the signed-tag, recipe, origin, policy, and authorization
preflight; it does not submit a request. To inspect the local package boundary
before publishing, build and verify the archive with the same native executable
that will sign and submit the request:

```text
"$HARA_NATIVE" bundle build . --output /tmp/acme-widgets-1.2.3.harp
HARA_DIST_HOME="$(mktemp -d)" "$HARA_NATIVE" bundle verify /tmp/acme-widgets-1.2.3.harp
```

The publisher binds a release to a Git tag named exactly `v` plus the package
version. Ensure the worktree is clean, push the source commit, create a signed
tag, and verify it locally and on the remote before requesting publication:

```text
git status --short
git push origin main
git tag -s v1.2.3 -m "acme/widgets 1.2.3"
git verify-tag v1.2.3
git push origin v1.2.3
git ls-remote --tags origin refs/tags/v1.2.3
```

Use your repository’s normal protected-branch/release policy when the source
branch or tag requires review. Do not move or reuse a published version tag.

### Local preflight before signing the tag

While preparing a real package repository, this local-only diagnostic can run
the remaining preflight checks before a release tag exists:

```text
"$HARA_NATIVE" publish --tap hara --dry-run --skip-signed-tag .
```

It uses the current Git `HEAD` only to construct a local diagnostic intent. It
does not verify source-release provenance, is rejected unless `--dry-run` is
also present, and can never submit a publication. It still checks the trusted
policy, origin, recipe, signer, and coordinate grant. Create and verify the
signed version tag before using the complete preflight below.

## Request publication

First perform the complete non-mutating publisher preflight:

```text
"$HARA_NATIVE" publish --tap hara --dry-run .
```

The dry run verifies the trusted tap policy, the signed `v1.2.3` source tag,
the origin repository, recipe digest, detached publisher signature, and the
coordinate grant. It does not contact the publication endpoint.

When the dry run succeeds and the release is authorized, request publication:

```text
"$HARA_NATIVE" publish --tap hara .
```

The CLI posts the signed canonical intent to
`https://packages.hara-lang.org/v1/publications`. Record the returned request
or release identifier. Treat it as a request—not as a published package—until
the registry has rebuilt the archive from the signed tag, validated the recipe,
and returned its attestation.

## Verify the published result

Use the registry record/attestation to obtain the final archive and its
publisher and registry signatures. In a fresh package store, verify the exact
archive before running it:

```text
HARA_DIST_HOME="$(mktemp -d)" /path/to/hara-native bundle verify downloaded.harp
HARA_DIST_HOME="$(mktemp -d)" /path/to/hara-native bundle run downloaded.harp --entry acme.widgets.main/main
```

Compare the final coordinate, version, signed source tag/commit, recipe digest,
archive SHA-256, publisher key id, and registry attestation to the release
request. A failed rebuild, missing grant, changed tag, or digest mismatch is a
failed publication: publish a new version after correction rather than
overwriting the requested release.

## Current limitations

This checkout’s checked-in smoke fixtures are deliberately local; only
`smoke-answer` contains the minimal typed recipe needed to demonstrate a local
publication preflight. Use the guide when creating the separate package
repository rather than treating their `0.1.0` identities as a release plan.
In particular, `publish --dry-run examples/smoke-answer` is not a smoke-test
command: publication preflight intentionally requires the enclosing source
repository to have a valid signed `v0.1.0` tag before it reaches the recipe and
grant checks. Use `make test-examples` to test the checked-in fixtures.
`--skip-signed-tag` can inspect the later local checks in a real untagged
repository, but it does not make a smoke fixture releasable or bypass its
missing publication grant.
Do not bypass the identity and registry protocol by uploading a local archive:
`hara-native publish` always sends a signed source-package request, and the
registry remains the authority that deploys the immutable release.
