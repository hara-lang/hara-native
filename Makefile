SHELL := /usr/bin/env bash

.DEFAULT_GOAL := help

.PHONY: help test \
	test-boundary test-rust test-raw test-jvm \
	test-conformance test-conformance-full test-conformance-rust test-conformance-jvm test-conformance-browser \
	mirror-conformance check-conformance-mirror \
	web-install test-browser-integrity test-provider-hosts \
	build-browser-profiles browser-playwright-install test-browser-sdk test-browser-playwright

help:
	@printf '%s\n' \
	  'Native host validation layers:' \
	  '  make test-boundary             source-free repository and build-input gate' \
	  '  make test-rust                 generic Rust hara-native CLI' \
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
	@test -z "$$(find . -type f -name '*.hal' -print -quit)"
	@test -z "$$(find providers -type f \( -name 'project.edn' -o -name 'extension.edn' -o -name 'provider.sha256' \) -print -quit)"
	@! rg -n 'HARA_SOURCE_ROOT|hal-src|std\.foundation\.hbx|cli\.hbx|core/lib' core/rust/Cargo.toml core/rust/build.rs core/rust/crates core/rust/src core/java/pom.xml core/java/src/main/resources --glob '!**/*test*' --glob '!tests.rs' --glob '!execution_tests.rs' --glob '!differential_tests.rs'
	@! rg -n 'std\.foundation/(assoc|get|first|rest|map|reduce|conj)|std\.foundation\.coroutine/(await|yield)' core/rust/src core/rust/tests core/java/src/test core/rust/web --glob '*.rs' --glob '*.java' --glob '*.mjs' --glob '*.js' --glob '!node_modules/**'
	@! rg -n 'std\.foundation(?:\.|/)|hara-specs-registry|HARA_SPECS_REGISTRY' core/rust/specs/native-protocol-v1.edn core/rust/specs/language-v1.edn core/java/src/test/java/hara/truffle/bytecode/HbcCodecTest.java core/rust/web/native-protocol-conformance.test.mjs core/rust/web/language-conformance.test.mjs

test-rust:
	cargo check --manifest-path core/rust/Cargo.toml --bin hara-native
	cargo test --manifest-path core/rust/Cargo.toml --bin hara-native

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
