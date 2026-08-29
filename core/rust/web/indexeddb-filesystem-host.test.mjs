import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";

import { createIndexedDbFilesystemHost } from "./host/indexeddb-filesystem-host.js";

let sequence = 0;
const databaseName = () => `hara-indexeddb-host-${++sequence}`;

async function removeDatabase(name) {
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    request.onsuccess = () => resolve();
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error(`database deletion blocked: ${name}`));
  });
}

test("host registers redacted mounts and dispatches the complete primitive surface", async () => {
  const database = databaseName();
  const host = createIndexedDbFilesystemHost({ databaseName: database });
  const context = {};
  try {
    const descriptor = await host.register(context, 1, {
      provider: "indexeddb",
      key: "workspace"
    });
    assert.equal(descriptor.kind, "indexeddb");
    assert.equal(JSON.stringify(descriptor).includes(database), false);
    await host.invoke(context, 1, "mkdir", ["/src", { parents: true }]);
    await host.invoke(context, 1, "write", [
      "/src/main.hal",
      new Uint8Array([1, 2]),
      { mode: "create" }
    ]);
    assert.equal(await host.invoke(context, 1, "exists", ["/src/main.hal"]), true);
    assert.deepEqual(
      await host.invoke(context, 1, "read", ["/src/main.hal"]),
      new Uint8Array([1, 2])
    );
    const entry = await host.invoke(context, 1, "stat", ["/src/main.hal"]);
    assert.equal(entry.extensions["file/id"].startsWith("node-"), true);
    assert.deepEqual(await host.invoke(context, 1, "list", ["/src"]), ["/src/main.hal"]);
    await host.invoke(context, 1, "copy", [
      "/src/main.hal", "/src/copy.hal", { replace: false }
    ]);
    await host.invoke(context, 1, "move", [
      "/src/copy.hal", "/moved.hal", { replace: false }
    ]);
    await host.invoke(context, 1, "delete", ["/moved.hal"]);
    assert.equal(await host.invoke(context, 1, "exists?", ["/moved.hal"]), false);
  } finally {
    await host.closeContext(context);
    await removeDatabase(database);
  }
});

test("equal mount ids remain isolated by kernel context", async () => {
  const database = databaseName();
  const host = createIndexedDbFilesystemHost({ databaseName: database });
  const alpha = {};
  const beta = {};
  try {
    await host.register(alpha, 1, { namespace: "alpha" });
    await host.register(beta, 1, { namespace: "beta" });
    await host.invoke(alpha, 1, "write", ["/value", new Uint8Array([1]), { mode: "create" }]);
    await host.invoke(beta, 1, "write", ["/value", new Uint8Array([2]), { mode: "create" }]);
    assert.deepEqual(await host.invoke(alpha, 1, "read", ["/value"]), new Uint8Array([1]));
    assert.deepEqual(await host.invoke(beta, 1, "read", ["/value"]), new Uint8Array([2]));
  } finally {
    await host.closeContext(alpha);
    await host.closeContext(beta);
    await removeDatabase(database);
  }
});

test("persistent namespace identity is independent of transient mount ids", async () => {
  const database = databaseName();
  const firstHost = createIndexedDbFilesystemHost({ databaseName: database });
  const secondHost = createIndexedDbFilesystemHost({ databaseName: database });
  const firstContext = {};
  const secondContext = {};
  try {
    await firstHost.register(firstContext, 1, { key: "shared" });
    await firstHost.invoke(firstContext, 1, "write", [
      "/state", new Uint8Array([7, 8]), { mode: "create" }
    ]);
    await firstHost.close(firstContext, 1);
    await secondHost.register(secondContext, 99, { key: "shared" });
    assert.deepEqual(
      await secondHost.invoke(secondContext, 99, "read", ["/state"]),
      new Uint8Array([7, 8])
    );
  } finally {
    await firstHost.closeContext(firstContext);
    await secondHost.closeContext(secondContext);
    await removeDatabase(database);
  }
});

test("closing one mount removes only that opened capability", async () => {
  const database = databaseName();
  const host = createIndexedDbFilesystemHost({ databaseName: database });
  const context = {};
  try {
    await host.register(context, 1, { namespace: "one" });
    await host.register(context, 2, { namespace: "two" });
    assert.equal(await host.close(context, 1), true);
    await assert.rejects(host.invoke(context, 1, "stat", ["/"]), (error) => {
      assert.equal(error.code, "file/provider-closed");
      return true;
    });
    assert.equal((await host.invoke(context, 2, "stat", ["/"])).type, "directory");
  } finally {
    await host.closeContext(context);
    await removeDatabase(database);
  }
});
