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

Prepare a real source-package repository. `examples/smoke-answer` is the first
end-to-end publication fixture: its protected `hara-native` namespace still
requires an approved root-policy grant before its immutable `0.1.0` release can
be requested.

The project needs an immutable package version and an official-tap coordinate.
A project id without a tap prefix is normalized to the `hara` tap, so this:

```clojure
{:project/id acme/widgets
 :project/version "1.2.3"
 :project/recipe "project.receipe.edn"}
```

publishes the canonical coordinate `hara:acme/widgets`. An alternate tap must
be explicit (for example, `partner:acme/widgets`) and selected with the matching
`--tap` value.

Official publication also requires `project.receipe.edn`. It describes a
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

The private key stays outside the repository. A normal `hara-native publish`
now derives the public key, starts a browser device flow, and signs the fresh
server challenge automatically when a key is not yet granted. It sends only
the public key, key id, coordinate, canonical intent, and detached proof; it
never sends the private key.

The compatibility commands remain available for diagnosing an identity service,
but a new publisher normally starts at `publish`:

```text
"$HARA_NATIVE" publish --tap hara .
```

For an owner namespace such as `hara:YOUR_GITHUB_OWNER/*`, Identity creates an
automatically approved grant request. Protected namespaces such as
`hara:hara-native/*` create a review issue. In both cases the offline identity
root signer must produce and merge a valid signed policy PR before the command
can continue. Re-run the same `publish` command after that merge.

### Finalize a reviewed grant offline

The browser flow never receives the identity root private key and it never
changes policy. After the review issue is approved, a policy maintainer works
from an offline checkout of `hara-identity` and uses the same `hara-native`
executable to make the narrowly scoped change. This command refuses relative
paths, a root key that does not match `:identity/root-key`, a conflicting key
id, or a changed authorization-service key.

```text
export HARA_IDENTITY_ROOT_KEY_FILE="/offline/keys/hara-identity-root.ed25519"
export HARA_PUBLISH_AUTHORIZATION_PUBLIC_KEY="<64 lowercase hex characters>"

"$HARA_NATIVE" id policy grant \
  --identity "$PWD/identity.edn" \
  --root-key-file "$HARA_IDENTITY_ROOT_KEY_FILE" \
  --key-id "hoebat-2026-01" \
  --public-key "96369cd1bb1ea0221511ff5f2b824bd7e4617efe3d91b5809bf16029e95facfb" \
  --github-subject "YOUR_NUMERIC_GITHUB_ID" \
  --coordinate "hara:hara-native/smoke-answer" \
  --authorization-public-key "$HARA_PUBLISH_AUTHORIZATION_PUBLIC_KEY"
```

Use `--dry-run` first to inspect the complete replacement policy and detached
signature without writing either file. The normal command writes only
`identity.edn` and its matching `identity.edn.sig`; review those two files,
commit them on a policy branch, and open the protected policy PR through the
repository's normal GitHub workflow. The public authorization key must match
the private Ed25519 key configured only on the Identity service as
`HARA_PUBLISH_AUTHORIZATION_PRIVATE_KEY`; it is not the publisher key and must
not be put in a source repository.

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

The publisher binds a release to a Git tag named exactly the package version
(for example, `1.2.3`). `:project/release-tag` can override that default for a
monorepo. Ensure the worktree is clean, push the source commit, create a signed
tag, and verify it locally and on the remote before requesting publication:

```text
git status --short
git push origin main
git tag -s 1.2.3 -m "acme/widgets 1.2.3"
git verify-tag 1.2.3
git push origin 1.2.3
git ls-remote --tags origin refs/tags/1.2.3
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

The dry run verifies the trusted tap policy, the signed `1.2.3` source tag,
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

`smoke-answer` contains the minimal typed recipe used for the first protected
namespace rollout. `publish --dry-run examples/smoke-answer` intentionally
requires the enclosing source repository to have a valid signed `0.1.0` tag
before it reaches recipe and grant checks. Use `make test-examples` for local
fixture verification. `--skip-signed-tag` can inspect later local checks in an
untagged repository, but it never makes a request releasable.
Do not bypass the identity and registry protocol by uploading a local archive:
`hara-native publish` always sends a signed source-package request, and the
registry remains the authority that deploys the immutable release.
