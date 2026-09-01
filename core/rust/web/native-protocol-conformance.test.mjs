import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { start as startNativeFull } from "./packages/browser/dist/native-full/hara.mjs";
import { start as startNativeVm } from "./packages/browser/dist/native-vm/hara.mjs";
import { HtaKeyword, parseEdnData } from "./packages/hta/index.js";
import { CapabilityRegistry } from "./studio/capability-registry.js";

const HNC_MAGIC = "HNC1";
const ERROR_EXPECTATION_PREFIX = "!error:";

function expectedErrorCategory(expectation) {
  return expectation.startsWith(ERROR_EXPECTATION_PREFIX)
    ? expectation.slice(ERROR_EXPECTATION_PREFIX.length)
    : null;
}

function normalizedErrorCategory(error) {
  const message = String(error?.message ?? error);
  if (message.includes("protocol/arity:")) return "protocol/arity";
  if (message.includes("protocol/unsupported-receiver:")) return "protocol/unsupported-receiver";
  if (message.includes("Ex$Arity") || message.includes("Wrong number of args")) {
    return "native/arity";
  }
  if (/^Expected .* arguments, received /.test(message)) return "native/arity";
  if (message.includes("expects")) {
    if (/expects (?:no|one|two|three|four|at least)\b/.test(message)) {
      return "native/arity";
    }
    if (/number|numeric|integer|string/.test(message)) return "native/type";
    return "native/arity";
  }
  return null;
}

function parseHnc(bytes) {
  assert.equal(new TextDecoder().decode(bytes.subarray(0, 4)), HNC_MAGIC);
  assert.ok(bytes.byteLength >= 36, "HNC1 header is complete");
  const expectedDigest = Buffer.from(bytes.subarray(4, 36)).toString("hex");
  const payload = bytes.subarray(36);
  assert.equal(createHash("sha256").update(payload).digest("hex"), expectedDigest);

  let offset = 0;
  const take = (length) => {
    assert.ok(Number.isSafeInteger(length) && length >= 0, "HNC1 length is valid");
    const end = offset + length;
    assert.ok(end <= payload.byteLength, "HNC1 field remains within the payload");
    const value = payload.subarray(offset, end);
    offset = end;
    return value;
  };
  const takeU32 = () => {
    const field = take(4);
    return new DataView(field.buffer, field.byteOffset, field.byteLength).getUint32(0, true);
  };
  const takeBytes = () => take(takeU32());
  const decode = (value) => new TextDecoder().decode(value);
  const suiteCount = takeU32();
  assert.equal(suiteCount, 2, "HNC1 contains native and protocol suites");
  const suites = [];
  for (let suiteIndex = 0; suiteIndex < suiteCount; suiteIndex += 1) {
    const id = decode(takeBytes());
    const setup = takeBytes();
    const caseCount = takeU32();
    assert.ok(caseCount > 0, `${id} suite has cases`);
    const cases = [];
    for (let caseIndex = 0; caseIndex < caseCount; caseIndex += 1) {
      cases.push({ id: decode(takeBytes()), expectedDisplay: decode(takeBytes()), artifact: takeBytes() });
    }
    suites.push({ id, setup, cases });
  }
  assert.equal(offset, payload.byteLength, "HNC1 has no trailing bytes");
  assert.deepEqual(suites.map((suite) => suite.id), ["native", "protocol"]);
  return suites;
}

function field(map, name) {
  for (const [key, value] of map) {
    if (key instanceof HtaKeyword && key.name === name) return value;
  }
  throw new Error(`capability profile record is missing :${name}`);
}

function keywords(value, fieldName) {
  assert.ok(Array.isArray(value), `${fieldName} must be a vector`);
  return value.map((entry) => {
    assert.ok(entry instanceof HtaKeyword, `${fieldName} entries must be keywords`);
    return entry.name;
  });
}

async function capabilityProfiles() {
  const source = await readFile(
    new URL("../assets/native-capability-profiles-v1.edn", import.meta.url),
    "utf8",
  );
  const corpus = parseEdnData(source, "native/capability-profiles-malformed");
  assert.ok(corpus instanceof Map, "capability profile corpus must be a map");
  assert.equal(field(corpus, "format"), "hara.native/capability-profiles/v1");
  const capabilities = keywords(field(corpus, "capabilities"), "capabilities");
  const profiles = field(corpus, "profiles");
  assert.ok(Array.isArray(profiles), "profiles must be a vector");
  return {
    capabilities,
    profiles: profiles.map((profile) => {
      assert.ok(profile instanceof Map, "profile must be a map");
      const id = field(profile, "id");
      assert.ok(id instanceof HtaKeyword, "profile id must be a keyword");
      return { id: id.name, grants: keywords(field(profile, "grants"), "profile grants") };
    }),
  };
}

test("browser core executes native/protocol HNC1 cases in both Wasm profiles", async () => {
  const bytes = new Uint8Array(
    await readFile(new URL("../assets/native-protocol-conformance.hnc", import.meta.url)),
  );
  const suites = parseHnc(bytes);
  assert.doesNotMatch(
    new TextDecoder().decode(bytes),
    /std\.foundation(?:\.|\/)[^/]/,
    "HNC1 artifacts must not require Foundation methods",
  );

  const expected = suites.reduce((count, suite) => count + suite.cases.length, 0);
  for (const [profile, start] of [["native-vm", startNativeVm], ["native-full", startNativeFull]]) {
    const hara = await start();
    let executed = 0;
    try {
      for (const suite of suites) {
        hara.evalBytecode(suite.setup);
        for (const testCase of suite.cases) {
          const expectedError = expectedErrorCategory(testCase.expectedDisplay);
          try {
            const actual = hara.evalBytecode(testCase.artifact);
            assert.equal(expectedError, null, `${profile} ${testCase.id} expected ${expectedError} but returned ${actual}`);
            assert.equal(actual, testCase.expectedDisplay, `${profile} ${testCase.id}`);
          } catch (error) {
            if (expectedError === null) throw error;
            assert.equal(normalizedErrorCategory(error), expectedError, `${profile} ${testCase.id}`);
          }
          executed += 1;
        }
      }
    } finally {
      await hara.dispose();
    }
    assert.equal(executed, expected, `${profile} ran all declared native/protocol cases`);
  }
});

test("HNC1 outcome categories reject the wrong normalized error", () => {
  assert.equal(expectedErrorCategory("!error:protocol/arity"), "protocol/arity");
  assert.equal(normalizedErrorCategory(new Error("protocol/unsupported-receiver: missing")), "protocol/unsupported-receiver");
  assert.equal(normalizedErrorCategory(new Error("abs expects one numeric value")), "native/arity");
  assert.equal(normalizedErrorCategory(new Error("abs expects a numeric value")), "native/type");
  assert.notEqual(
    normalizedErrorCategory(new Error("protocol/unsupported-receiver: missing")),
    "protocol/arity",
  );
});

test("browser session grants conform to the shared native capability profiles", async () => {
  const corpus = await capabilityProfiles();
  assert.deepEqual(corpus.capabilities, [
    "kernel", "sandbox", "file", "network", "native-runtime", "host-call",
  ]);
  assert.deepEqual(corpus.profiles.map((profile) => profile.id), [
    "zero", "kernel-sandbox", "file", "network", "native-runtime", "host-call", "all",
  ]);

  const registry = new CapabilityRegistry({ capabilities: corpus.capabilities });
  for (const profile of corpus.profiles) {
    const sessionId = `capability-profile/${profile.id}`;
    assert.deepEqual(registry.grant(sessionId, profile.grants), profile.grants.slice().sort());
    for (const capability of corpus.capabilities) {
      if (profile.grants.includes(capability)) {
        assert.doesNotThrow(() => registry.assert(sessionId, [capability]));
      } else {
        assert.throws(
          () => registry.assert(sessionId, [capability]),
          (error) => error?.code === "program/capability-denied",
          `${profile.id} denies ${capability}`,
        );
      }
    }
  }
});
