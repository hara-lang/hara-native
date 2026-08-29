import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";

import { createHostServices } from "./host/provider-services.js";

let sequence = 0;
const databaseName = (label) => `hara-provider-services-${label}-${++sequence}`;

function field(map, name) {
  for (const [key, value] of map) {
    if (key?.name === name) return value;
  }
  return undefined;
}

async function removeDatabase(name) {
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    request.onsuccess = () => resolve();
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error(`database deletion blocked: ${name}`));
  });
}

test("provider services route IndexedDB mounts through the transactional host", async () => {
  const base = databaseName("indexeddb");
  const physical = `${base}-filesystems`;
  const services = createHostServices({ dbName: base });
  const context = {};
  const invocation = { kernelContext: context, mountId: 1, sessionId: "ROOT" };
  try {
    await services.filesystemHost.register(context, 1, {
      provider: "indexeddb",
      key: "workspace"
    });
    assert.equal(services.filesystemHost.descriptor(context, 1).kind, "indexeddb");
    await services["file/mkdir"].call(invocation, "/src");
    assert.equal(
      await services["file/write"].call(
        invocation,
        "/src/main.bin",
        new Uint8Array([0, 1, 0, 255]),
        new Map([[{ name: "mode" }, { name: "create" }]])
      ),
      "/src/main.bin"
    );
    assert.deepEqual(
      await services["file/read"].call(invocation, "/src/main.bin"),
      new Uint8Array([0, 1, 0, 255])
    );
    const stat = await services["file/stat"].call(invocation, "/src/main.bin");
    assert.ok(stat instanceof Map);
    assert.equal(field(stat, "path"), "/src/main.bin");
    assert.equal(field(stat, "type").name, "file");
    const entries = await services["file/entries"].call(invocation, "/src");
    assert.deepEqual(entries.map((entry) => field(entry, "path")), ["/src/main.bin"]);
    assert.deepEqual(await services["file/list"].call(invocation, "/src"), ["/src/main.bin"]);
    assert.deepEqual(await services["file/walk"].call(invocation, "/"), ["/src/main.bin"]);
    assert.equal(
      await services["file/copy"].call(
        invocation, "/src/main.bin", "/src/copy.bin"
      ),
      "/src/copy.bin"
    );
    assert.equal(
      await services["file/move"].call(
        invocation, "/src/copy.bin", "/moved.bin"
      ),
      "/moved.bin"
    );
    assert.equal(await services["file/delete"].call(invocation, "/moved.bin"), "/moved.bin");
    assert.equal(await services["file/exists?"].call(invocation, "/moved.bin"), false);
  } finally {
    await services.filesystemHost.closeContext(context);
    await removeDatabase(physical);
  }
});

test("memory mounts retain their existing compatibility implementation", async () => {
  const services = createHostServices({ indexedDB: null });
  const context = {};
  const invocation = { kernelContext: context, mountId: 1, sessionId: "ROOT" };
  try {
    await services.filesystemHost.register(context, 1, { provider: "memory" });
    await services["file/mkdir"].call(invocation, "/src");
    await services["file/write"].call(
      invocation, "/src/main.hal", new Uint8Array([1, 2])
    );
    assert.deepEqual(
      await services["file/read"].call(invocation, "/src/main.hal"),
      new Uint8Array([1, 2])
    );
    assert.deepEqual(await services["file/list"].call(invocation, "/src"), ["/src/main.hal"]);
    assert.equal(services.filesystemHost.descriptor(context, 1).kind, "memory");
  } finally {
    await services.filesystemHost.closeContext(context);
  }
});

test("filesystem calls retain per-session capability gating", async () => {
  const base = databaseName("denied");
  const physical = `${base}-filesystems`;
  const services = createHostServices({
    dbName: base,
    grantedCapabilities: ["store"]
  });
  const context = {};
  const invocation = { kernelContext: context, mountId: 1, sessionId: "ROOT" };
  try {
    await services.filesystemHost.register(context, 1, {
      provider: "indexeddb",
      namespace: "workspace"
    });
    assert.throws(
      () => services["file/read"].call(invocation, "/missing"),
      /host\/capability-denied:filesystem/
    );
  } finally {
    await services.filesystemHost.closeContext(context);
    await removeDatabase(physical);
  }
});
