import { expect, test } from "@playwright/test";

test("IndexedDB filesystem runs transactional byte and revision conformance in Chromium", async ({ page }) => {
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const { createIndexedDbFilesystemFactory } = await import(
      "/rust/web/host/indexeddb-filesystem-provider.js"
    );
    const database = `hara-browser-indexeddb-${crypto.randomUUID()}`;
    const factory = createIndexedDbFilesystemFactory({ databaseName: database });
    const first = await factory.open({ namespace: "workspace", quotaBytes: 1024 * 1024 });
    const second = await factory.open({ namespace: "workspace", quotaBytes: 1024 * 1024 });
    try {
      await first.mkdir(null, "/src", { parents: true });
      await first.write(
        null,
        "/src/main.bin",
        new Uint8Array([0, 1, 0, 255]),
        { mode: "create" }
      );
      const stale = await first.stat(null, "/src/main.bin");
      await second.write(
        null,
        "/src/main.bin",
        new Uint8Array([7, 8]),
        { mode: "replace" }
      );
      let conflict = null;
      try {
        await first.write(
          null,
          "/src/main.bin",
          new Uint8Array([9]),
          { mode: "replace" },
          { expectedRevision: stale.extensions["file/revision"] }
        );
      } catch (error) {
        conflict = error.code;
      }
      const bytes = [...await first.read(null, "/src/main.bin")];
      const page = await first.entriesPage(null, "/src", { limit: 16 });
      return {
        bytes,
        conflict,
        paths: page.entries.map((entry) => entry.path),
        descriptor: first.descriptor()
      };
    } finally {
      await first.close();
      await second.close();
      await new Promise((resolve, reject) => {
        const request = indexedDB.deleteDatabase(database);
        request.onsuccess = () => resolve();
        request.onerror = () => reject(request.error);
      });
    }
  });

  expect(result.bytes).toEqual([7, 8]);
  expect(result.conflict).toBe("file/conflict");
  expect(result.paths).toEqual(["/src/main.bin"]);
  expect(result.descriptor.kind).toBe("indexeddb");
  expect(result.descriptor.capabilities).toContain("transactions");
});
