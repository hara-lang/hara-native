# Protocol lookup during migration emission

A one-second sample of the original Python basic-client migration diagnostic
(PID 64956, 2026-09-14) showed active native VM calls, protocol resolution, and
string/allocation work. The sample is retained locally at
`/private/tmp/hara-python-exchange-64956.sample.txt`.

`find_protocol` previously constructed both qualified and runtime names while
scanning registry entries. It now compares borrowed namespace/name slices and
the exact `/` or `.` separator. Registry order, accepted spellings, declaration
identity and absence behavior are unchanged. No cache or mutable state is added.

Validation:

- `cargo test --offline --lib protocol_lookup -- --include-ignored --nocapture`
  passes both before and after. It covers every registered short, qualified and
  runtime name, and rejects malformed prefixes, suffixes and separators.
- The explicit fixed-work timing (debug build, 7,600 lookups) measured
  50.097209 ms before and 6.954459 ms after. This is a local microbenchmark, not
  an end-to-end migration speed claim or a timing-based acceptance threshold.
- `cargo test --offline --test native-lang` passes all 45 tests with permission
  for the existing loopback fixtures.
- Scoped `git diff --check` passes.
- `cargo build --offline --release --bin hara-native` passes. The rebuilt
  launcher passes the installed schema-base (19 facts) and schema-find
  (7 facts) tests.

The already-running Python migration process uses the previous release binary;
it was not interrupted or restarted to apply this optimization. That original
process subsequently passed its six-payload exchange and registry-cleanup fact.
Its negative control and restored run are now executing serially on the rebuilt
release. Any end-to-end speed improvement remains unproven at this checkpoint.
