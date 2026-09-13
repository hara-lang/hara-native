# Outbound socket receive streams

Ryuk's faithful migration requires a relay over an outbound TCP connection.
`Socket/connect` previously stored its handle only in the provider's `sockets`
map, while `events` (also used by `Socket/receive-stream`) accepted only
listeners and accepted connections. Creating the relay therefore failed with
`receive-stream failed: socket/invalid: unknown socket handle`.

The provider now accepts outbound handles for event subscriptions and runs a
reader clone that forwards peer data and EOF into the existing event queues.
Explicit close shuts down both TCP directions before dropping the writer,
settles pending reads, and removes subscriptions and callbacks. Peer EOF drops
the active transport and retains its association until owner cleanup, as with
accepted connections. The existing connect callback and send behavior remain
unchanged; no new native API is introduced.

Validation on the local Rust host:

- Two new outbound provider tests failed before the implementation with the
  exact unknown-handle error; both pass afterward.
- `cargo test --lib socket_lifecycle_tests -- --nocapture`: 5 passed.
- `cargo test --test native-lang`: 40 passed, including outbound ping/pong
  receive-stream round trips on interpreter and direct-native backends.
- `cargo build --release --bin hara-native --locked --offline`: passed.
- `cargo test --lib`: 804 passed, 2 failed. The failing distribution digest
  diagnostic and native-inventory drift checks also failed in earlier recorded
  runs; they do not exercise socket streams. The full suite is not green.
- The rebuilt host passes the staged Ryuk contract suite, including the exact
  `label=reaped=true\n` bytes observed by a real loopback listener and subsequent
  relay/socket teardown. Docker calls are isolated test doubles, not a daemon.
- The installed native basic-server suite passes 33 tests after the rebuild.

The Ryuk fixture reads the listener's already-subscribed event stream, assembling
data chunks through the newline. It does not assume a late connection-stream
subscription replays earlier bytes. The two native provider tests separately
cover outbound receive bytes, peer EOF, and closing a pending read while the peer
remains open.

This fixes a native prerequisite, not the entire runtime migration. Ryuk's
historical tests, partial-startup cleanup, and background Promise scheduling
remain pending in its migration record. No commit or publication was performed.
