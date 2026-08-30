import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { start as startNativeFull } from "./packages/browser/dist/native-full/hara.mjs";
import { start as startNativeVm } from "./packages/browser/dist/native-vm/hara.mjs";

const HLC_MAGIC = "HLC1";
const ERROR_EXPECTATION_PREFIX = "!error:";

function expectedErrorCategory(expectation) {
  return expectation.startsWith(ERROR_EXPECTATION_PREFIX)
    ? expectation.slice(ERROR_EXPECTATION_PREFIX.length)
    : null;
}

function normalizedErrorCategory(error) {
  const message = String(error?.message ?? error);
  const lower = message.toLowerCase();
  if (lower.includes("divide by zero") || lower.includes("division by zero")) return "division by zero";
  if (lower.includes("expects") && lower.includes("numbers")) return "expects numbers";
  return null;
}

function parseHlc(bytes) {
  assert.equal(new TextDecoder().decode(bytes.subarray(0, 4)), HLC_MAGIC);
  assert.ok(bytes.byteLength >= 36, "HLC1 header is complete");
  const expectedDigest = Buffer.from(bytes.subarray(4, 36)).toString("hex");
  const payload = bytes.subarray(36);
  assert.equal(createHash("sha256").update(payload).digest("hex"), expectedDigest);

  let offset = 0;
  const take = (length) => {
    assert.ok(Number.isSafeInteger(length) && length >= 0, "HLC1 length is valid");
    const end = offset + length;
    assert.ok(end <= payload.byteLength, "HLC1 field remains within the payload");
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
  const count = takeU32();
  assert.ok(count >= 20, "HLC1 has functional behavior cases");
  const cases = [];
  for (let index = 0; index < count; index += 1) {
    const id = decode(takeBytes());
    const layer = decode(takeBytes());
    assert.ok(["parser", "evaluator", "native-abi"].includes(layer), `${id} has a supported layer`);
    const expectation = decode(takeBytes());
    const browserSafe = takeU32();
    assert.ok(browserSafe === 0 || browserSafe === 1, `${id} browser-safe field is a boolean`);
    const source = decode(takeBytes());
    const artifact = takeBytes();
    cases.push({ id, layer, expectation, browserSafe: browserSafe === 1, source, artifact });
  }
  assert.equal(offset, payload.byteLength, "HLC1 has no trailing bytes");
  return cases;
}

test("browser profiles execute every Rust-produced HLC1 behavior case", async () => {
  const bytes = new Uint8Array(
    await readFile(new URL("../assets/language-conformance.hlc", import.meta.url)),
  );
  const cases = parseHlc(bytes);
  assert.ok(cases.some((testCase) => testCase.layer === "parser"));
  assert.ok(cases.some((testCase) => testCase.layer === "evaluator"));
  assert.ok(cases.some((testCase) => testCase.layer === "native-abi"));
  assert.ok(cases.some((testCase) => testCase.browserSafe));
  for (const [profile, start] of [["native-vm", startNativeVm], ["native-full", startNativeFull]]) {
    const hara = await start();
    let executed = 0;
    try {
      for (const testCase of cases) {
        assert.doesNotMatch(testCase.source, /std\.(?:foundation|lib)(?:\.|\/)/, `${testCase.id} source`);
        const expectedError = expectedErrorCategory(testCase.expectation);
        try {
          const actual = hara.evalBytecode(testCase.artifact);
          assert.equal(expectedError, null, `${profile} ${testCase.id} expected ${expectedError} but returned ${actual}`);
          assert.equal(actual, testCase.expectation, `${profile} ${testCase.id}`);
        } catch (error) {
          if (expectedError === null) throw error;
          assert.equal(normalizedErrorCategory(error), expectedError, `${profile} ${testCase.id}`);
        }
        executed += 1;
      }
    } finally {
      await hara.dispose();
    }
    assert.equal(executed, cases.length, `${profile} ran every HLC1 case`);
  }
});

test("HLC1 error expectations reject the wrong normalized error", () => {
  assert.equal(expectedErrorCategory("!error:division by zero"), "division by zero");
  assert.equal(normalizedErrorCategory(new Error("Divide by zero")), "division by zero");
  assert.equal(normalizedErrorCategory(new Error("+ expects two numbers")), "expects numbers");
  assert.notEqual(normalizedErrorCategory(new Error("+ expects two numbers")), "division by zero");
});
