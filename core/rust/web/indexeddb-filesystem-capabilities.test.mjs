import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";

import { createIndexedDbFilesystemFactory } from "./host/indexeddb-filesystem-provider.js";

let sequence = 0;

async function removeDatabase(name) {
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    request.onsuccess = () => resolve();
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error(`database deletion blocked: ${name}`));
  });
}

function expectCode(code) {
  return (error) => {
    assert.equal(error?.code, code);
    return true;
  };
}

test("unsupported mutation semantics reject instead of weakening the contract", async () => {
  const database = `hara-indexeddb-capabilities-${++sequence}`;
  const factory = createIndexedDbFilesystemFactory({ databaseName: database });
  const filesystem = await factory.open({ namespace: "workspace" });
  try {
    assert.equal(filesystem.capabilities().includes("atomic-move"), false);
    assert.equal(filesystem.capabilities().includes("preserve-modified"), false);
    await filesystem.write(
      null,
      "/source",
      new Uint8Array([1, 2, 3]),
      { mode: "create" }
    );

    await assert.rejects(
      filesystem.copy(
        null,
        "/source",
        "/copy",
        { preserveModified: true }
      ),
      (error) => {
        expectCode("file/unsupported")(error);
        assert.equal(error.providerCode, "preserve-modified-unavailable");
        return true;
      }
    );
    await assert.rejects(
      filesystem.stat(null, "/copy"),
      expectCode("file/not-found")
    );

    await assert.rejects(
      filesystem.move(
        null,
        "/source",
        "/target",
        { atomic: true }
      ),
      (error) => {
        expectCode("file/unsupported")(error);
        assert.equal(error.providerCode, "atomic-move-unavailable");
        return true;
      }
    );
    assert.deepEqual(
      await filesystem.read(null, "/source"),
      new Uint8Array([1, 2, 3])
    );
    await assert.rejects(
      filesystem.stat(null, "/target"),
      expectCode("file/not-found")
    );
  } finally {
    await filesystem.close();
    await removeDatabase(database);
  }
});
