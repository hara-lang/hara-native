# Clipboard text boundary

The approved migration now has `OS/clipboard-copy` (one string, returning that
string) and `OS/clipboard-paste` (no arguments, returning text). Both require
native-runtime capability and process permission before accessing the host.
These methods transport text; they do not pretty-print arbitrary Hara values.
The original pointer invocation path supplies an already emitted string.

The JVM uses the original AWT clipboard mechanism. Rust uses `pbcopy`/`pbpaste`
on macOS, `wl-copy`/`wl-paste` on Wayland, or `xclip` on X11. Missing desktop
tools, command failures, and invalid UTF-8 are errors; there is no silent
in-memory fallback. Rust Windows and browser clipboard support remain
unsupported. Desktop backend availability is not established by unit tests.

No clipboard cache is retained. Copy replaces the host text and paste reads
it back. A caller temporarily changing the desktop clipboard is responsible
for preserving its previous representation; a text-only snapshot cannot
restore images or other clipboard flavors. Automated tests therefore never
read or overwrite the user's system clipboard.

Validation:

- New JVM capability test and Rust argument test fail against the previous
  implementation with unbound clipboard symbols, and pass with this change.
- JVM uses a fresh isolated AWT clipboard: exact Unicode, newline, empty text,
  replacement, and unsupported-flavor behavior are checked.
- Rust uses isolated command fixtures: exact text round trips, a 300 KB pipe
  round trip, launch failure, and unsuccessful exit are checked. That test is
  included in the active native-language test target as well as library tests.
- Rust interpreter and direct-native reject invalid arguments and deny both
  valid clipboard calls when the process provider is absent.
- `make test-rust`: 34 CLI, 34 native-language, and 6 registry tests pass.
- `mvn -q -f core/java/pom.xml -Djacoco.skip=true test`: passes with loopback
  permission for existing server fixtures.
- Release Rust binary rebuilt; the macro consumer passes 60 facts in fresh
  verification `c62d4fb6-7b2c-4a02-a0a4-fdcf29612e4a`. It still uses original
  model/runtime overlays and is not installed-only pipeline closure.

The Foundation structural migration rule now wires original `ptr-invoke` to
this boundary, preserving copy-before-input/evaluation ordering and formatting
nonstrings before copying. Pointer's 54 facts pass, but do not execute the
desktop clipboard branch. Native method calls cannot safely be mocked by
replacing their exposed Var; no such clipboard probe was performed.

Still pending: real desktop integration validation and full cross-host conformance.
