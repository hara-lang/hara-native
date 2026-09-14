# Synchronous callback suspension

The native entry of a compiled ordinary Hara function must return the completed
function result, not the Promise used internally to resume its suspended frame.
This matters when a function is called through native higher-order operations or
from another compiled source unit. The native callback previously converted a
`VmOutcome::Suspended` to a Promise and returned it as ordinary data. Numeric or
collection operations then consumed that Promise, and delayed exceptions escaped
the caller's catch boundary before the callee's finally block completed.

The host synchronous callback now resumes suspended frames until they return or
fail. The separate suspension-aware fiber callback is unchanged. Explicitly
returned Promises and functions marked async still return Promises without being
eagerly awaited. The change is guarded to non-Wasm hosts; it does not claim a
Wasm synchronous-wait implementation or alter coroutine yield behavior.

Three `native-lang` regression tests cover cross-unit callback resumption,
explicit and async pending-Promise returns, and delayed rejection with finally
cleanup. Before the fix, two tests fail: the numeric caller receives a Promise,
and the error case returns `[<promise> 0]` instead of `[:caught 1]`. With the fix,
all three pass. The native-lang (48), native-command (2), and native-test-registry
(6) suites pass with permission for their local socket fixtures. The initial
sandboxed native-lang run had four socket permission failures; these are not
semantic test failures.

This resolves the synchronous suspension boundary only. It does not make
`Promise/run` concurrent, change zero-delay scheduling, or establish Foundation
`pmap` parity. Management parallel dispatch must still satisfy its independent
overlap regression before being installed.

## Cooperative peer progress

The follow-up dispatch regression exposed two independent scheduler issues.
First, a global progress guard prevented a nested wait from advancing any peer.
Progress now guards active Promise identities individually, using weak references
and a scoped stack that unwinds after callback failure. It still prevents the
active Promise from being re-entered. Second, `Promise/all` polled the first
already-ready callback before registering later peers. It now registers the full
batch before polling, so a callback may wait for a peer even when every timer is
already due. Registration is internal; no Hara API was added.

Both regressions fail before their respective fixes. Tests also verify exactly-once
execution and restoration of the progress guard after a callback panics. The
native-lang (51), native-command (2) and native-test-registry (6) suites pass.

The Hara management adapter can now dispatch bulk items with deferred Promise
callbacks and join their ordered results. Its overlap, error-isolation and ordering
assertions pass. This is cooperative overlap on the native scheduler, not a claim
of CPU-thread parallelism or a change to the eager behavior of Promise/run and
zero-delay callbacks.
