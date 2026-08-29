# Browser native host

`packages/browser` publishes `@hara-lang/native-browser`: the isolated core
Wasm runtime and verified HARP package loader. `packages/hta` publishes the
transport codec, worker lifecycle, and Node/browser provider boundary.

The browser host contains no Hara source bundle. It downloads a lock-pinned
archive from `https://packages.hara-lang.org`, checks the archive and every
declared file, and only then activates browser bytecode or an HTA provider.

Run the supported browser-host tests:

```text
npm ci --ignore-scripts
npm run test:wasm-runtime
npm run test:browser-packages
npm run test:hta
npm run test:provider-hosts
```

The release workflow builds `native-vm` and `native-full` from the Rust Wasm
library, packages both profiles in `@hara-lang/native-browser`, and emits an
artifact hash in the release manifest.
