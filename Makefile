SHELL := /usr/bin/env bash

.DEFAULT_GOAL := help

.PHONY: help test \
	test-boundary test-rust test-raw test-jvm test-native-benchmark benchmark-native \
	test-conformance test-conformance-full test-conformance-rust test-conformance-jvm test-conformance-browser \
	mirror-conformance check-conformance-mirror \
	web-install test-browser-integrity test-provider-hosts \
	build-browser-profiles browser-playwright-install test-browser-sdk test-browser-playwright \
	test-examples test-example-answer test-example-require build-signer test-signer test-publication

# hara-native builds, verifies, and runs the checked-in smoke projects. Each
# host run receives a fresh package store and cleans it at recipe exit.
SMOKE_TARGET ?= target/smoke-examples

help:
	@printf '%s\n' \
	  'Native host validation layers:' \
	  '  make test-boundary             source-free repository and build-input gate' \
	  '  make test-rust                 generic Rust hara-native CLI' \
	  '  make test-native-benchmark     benchmark coordinator and worker unit tests' \
	  '  make benchmark-native PROFILE=smoke|guard|standard   build isolated tier workers and record evidence' \
	  '  make test-raw                  raw Wasm host boundary' \
	  '  make test-jvm                  JVM CLI, HARP, and prebuilt-provider loader' \
	  '  make test-conformance          serial native/protocol and language Rust/JVM/browser conformance' \
	  '  make test-conformance-full     portable HNC1 conformance plus trusted provider profiles' \
	  '  make mirror-conformance HNC_MIRROR=/path   write a registry HNC source mirror' \
	  '  make check-conformance-mirror HNC_MIRROR=/path   validate a registry HNC source mirror' \
	  '  make test-browser-integrity    Node HARP, package, and HTA verification' \
	  '  make test-provider-hosts       trusted provider-host adapters' \
	  '  make test-browser-sdk          generated native-vm/native-full SDK' \
	  '  make test-browser-playwright   Chromium smoke against generated profiles' \
	  '  make test-examples          native-only source-package smoke fixtures' \
	  '  make test-example-answer    answer fixture only' \
	  '  make test-example-require   require fixture only' \
	  '  make build-signer              build hara-native with its integrated development signer' \
	  '  make test-signer               test the integrated signer and legacy protocol response' \
	  '  make test-publication          test native id-enrollment and publish-command wiring' \
	  '  make test                      every layer, serially'

test:
	+$(MAKE) test-boundary
	+$(MAKE) test-conformance
	+$(MAKE) test-rust
	+$(MAKE) test-raw
	+$(MAKE) test-jvm
	+$(MAKE) test-browser-integrity
	+$(MAKE) test-provider-hosts
	+$(MAKE) test-browser-sdk
	+$(MAKE) test-browser-playwright

test-boundary:
	@test -z "$$(find . -path './examples' -prune -o -type f -name '*.hal' -print -quit)"
	@test -z "$$(find providers -type f \( -name 'project.edn' -o -name 'extension.edn' -o -name 'provider.sha256' \) -print -quit)"
	@! rg -n 'HARA_SOURCE_ROOT|hal-src|std\.foundation\.hbx|cli\.hbx|core/lib' core/rust/Cargo.toml core/rust/build.rs core/rust/crates core/rust/src core/java/pom.xml core/java/src/main/resources --glob '!**/*test*' --glob '!tests.rs' --glob '!execution_tests.rs' --glob '!differential_tests.rs'
	@! rg -n 'std\.foundation/(assoc|get|first|rest|map|reduce|conj)|std\.foundation\.coroutine/(await|yield)' core/rust/src core/rust/tests core/java/src/test core/rust/web --glob '*.rs' --glob '*.java' --glob '*.mjs' --glob '*.js' --glob '!node_modules/**'
	@! rg -n 'std\.foundation(?:\.|/)|hara-specs-registry|HARA_SPECS_REGISTRY' core/rust/specs/native-protocol-v1.edn core/rust/specs/language-v1.edn core/java/src/test/java/hara/truffle/bytecode/HbcCodecTest.java core/rust/web/native-protocol-conformance.test.mjs core/rust/web/language-conformance.test.mjs

# These targets remain outside `test` as user-facing source-package smoke
# fixtures. Each host run receives a fresh store and cleans it at recipe exit.
test-examples: test-example-answer test-example-require

test-example-answer:
	@mkdir -p "$(SMOKE_TARGET)"
	cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle build examples/smoke-answer --output "$(SMOKE_TARGET)/smoke-answer.harp"
	@store="$$(mktemp -d)"; trap 'rm -rf -- "$$store"' EXIT; \
	  HARA_DIST_HOME="$$store" cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle verify "$(SMOKE_TARGET)/smoke-answer.harp" && \
	  HARA_DIST_HOME="$$store" cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle run "$(SMOKE_TARGET)/smoke-answer.harp" --entry hara-native.smoke.answer.main/main

test-example-require:
	@mkdir -p "$(SMOKE_TARGET)"
	cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle build examples/smoke-require --output "$(SMOKE_TARGET)/smoke-require.harp"
	@store="$$(mktemp -d)"; trap 'rm -rf -- "$$store"' EXIT; \
	  HARA_DIST_HOME="$$store" cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle verify "$(SMOKE_TARGET)/smoke-require.harp" && \
	  HARA_DIST_HOME="$$store" cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native -- bundle run "$(SMOKE_TARGET)/smoke-require.harp" --entry hara-native.smoke.require.main/main

test-rust:
	cargo check --manifest-path core/rust/Cargo.toml --bin hara-native
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native
	cargo test --manifest-path core/rust/Cargo.toml --test native-lang
	cargo test --manifest-path core/rust/Cargo.toml --test native-test-registry

build-signer:
	cargo build --release --manifest-path core/rust/Cargo.toml --bin hara-native

test-signer:
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native signer::

test-publication:
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native parses_source_and_bundle_commands
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native rejects_unknown_publish_options
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native accepts_a_signed_tag_skip_for_a_local_publish_dry_run
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native signed_tag_skip_uses_head_only_for_a_local_preflight
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native enrollment_dry_run_signs_the_exact_canonical_request_in_process

# Each tier receives its own Cargo target directory. This keeps feature-built
# worker identities distinct and allows the coordinator to record their exact
# binary digests in same-run evidence.
PROFILE ?= smoke
BENCH_TARGET ?= core/rust/target/native-benchmark
BENCH_OUTPUT ?= $(BENCH_TARGET)/evidence-$(PROFILE).json

test-native-benchmark:
	cargo test --locked --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark
	cargo test --locked --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark-worker
	cargo check --locked --manifest-path core/rust/Cargo.toml --no-default-features --features whole-wasm --bin hara-native-benchmark-worker

benchmark-native:
	@mkdir -p $(BENCH_TARGET)
	CARGO_TARGET_DIR=$(BENCH_TARGET)/coordinator cargo build --locked --release --no-default-features --features bytecode-vm --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark
	CARGO_TARGET_DIR=$(BENCH_TARGET)/vm cargo build --locked --release --no-default-features --features bytecode-vm --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark-worker
	CARGO_TARGET_DIR=$(BENCH_TARGET)/trace-checked cargo build --locked --release --no-default-features --features tracing-jit --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark-worker
	CARGO_TARGET_DIR=$(BENCH_TARGET)/trace-native cargo build --locked --release --no-default-features --features tracing-jit,native-jit --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark-worker
	CARGO_TARGET_DIR=$(BENCH_TARGET)/whole-wasm cargo build --locked --release --no-default-features --features whole-wasm --manifest-path core/rust/Cargo.toml --bin hara-native-benchmark-worker
	$(BENCH_TARGET)/coordinator/release/hara-native-benchmark run --profile $(PROFILE) --corpus core/rust/assets/native-benchmark-v1.json --rules core/rust/assets/native-benchmark-rules-v1.json --output $(BENCH_OUTPUT) --vm $(BENCH_TARGET)/vm/release/hara-native-benchmark-worker --trace-checked $(BENCH_TARGET)/trace-checked/release/hara-native-benchmark-worker --trace-native $(BENCH_TARGET)/trace-native/release/hara-native-benchmark-worker --whole-wasm $(BENCH_TARGET)/whole-wasm/release/hara-native-benchmark-worker
	$(BENCH_TARGET)/coordinator/release/hara-native-benchmark validate --evidence $(BENCH_OUTPUT) --rules core/rust/assets/native-benchmark-rules-v1.json

test-raw:
	cargo test --manifest-path core/rust/crates/raw/Cargo.toml

test-jvm:
	mvn -q -f core/java/pom.xml -Djacoco.skip=true test
	mvn -q -f core/java/pom.xml -Djacoco.skip=true -DskipTests package

# Native-owned portable semantics. The HNC1 ABI artifact and HLC1 functional
# language artifact are generated from local declarative EDN specifications.
test-conformance:
	+$(MAKE) test-conformance-rust
	+$(MAKE) test-conformance-jvm
	+$(MAKE) test-conformance-browser

test-conformance-full:
	+$(MAKE) test-conformance
	+$(MAKE) test-provider-hosts

test-conformance-rust:
	cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- check
	cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native-language-conformance-artifact -- check
	cargo test --manifest-path core/rust/crates/runtime/Cargo.toml --features code-vm-conformance vm::conformance::tests
	cargo test --manifest-path core/rust/crates/runtime/Cargo.toml --features code-vm-conformance language_conformance_tests

test-conformance-jvm:
	mvn -q -f core/java/pom.xml -Djacoco.skip=true '-Dtest=HbcCodecTest#executesNativeProtocolConformanceArtifactSerially+executesLanguageConformanceArtifactSerially' test

test-conformance-browser: build-browser-profiles
	cd core/rust/web && npm run test:native-protocol-conformance
	cd core/rust/web && npm run test:language-conformance

mirror-conformance:
	@test -n "$(HNC_MIRROR)"
	cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- mirror "$(HNC_MIRROR)"

check-conformance-mirror:
	@test -n "$(HNC_MIRROR)"
	cargo run --quiet --manifest-path core/rust/Cargo.toml --bin hara-native-conformance-artifact -- check-mirror "$(HNC_MIRROR)"

web-install:
	cd core/rust/web && npm ci --ignore-scripts

test-browser-integrity: web-install
	cd core/rust/web && npm run test:wasm-runtime
	cd core/rust/web && npm run test:browser-packages
	cd core/rust/web && npm run test:hta

test-provider-hosts: web-install
	cd core/rust/web && npm run test:provider-hosts

build-browser-profiles: web-install
	cd core/rust/web && npm run build:browser:profiles

browser-playwright-install: web-install
	cd core/rust/web && npx playwright install chromium chromium-headless-shell

test-browser-sdk: build-browser-profiles
	cd core/rust/web && npm run test:browser-sdk

test-browser-playwright: build-browser-profiles browser-playwright-install
	cd core/rust/web && npm run test:playwright-native
