# Component queries

The approved Foundation migration adds `IComponent/info` (component, level)
and `IComponent/health` (component) to the existing lifecycle protocol owner.
No parallel component-query protocol is introduced.

Guest implementations receive the level unchanged and may return structured
information, structured health, nil, or false. The JVM default `info` returns
status and default `health` returns started state. `ResourceInstance` delegates
to its wrapped component when available. Rust exposes query-level and health
associated types; its existing session and work hosts return started state for
health, and use the status default for info. These defaults do not replace
explicit guest runtime query implementations.

The source-free regression checks exact structured results, false health,
nil information, and rejected arities. It fails on the prior implementation
because `health` is not a declared protocol method, and passes on JVM and both
Rust interpreter and direct-native execution.

Validation completed:

- `make test-rust`: 34 CLI, 32 native-language, and 6 test-registry tests pass.
- `mvn -q -f core/java/pom.xml -Djacoco.skip=true test`: passes with loopback
  permission; the active `HaraNativeLangTest` suite has 13 passing tests.
- Release Rust binary rebuilt for the language checks.
- Installed `lang.core.runtime-proxy`: 16 authored facts pass; a deliberate
  query-expectation mutation produces 14 passes and 2 failures.
- Macro consumer: 60 facts pass with original model/runtime overlays.

The proxy migration preserves the original query operands and return values.
Its 14 historical integration facts remain unexecuted. Clipboard support,
full original RuntimeDefault installation, and full cross-host conformance
are not claimed complete here.
