import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";

import {
  FilesystemProviderError,
  createIndexedDbFilesystemFactory,
  normalizeLogicalPath
} from "./host/indexeddb-filesystem.js";

let sequence = 0;
const databaseName = (label) => `hara-indexeddb-filesystem-${label}-${++sequence}`;

async function open(label, configuration = {}, factoryOptions = {}) {
  const database = databaseName(label);
  const factory = createIndexedDbFilesystemFactory({ databaseName: database, ...factoryOptions });
  const provider = await factory.open({ namespace: "workspace", ...configuration });
  return { database, factory, provider };
}

async function removeDatabase(name) {
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    request.onsuccess = () => resolve();
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error(`database deletion blocked: ${name}`));
  });
}

async function closeAndRemove(database, ...providers) {
  for (const provider of providers) await provider?.close();
  await removeDatabase(database);
}

function expectCode(code) {
  return (error) => {
    assert.ok(error instanceof FilesystemProviderError);
    assert.equal(error.code, code);
    assert.equal(error.data()["file/provider"], "indexeddb");
    return true;
  };
}

test("logical paths are canonical and cannot escape the mounted root", () => {
  assert.equal(normalizeLogicalPath("src//./main.hal"), "/src/main.hal");
  assert.equal(normalizeLogicalPath("/"), "/");
  assert.throws(() => normalizeLogicalPath("../../outside"), expectCode("file/outside-root"));
  assert.throws(() => normalizeLogicalPath("C:/outside"), expectCode("file/invalid-path"));
  assert.throws(() => normalizeLogicalPath("src\\main.hal"), expectCode("file/invalid-path"));
});

test("factory validates trusted configuration and returns a redacted descriptor", async () => {
  const database = databaseName("descriptor");
  const factory = createIndexedDbFilesystemFactory({ databaseName: database });
  assert.equal(factory.kind, "indexeddb");
  assert.throws(() => factory.validate({}), /namespace/);
  assert.throws(
    () => factory.validate({ namespace: "workspace", version: 2 }),
    /schema version/
  );
  const provider = await factory.open({
    namespace: "workspace",
    quotaBytes: 1024,
    maxFileBytes: 512
  });
  const descriptor = provider.descriptor();
  assert.equal(descriptor.kind, "indexeddb");
  assert.equal(descriptor.display, "IndexedDB:workspace");
  assert.equal(descriptor["read-only?"], false);
  assert.deepEqual(descriptor.capabilities, [
    "read", "write", "entries", "mkdir", "delete", "copy", "move", "append",
    "revision-check", "transactions"
  ]);
  assert.equal(descriptor.extensions["provider/schema-version"], 1);
  assert.equal(JSON.stringify(descriptor).includes(database), false);
  await closeAndRemove(database, provider);
});

test("create replace append and exact bytes preserve entry identity", async () => {
  const { database, provider } = await open("write-modes");
  try {
    await provider.mkdir(null, "/src", { parents: true, existsOk: true });
    const created = await provider.write(
      null,
      "/src/main.bin",
      new Uint8Array([0, 1, 0, 255]),
      { mode: "create" }
    );
    const first = await provider.stat(null, "/src/main.bin");
    assert.equal(first.path, "/src/main.bin");
    assert.equal(first.type, "file");
    assert.equal(first.size, 4);
    assert.equal(first.extensions["file/revision"], created.revision);
    assert.deepEqual(await provider.read(null, "/src/main.bin"), new Uint8Array([0, 1, 0, 255]));

    await assert.rejects(
      provider.write(null, "/src/main.bin", new Uint8Array([9]), { mode: "create" }),
      expectCode("file/already-exists")
    );
    await provider.write(null, "/src/main.bin", new Uint8Array([7]), { mode: "replace" });
    await provider.write(null, "/src/main.bin", new Uint8Array([8, 9]), { mode: "append" });
    const final = await provider.stat(null, "/src/main.bin");
    assert.equal(final.extensions["file/id"], first.extensions["file/id"]);
    assert.notEqual(final.extensions["file/revision"], first.extensions["file/revision"]);
    assert.deepEqual(await provider.read(null, "/src/main.bin"), new Uint8Array([7, 8, 9]));
  } finally {
    await closeAndRemove(database, provider);
  }
});

test("entries page deterministically across opaque continuation tokens", async () => {
  const { database, provider } = await open("pagination");
  try {
    await provider.mkdir(null, "/data", { parents: true });
    for (const name of ["z", "a", "m", "b"]) {
      await provider.write(null, `/data/${name}`, new Uint8Array([name.charCodeAt(0)]), {
        mode: "create"
      });
    }
    const first = await provider.entriesPage(null, "/data", { limit: 2 });
    assert.deepEqual(first.entries.map((entry) => entry.path), ["/data/a", "/data/b"]);
    assert.equal(typeof first.nextToken, "string");
    const second = await provider.entriesPage(null, "/data", {
      limit: 2,
      token: first.nextToken
    });
    assert.deepEqual(second.entries.map((entry) => entry.path), ["/data/m", "/data/z"]);
    assert.equal(second.nextToken, null);
  } finally {
    await closeAndRemove(database, provider);
  }
});

test("two provider instances enforce optimistic revision conflicts", async () => {
  const database = databaseName("conflict");
  const factory = createIndexedDbFilesystemFactory({ databaseName: database });
  const first = await factory.open({ namespace: "workspace" });
  const second = await factory.open({ namespace: "workspace" });
  try {
    await first.write(null, "/value.bin", new Uint8Array([1]), { mode: "create" });
    const stale = await first.stat(null, "/value.bin");
    await second.write(null, "/value.bin", new Uint8Array([2]), { mode: "replace" });
    await assert.rejects(
      first.write(
        null,
        "/value.bin",
        new Uint8Array([3]),
        { mode: "replace" },
        { expectedRevision: stale.extensions["file/revision"] }
      ),
      (error) => {
        expectCode("file/conflict")(error);
        assert.equal(error.data()["file/expected-revision"], stale.extensions["file/revision"]);
        assert.notEqual(error.data()["file/revision"], stale.extensions["file/revision"]);
        return true;
      }
    );
    assert.deepEqual(await first.read(null, "/value.bin"), new Uint8Array([2]));
  } finally {
    await closeAndRemove(database, first, second);
  }
});

test("copy move and delete mutate one namespace without partial results", async () => {
  const { database, provider } = await open("mutations");
  try {
    await provider.mkdir(null, "/a", { parents: true });
    await provider.write(null, "/a/source", new Uint8Array([4, 5]), { mode: "create" });
    await provider.copy(null, "/a/source", "/a/copied", { replace: false });
    assert.deepEqual(await provider.read(null, "/a/copied"), new Uint8Array([4, 5]));
    await provider.move(null, "/a/copied", "/moved", { parents: false });
    await assert.rejects(provider.stat(null, "/a/copied"), expectCode("file/not-found"));
    assert.deepEqual(await provider.read(null, "/moved"), new Uint8Array([4, 5]));
    await provider.delete(null, "/moved");
    await assert.rejects(provider.read(null, "/moved"), expectCode("file/not-found"));
    await provider.mkdir(null, "/a/non-empty", { parents: true });
    await provider.write(null, "/a/non-empty/item", new Uint8Array(), { mode: "create" });
    await assert.rejects(
      provider.delete(null, "/a/non-empty"),
      expectCode("file/directory-not-empty")
    );
  } finally {
    await closeAndRemove(database, provider);
  }
});

test("an injected mutation failure aborts every IndexedDB record change", async () => {
  let fail = false;
  const { database, provider } = await open("rollback", {}, {
    faultInjector(stage) {
      if (fail && stage === "move:after-records") throw new Error("injected failure");
    }
  });
  try {
    await provider.write(null, "/source", new Uint8Array([1, 2, 3]), { mode: "create" });
    fail = true;
    await assert.rejects(
      provider.move(null, "/source", "/target", { replace: false }),
      expectCode("file/io")
    );
    assert.deepEqual(await provider.read(null, "/source"), new Uint8Array([1, 2, 3]));
    await assert.rejects(provider.stat(null, "/target"), expectCode("file/not-found"));
  } finally {
    await closeAndRemove(database, provider);
  }
});

test("file and mount quotas fail before a successful transaction is exposed", async () => {
  const { database, provider } = await open("quota", {
    quotaBytes: 5,
    maxFileBytes: 4
  });
  try {
    await assert.rejects(
      provider.write(null, "/too-large", new Uint8Array(5), { mode: "create" }),
      expectCode("file/quota-exceeded")
    );
    await assert.rejects(provider.stat(null, "/too-large"), expectCode("file/not-found"));
    await provider.write(null, "/a", new Uint8Array(3), { mode: "create" });
    await assert.rejects(
      provider.write(null, "/b", new Uint8Array(3), { mode: "create" }),
      expectCode("file/quota-exceeded")
    );
    await assert.rejects(provider.stat(null, "/b"), expectCode("file/not-found"));
  } finally {
    await closeAndRemove(database, provider);
  }
});

test("namespaces are isolated inside one database", async () => {
  const database = databaseName("namespaces");
  const factory = createIndexedDbFilesystemFactory({ databaseName: database });
  const alpha = await factory.open({ namespace: "alpha" });
  const beta = await factory.open({ namespace: "beta" });
  try {
    await alpha.write(null, "/same", new Uint8Array([1]), { mode: "create" });
    await beta.write(null, "/same", new Uint8Array([2]), { mode: "create" });
    assert.deepEqual(await alpha.read(null, "/same"), new Uint8Array([1]));
    assert.deepEqual(await beta.read(null, "/same"), new Uint8Array([2]));
  } finally {
    await closeAndRemove(database, alpha, beta);
  }
});

test("cancellation and close reject deterministically", async () => {
  const { database, provider } = await open("lifecycle");
  try {
    const cancellation = new AbortController();
    cancellation.abort();
    await assert.rejects(
      provider.stat({ signal: cancellation.signal }, "/"),
      expectCode("file/cancelled")
    );
    await provider.close();
    await assert.rejects(provider.stat(null, "/"), expectCode("file/provider-closed"));
  } finally {
    await removeDatabase(database);
  }
});
