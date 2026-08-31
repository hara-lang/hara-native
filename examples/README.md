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

`smoke-answer` is the first end-to-end publication fixture. Its
`hara-native/smoke-answer` coordinate is protected, so the `0.1.0` release
waits for its reviewed root-policy grant and signed source tag. The same steps
also apply to a new source-package repository.

The native flow has five boundaries:

1. Create a local signing key.
2. Let `publish` prove the public key through browser sign-in.
3. Wait for the scoped identity-policy grant when review is required.
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
export HARA_OFFICIAL_ROOT_SHA256="sha256:b8733a0627b8d0063974b6b2a01721da76ee24e1f069a32e5e2cecb522f5ec40"
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

### Sign in, prove the key, and request a scope

Run the normal publish command. If the policy has no matching grant,
`hara-native` creates a device request, signs its fresh challenge with the
local key, prints a browser URL, and polls until GitHub confirmation finishes:

```text
"$HARA_NATIVE" publish --tap hara .
```

For `hara:YOUR_GITHUB_OWNER/*`, the request is automatically approved for root
signing. Protected namespaces, including `hara:hara-native/*`, create a GitHub
review issue. Identity turns the verified broker issue into the signed,
exact-coordinate policy PR automatically. After its one policy-owner approval
merges the PR, rerun the exact same command; see
[PUBLISHING.md](../PUBLISHING.md#policy-automation-and-recovery).

### Obtain permission for a real package

Before publishing, a policy maintainer must grant the exact coordinate to the
enrolled key id. For example, a project with this manifest fragment:

```clojure
{:project/id acme/widgets
 :project/version "1.2.3"
 :project/recipe "project.receipe.edn"}
```

has the official coordinate `hara:acme/widgets`. Ask the maintainer to grant
`$HARA_SIGNER_KEY_ID` permission for that exact coordinate. A key enrolled for
one package cannot publish another package unless its policy grant says so.

The project also needs `project.receipe.edn` with a typed, reproducible recipe and
a signed Git tag named exactly `1.2.3`. The full recipe and tag requirements
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
| `publish` prints a browser URL | Open it in the GitHub account that owns the requested namespace and confirm the exact displayed key fingerprint and coordinate. |
| `publish` reports a pending root-policy review | Open the printed issue URL; after the signed identity-policy PR merges, rerun the same command. |
| `identity policy does not authorize publisher key` | The policy revision still lacks the exact coordinate or namespace grant. Do not bypass it or upload an archive manually. |
| `publish --dry-run` reports that the version tag is missing or unsigned | Create and verify a clean signed tag named `1.2.3` (or the declared `:project/release-tag`). `--skip-signed-tag` is diagnostic only. |

For the complete operational reference, including recipe, signed-tag, and
published-artifact verification requirements, see [PUBLISHING.md](../PUBLISHING.md).
