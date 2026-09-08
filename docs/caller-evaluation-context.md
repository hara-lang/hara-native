# Caller evaluation context for the Foundation language port

Approved scope: preserve the caller's evaluation namespace for the native
`lang.core.impl-entry` construction pipeline, independently of a function's
lexical namespace. No new HAL public primitive is introduced.

Native callable callbacks retain the outer invocation's evaluation namespace
while restoring their own captured lexical environment. `Base/current-namespace`
reports that evaluation namespace. `Runtime/eval` evaluates against its mappings;
ordinary compiled global references remain bound to their defining Vars.
Explicit `Runtime/eval-in` temporarily overrides the evaluation namespace.
The dynamic selection is scoped with a drop guard; nested calls restore it.

The compiler and evaluator also accept a single clause in the existing anonymous
multi-arity syntax, such as `(fn ([value] value))`. An empty body remains invalid.
This allows original Foundation macro templates to execute without rewriting
their arity structure.

## Evidence and remaining scope

`cargo test --test native-lang` passes all 11 tests. The context regression was
observed failing before the fix (`[103 false 103]` instead of `[103 true 12]`).
It checks lexical binding, caller lookup, an executable returned template,
single-clause, multiple and variadic arities, invalid empty bodies, explicit
namespace override, and recovery after errors. The explicit override and
single-clause regressions were each observed failing before their fixes.
`cargo test --test native-test-registry` passes all 6 tests.
The saved `lang_core_impl_entry_context.edn` migration probe now passes all
5 checks on the rebuilt debug executable (previously 2 passed, 1 failed, and
2 errored). Its source and expected values were not changed for this recheck.
The Foundation `code.migrate.lang-test` regression suite passes all 114 checks.

This is not a claim of full Foundation dynamic binding parity. Interpreter-only
call chains, symbolic modifier resolution, macro expansion context, and suspended
or interleaved callbacks need separate audits. The language source candidates
still require migration generation, source/test correspondence, and installation.
The rebuilt artifact is the debug native executable; release is not rebuilt.
