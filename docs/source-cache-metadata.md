# Source-cache metadata fidelity

The runtime-override migration exposed two distinct load boundaries:

1. Interpreter source loading predeclared `defonce` Vars as nil. A macro that
   initializes only absent Vars then skipped the initializer and its metadata.
   `defonce` is no longer synthetically predeclared.
2. Direct-native source caching encoded quoted constants with HTA, whose data
   contract omits metadata. A quoted dynamic definition consequently lost its
   dynamic marker on the next process's cache hit. Cold-only tests missed this.

The source cache now encodes and decodes a proposed entry and compares each
constant's complete source-form representation. Only exact round trips are
cached. Metadata-bearing syntax that HTA cannot retain is recompiled on later
loads; it is never silently substituted with a different program. Constants
that cannot be compared as source forms also fail closed. This can reduce cache
coverage; it does not change the HTA data format or make process-local metadata
portable.

The cache-key domain is advanced from `hara-direct-native-source-v1` to `v2`,
so entries written under the lossy policy are not reused. Existing cache files
are left intact. This is separate from the HBC artifact-format version.

Validation:

- The cache-enabled native regression reproduced `Var has no dynamic binding`
  on the second load before the cache correction.
- It now passes interpreter and direct-native cold/warm runs, preserving the
  initializer, dynamic scope, and no-reinitialization behavior.
- Three source-cache unit tests pass: ordinary reuse, distribution fallback,
  and rejection of lossy quoted metadata.
- `make test-rust` passes 34 CLI, 35 native-language, and 6 registry tests.
- The rebuilt release runtime passes the actual imported Hara override test
  twice in fresh processes, and all 47 pointer facts with the installed source.

This fixes source-cache admission, not every possible metadata-bearing HBC
publication path. Full cross-host conformance remains outside these results.
