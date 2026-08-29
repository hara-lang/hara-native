# Native provider hosts

This directory contains trusted host adapters only. They implement the
filesystem capability boundary for GitHub, WebDAV, S3, Google Drive, SFTP, and
IndexedDB, and they are tested without a Hara source checkout.

A provider's `project.edn`, extension declaration, compiled Wasm façade,
digest, and HARP archive belong to the Hara package repository and are
published through `packages.hara-lang.org`. At runtime this repository accepts
one only after the host has verified the archive, its declared digest, and the
selected browser/JVM/native target.

Run the host-adapter tests from `core/rust/web`:

```text
npm run test:provider-hosts
```
