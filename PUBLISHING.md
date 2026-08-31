# Publishing Hara source packages

GitHub Packages is the source of truth for Hara packages. Hara Native builds
and verifies local HARP inputs; it does not upload packages, hold a publisher
key, contact Hara Identity, or publish directly to GHCR.

For a source repository `OWNER/REPOSITORY`, the published source image is:

```text
ghcr.io/hara-packages/OWNER.REPOSITORY:<version>
```

When that repository owns specifications, the release also contains the paired
image:

```text
ghcr.io/hara-packages/OWNER.REPOSITORY.specs:<version>
```

Both images are built from the same signed source tag. The public Packages API
serves a reviewed lock containing the archive SHA-256, OCI repository, and OCI
manifest digest, so runtime clients retain their existing HTTP transport and do
not need direct registry credentials.

## Source repository responsibilities

1. Declare an immutable version and a typed `:project/recipe` in `project.edn`.
   The recipe must use an approved adapter and must not contain shell commands
   or scripts.
2. Run the matching Hara Native validation locally:

   ```text
   hara-native test --project .
   hara-native bundle build . --output /tmp/source.harp
   hara-native bundle verify /tmp/source.harp
   hara-native bundle build spec --output /tmp/specs.harp # when the project has spec/
   hara-native bundle verify /tmp/specs.harp
   ```

3. Push a reviewed commit and an immutable signed version tag. The tag starts
   the repository's publication-request workflow.
4. That workflow verifies the tag, records the exact project, recipe, native,
   and optional specs-tree revisions, signs a receipt with GitHub OIDC, and
   opens or updates a receipt pull request in `hara-lang/hara-packages`.

Source repositories never receive a GHCR publish token. `hara-native publish`
and the legacy `package publish` kernel route fail closed with
`package/publication-github-workflow-required`.

## Central publication

Only the protected `hara-lang/hara-packages` workflow may publish. It verifies
the receipt and signature, checks the signed tag and declared files, builds
with the recorded Hara Native revision, verifies both HARP archives, publishes
immutable version and digest tags to the `hara-packages` GitHub organization,
makes the images public, and reads their manifests back from GHCR.

GitHub repository access, protected environments, and the merged receipt are
the release authorization boundary. Hara Identity remains independent of this
package flow; no Identity grant, detached publisher key, registry intake POST,
or object-store upload is involved.

## Consumer verification

Resolve through `https://packages.hara-lang.org/v1/registry?ref=main`. The
registry response is cacheable but all locks pin both the HARP digest and OCI
manifest. Downloaded bytes are re-hashed before installation. Historical
registry-commit pins are intentionally retired rather than silently resolving
to mutable historical state.
