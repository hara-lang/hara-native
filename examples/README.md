# Hara Native smoke-test walkthrough

This first-run guide builds, verifies, and runs the two small source packages
in this directory using **only `hara-native`**. No second CLI, wrapper, or
environment variable is required. The final command for each fixture prints
`42`.

## What the smoke tests prove

| Fixture | What it proves |
| --- | --- |
| `smoke-answer` | `hara-native` can package a source project, verify the HARP archive, and run its declared entry point. |
| `smoke-require` | The packaged entry point can require and call a sibling project namespace. |

These are local smoke packages, not registry releases. They use portable
arithmetic only and require no providers or Foundation package.

## What you need

You need this checkout and a Rust toolchain that provides `cargo`. Run every
command below from the root of the checkout:

```text
cd /path/to/hara-native
```

## 1. Create the `hara-native` executable

Build the optimized executable once:

```text
cargo build --release --manifest-path core/rust/Cargo.toml --bin hara-native
```

Cargo writes the executable to:

```text
core/rust/target/release/hara-native
```

Set a short variable for it, then confirm that it runs:

```text
export HARA_NATIVE="$PWD/core/rust/target/release/hara-native"
test -x "$HARA_NATIVE"
"$HARA_NATIVE" --version
```

`test -x` prints nothing when the path is executable. Re-run the release build
after changing native Rust code. The same executable owns local archive builds,
verification, execution, signing, identity enrollment, and publication; there
is no separate signer, packager, or publisher binary.

## 2. Run the smallest smoke package, step by step

`smoke-answer` has one entry namespace. Its `main` function evaluates
`(+ 19 23)`, so the expected result is `42`.

First build a HARP archive from the project directory:

```text
mkdir -p target/smoke-walkthrough
"$HARA_NATIVE" bundle build examples/smoke-answer --output target/smoke-walkthrough/smoke-answer.harp
```

Create an isolated package store. It prevents the walkthrough from reading or
altering packages installed elsewhere on your machine:

```text
export HARA_DIST_HOME="$(mktemp -d)"
```

Verify the archive, then run the entry point declared by the project:

```text
"$HARA_NATIVE" bundle verify target/smoke-walkthrough/smoke-answer.harp
"$HARA_NATIVE" bundle run target/smoke-walkthrough/smoke-answer.harp --entry hara-native.smoke.answer.main/main
```

The final command must print:

```text
42
```

The operations are deliberately separate:

1. `bundle build` makes a deterministic HARP archive from the project files.
2. `bundle verify` checks archive paths, digests, and package metadata before
   it can be trusted.
3. `bundle run` installs the verified archive in the isolated store, mounts its
   project namespaces, and calls the requested entry point.

Remove the temporary store when you finish:

```text
rm -rf "$HARA_DIST_HOME"
unset HARA_DIST_HOME
```

## 3. Run the project-local `require` package

`smoke-require` has a `math` namespace and an entry point that calls
`(math/advance 41)`. Build and run it exactly as before, changing the project
path, archive name, and entry point:

```text
mkdir -p target/smoke-walkthrough
"$HARA_NATIVE" bundle build examples/smoke-require --output target/smoke-walkthrough/smoke-require.harp
export HARA_DIST_HOME="$(mktemp -d)"
"$HARA_NATIVE" bundle verify target/smoke-walkthrough/smoke-require.harp
"$HARA_NATIVE" bundle run target/smoke-walkthrough/smoke-require.harp --entry hara-native.smoke.require.main/main
rm -rf "$HARA_DIST_HOME"
unset HARA_DIST_HOME
```

It also prints `42`. Successful execution proves the archive preserves the
project-local `require`; it does not make unrelated namespaces ambient in the
generic host.

## 4. Run the Make shortcuts

After following one example manually, these commands run the same native-only
smoke flow with fresh stores and automatic cleanup:

```text
make test-example-answer
make test-example-require
make test-examples
```

The fixtures stay outside `make test` because they are user-facing source
package smoke inputs. The normal host validation remains source-free.

## 5. Follow the publishing path with `hara-native`

Do **not** submit either checked-in smoke fixture. Their identifiers and
`0.1.0` versions are local examples, and a registry release is immutable.
`smoke-answer` includes a minimal typed recipe so it can exercise local
publication preflight, but it has no signed release tag or publication grant.
Use the following steps in a real source-package repository after replacing the
sample owner, package id, version, and paths.

The native flow has five boundaries:

1. Create a local signing key.
2. Enroll its public key with the identity service.
3. Obtain an explicit grant for one package coordinate.
4. Preflight the signed, tagged source release.
5. Submit the publication request; the registry rebuilds and deploys it.

### Create a development signing key

The native executable stores a development Ed25519 seed in a new Unix `0600`
file. The seed is private: do not commit, share, print, or put it in a CI
variable. This bundled key store is for a personal development key only; do
not use it for a team or production key until a reviewed key-management backend
is available.

Choose an owner name and a stable key id before enrollment. The key id is the
name that the registry policy later grants permission to use:

```text
export PUBLISH_OWNER="YOUR_GITHUB_OWNER"
install -d -m 700 "$HOME/.local/state/hara"
export HARA_SIGNER_KEY_FILE="$HOME/.local/state/hara/publisher.ed25519"
export HARA_SIGNER_KEY_ID="$PUBLISH_OWNER-2026-01"
export HARA_OFFICIAL_ROOT_SHA256="sha256:8861d398c14a53b2fe13f7736310bb2c55624260c84e131452457e8aa69ac3dc"
"$HARA_NATIVE" signer generate --key-file "$HARA_SIGNER_KEY_FILE"
"$HARA_NATIVE" signer public-key --key-file "$HARA_SIGNER_KEY_FILE"
```

`signer generate` refuses to overwrite an existing key and prints only the
public key. The output from `public-key` is 64 lowercase hexadecimal characters;
it is safe to give that public value to an identity-policy maintainer.

You can prove that the key signs an intent without contacting a service:

```text
printf '%s' '{:intent/format "0.0.0-alpha"}' | "$HARA_NATIVE" signer sign
```

The response contains the configured `:key/id` and an Ed25519 signature, never
the seed.

The root fingerprint is public, not private-key material. It pins the official
identity policy root so `publish` can verify the policy it fetches from the
official tap.

### Preview and perform identity enrollment

`id enroll` derives the public key from `HARA_SIGNER_KEY_FILE`, prepares a
canonical enrollment record, and signs it in the native process. It sends only
the public key, key id, owner, server challenge, and detached signature. The
private seed remains in the local file.

This preview is fully local because it supplies an intentionally fake challenge
and uses `--dry-run`. It lets you inspect the exact request shape without
changing an account:

```text
"$HARA_NATIVE" id enroll --tap hara --owner "$PUBLISH_OWNER" --challenge preview-only --dry-run
```

When you are ready to enroll for real, authenticate in the browser URL printed
by `id login`, then run the two commands below. `id enroll` fetches a real
one-time challenge and posts the signed enrollment, so it changes identity
service state:

```text
"$HARA_NATIVE" id login
"$HARA_NATIVE" id enroll --tap hara --owner "$PUBLISH_OWNER"
"$HARA_NATIVE" id status
```

Successful enrollment proves that you control the key. It does **not** authorize
that key to publish every package.

### Obtain permission for a real package

Before publishing, a policy maintainer must grant the exact coordinate to the
enrolled key id. For example, a project with this manifest fragment:

```clojure
{:project/id acme/widgets
 :project/version "1.2.3"
 :project/recipe "hara.recipe.edn"}
```

has the official coordinate `hara:acme/widgets`. Ask the maintainer to grant
`$HARA_SIGNER_KEY_ID` permission for that exact coordinate. A key enrolled for
one package cannot publish another package unless its policy grant says so.

The project also needs `hara.recipe.edn` with a typed, reproducible recipe and
a signed Git tag named exactly `v1.2.3`. The full recipe and tag requirements
are in [PUBLISHING.md](../PUBLISHING.md).

### Preflight, submit, and observe deployment

From the real project root, first run the non-submitting preflight:

```text
"$HARA_NATIVE" publish --tap hara --dry-run .
```

It fetches and verifies the registry policy, checks the signed source tag,
origin, recipe, publisher signature, and coordinate grant. It builds the local
package as part of that check, but does **not** call the publication endpoint.

Only after that succeeds and the release is approved, submit the signed
publication request:

```text
"$HARA_NATIVE" publish --tap hara .
```

This does not deploy directly from your computer. It submits a signed request
to the registry. The protected registry rebuilds the archive from the signed
tag, verifies the result, records the immutable release, writes its attestation,
and then deploys the verified package. Record the registry response and verify
the published archive/attestation before considering the release deployed.

## Common first-run problems

| Symptom | What to check |
| --- | --- |
| `cargo` is missing | Install a Rust toolchain, then repeat the release build step. |
| `HARA_NATIVE` cannot be executed | Run the release build from the repository root and set `HARA_NATIVE` to the exact path above. |
| `bundle build` cannot read `project.edn` | Run it from the repository root and pass the project directory, such as `examples/smoke-answer`. |
| `bundle run` cannot find the entry | Copy the exact `--entry` value from the walkthrough or the example project's `project.edn`. |
| You expect a `.hal` file to execute as a loose script | Build the project first, then run the verified `.harp` archive. |
| `id enroll` reports an unknown key, owner, or challenge | Run `id login`, complete browser authentication, then retry enrollment with the same key id. |
| `publish --dry-run` reports a missing grant | Ask the policy maintainer to grant your enrolled key id the exact package coordinate. |
| `identity policy does not authorize publisher key` | Enrollment succeeded, but no policy grant exists for that key id and coordinate. Request the exact grant from the policy maintainer; do not retry publication until it is recorded in the signed policy. |
| `publish --dry-run` reports that `v…` is missing or unsigned | A real package repository needs a clean, signed tag named for its project version. Before creating it, `publish --dry-run --skip-signed-tag` can inspect the remaining local checks, but it can never submit a release. Do not use a checked-in smoke fixture as a publication preflight; run `make test-examples` instead. |

For the complete operational reference, including recipe, signed-tag, and
published-artifact verification requirements, see [PUBLISHING.md](../PUBLISHING.md).
