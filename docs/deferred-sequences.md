# Deferred iterator sequences

`Iter/seq-deferred` accepts one iterable and returns a reusable native Seq
without pulling its first element. It reuses an existing Seq unchanged.
Unlike `Iter/seq`, an empty input remains a truthy Seq rather than nil.
Constructing the source iterator can still reject an unsupported input; item
projection and iteration failures occur only when the sequence is consumed.

The existing Seq cache owns realized values and iteration errors. Repeated
consumption does not rerun projections or failing callbacks. Dropping all views
releases the cache and unconsumed source through their existing ownership.
There is no global state or separate lazy wrapper. `Iter/seq` is unchanged.

The Foundation task migration uses this boundary around `Iter/iter-map` so
vector-package output projection is deferred until consumption. Global
Foundation map/seq behavior is not changed.

Validation: two Rust native-lang regressions fail before the constructor exists
and pass after implementation. They assert zero initial pulls, memoized reads,
truthy empty results, consumption-time cached failures, and unary arity.
The management adapter's 24 written facts also pass, including the pinned
Foundation empty-result and exception-phase contracts. JVM/browser parity for
the new constructor has not been verified.
