# Method-level protocol support

`(Base/supports-method? protocol 'method receiver)` returns whether normal
protocol dispatch can select an implementation for that receiver. It does not
invoke the method. The method name must be an unqualified symbol; unknown
method names and unsupported receivers return false. Invalid arguments throw.
Like Base/satisfies?, the protocol can be supplied directly or through a Var.

This is distinct from `Base/satisfies?`, which continues to require the entire
protocol. A record implementing only IComponent/stop can therefore report true
for method support and false for whole-protocol satisfaction. Callers may use
method support to implement an explicit fallback for absent methods; failures
from an available method must still propagate.

Rust checks the same extension, named guest type, and native receiver paths as
ProtocolRegistry::invoke. Java uses HaraProtocol::implementation, the normal
dispatch resolver. Neither path calls the selected implementation to inspect it.

The same change makes Rust bytecode rest destructuring materialize a vector,
matching Java and the Rust interpreter. Reading or counting rest arguments no
longer consumes them. Nested rest bindings retain nil, false, and trailing
values; generated intrinsic calls remain protected from local shadowing.

Validation performed locally:

- Rust destructuring regression failed before the fix, then all four
  destructuring tests passed. The previous iterator-valued shadowing expectation
  was updated to the exact reusable vector result.
- The native-lang integration suite passed all 36 tests, serially, with loopback
  access. Its first sandboxed run had 35 passes and a denied local socket bind.
- Java method-support and rest-parity tests passed, two tests total. Maven used
  `-Djacoco.skip=true` because the installed coverage agent rejected the JDK's
  class-file version; no project coverage configuration was changed.
- The release native host was rebuilt. The staged Foundation script-def suite
  passed seven facts and annex passed nineteen, including stop-error propagation
  and state preservation. These are focused migration results, not completion
  of the complete lang.* migration or historical annex reconciliation.
