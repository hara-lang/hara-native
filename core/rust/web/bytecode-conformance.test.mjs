import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { start } from "./packages/browser/dist/native-vm/hara.mjs";

const HCC_MAGIC = "HCC0";

function parseHcc(bytes) {
  assert.equal(new TextDecoder().decode(bytes.subarray(0, 4)), HCC_MAGIC);
  assert.ok(bytes.byteLength >= 36, "HCC0 header is complete");
  const expectedDigest = Buffer.from(bytes.subarray(4, 36)).toString("hex");
  const payload = bytes.subarray(36);
  assert.equal(createHash("sha256").update(payload).digest("hex"), expectedDigest);

  let offset = 0;
  const take = (length) => {
    assert.ok(Number.isSafeInteger(length) && length >= 0, "HCC0 length is valid");
    const end = offset + length;
    assert.ok(end <= payload.byteLength, "HCC0 field remains within the payload");
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
  assert.ok(count <= payload.byteLength / 12, "HCC0 case count fits its payload");
  const cases = [];
  for (let index = 0; index < count; index += 1) {
    cases.push({
      id: decode(takeBytes()),
      expectedDisplay: decode(takeBytes()),
      artifact: takeBytes(),
    });
  }
  assert.equal(offset, payload.byteLength, "HCC0 has no trailing bytes");
  return cases;
}

test("browser core executes every source-free HCC success case serially", async () => {
  const bytes = new Uint8Array(
    await readFile(new URL("../assets/bytecode-conformance.hcc", import.meta.url)),
  );
  const cases = parseHcc(bytes);
  assert.ok(cases.length >= 80, "HCC0 corpus retains its full success lane");
  assert.doesNotMatch(
    new TextDecoder().decode(bytes),
    /std\.foundation\/[^/]/,
    "HCC0 success artifacts must not require Foundation methods",
  );

  const hara = await start();
  let executed = 0;
  let failureOwnershipRequired = 0;
  try {
    for (const testCase of cases) {
      if (testCase.id.startsWith("error/")) {
        failureOwnershipRequired += 1;
        continue;
      }
      assert.equal(
        hara.evalBytecode(testCase.artifact),
        testCase.expectedDisplay,
        testCase.id,
      );
      executed += 1;
    }
  } finally {
    await hara.dispose();
  }
  assert.equal(
    executed,
    cases.length - failureOwnershipRequired,
    "only failure-ownership vectors may be held outside the source-free success lane",
  );
  assert.ok(failureOwnershipRequired > 0, "HCC0 retains error ownership vectors");
});
