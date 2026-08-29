const SCHEMA_VERSION = 1;
const META_STORE = "filesystem-meta";
const NODE_STORE = "filesystem-nodes";
const CONTENT_STORE = "filesystem-contents";
const PARENT_NAME_INDEX = "by-parent-name";
const PARENT_INDEX = "by-parent";
const DEFAULT_DATABASE = "hara-filesystems";
const DEFAULT_QUOTA_BYTES = 100 * 1024 * 1024;
const DEFAULT_MAX_FILE_BYTES = 32 * 1024 * 1024;
const DEFAULT_PAGE_LIMIT = 256;
const MAX_PAGE_LIMIT = 1024;

const CAPABILITIES = Object.freeze([
  "read",
  "write",
  "entries",
  "mkdir",
  "delete",
  "copy",
  "move",
  "append",
  "revision-check",
  "transactions"
]);

export class FilesystemProviderError extends Error {
  constructor(code, message, {
    operation = null,
    path = null,
    target = null,
    providerCode = null,
    retryable = false,
    expectedRevision = null,
    actualRevision = null,
    cause = null
  } = {}) {
    super(message, cause == null ? undefined : { cause });
    this.name = "FilesystemProviderError";
    this.code = code.startsWith("file/") ? code : `file/${code}`;
    this.operation = operation;
    this.path = path;
    this.target = target;
    this.provider = "indexeddb";
    this.providerCode = providerCode;
    this.retryable = Boolean(retryable);
    this.expectedRevision = expectedRevision;
    this.actualRevision = actualRevision;
  }

  data() {
    return Object.freeze({
      "ex/code": this.code,
      "file/operation": this.operation,
      "file/path": this.path,
      "file/target": this.target,
      "file/provider": this.provider,
      "file/provider-code": this.providerCode,
      "file/retryable?": this.retryable,
      "file/expected-revision": this.expectedRevision,
      "file/revision": this.actualRevision
    });
  }
}

export function createIndexedDbFilesystemFactory({
  indexedDB: indexedDBFactory = globalThis.indexedDB,
  databaseName = DEFAULT_DATABASE,
  now = () => Date.now(),
  faultInjector = null
} = {}) {
  if (!indexedDBFactory || typeof indexedDBFactory.open !== "function") {
    throw new TypeError("IndexedDB is unavailable");
  }
  requireNonEmpty(databaseName, "filesystem database name");
  if (faultInjector != null && typeof faultInjector !== "function") {
    throw new TypeError("filesystem fault injector must be a function");
  }

  return Object.freeze({
    kind: "indexeddb",
    validate(configuration = {}) {
      validateConfiguration(configuration, databaseName);
      return true;
    },
    async open(configuration = {}) {
      const validated = validateConfiguration(configuration, databaseName);
      const database = await openDatabase(indexedDBFactory, validated.database);
      const provider = new IndexedDbFilesystem(database, {
        namespace: validated.namespace,
        quotaBytes: validated.quotaBytes,
        maxFileBytes: validated.maxFileBytes,
        now,
        faultInjector
      });
      try {
        await provider.initialize();
        return provider;
      } catch (error) {
        database.close();
        throw mapError(error, { operation: "open" });
      }
    }
  });
}

export async function openIndexedDbFilesystem(configuration, options = {}) {
  return createIndexedDbFilesystemFactory(options).open(configuration);
}

class IndexedDbFilesystem {
  #database;
  #namespace;
  #quotaBytes;
  #maxFileBytes;
  #now;
  #faultInjector;
  #closed = false;
  #mountRevision = "0";
  #activeTransactions = new Set();

  constructor(database, { namespace, quotaBytes, maxFileBytes, now, faultInjector }) {
    this.#database = database;
    this.#namespace = namespace;
    this.#quotaBytes = quotaBytes;
    this.#maxFileBytes = maxFileBytes;
    this.#now = now;
    this.#faultInjector = faultInjector;
    database.onversionchange = () => this.#invalidate("versionchange");
    database.onclose = () => this.#invalidate("close");
  }

  async initialize() {
    await this.#transaction([META_STORE, NODE_STORE], "readwrite", null, async (stores) => {
      let state = await request(stores.meta.get(stateKey(this.#namespace)));
      if (state == null) {
        state = initialState(this.#namespace, this.#quotaBytes, this.#maxFileBytes);
        stores.meta.put(state);
        stores.nodes.put(rootNode(this.#namespace, this.#now()));
      } else {
        if (state.schemaVersion > SCHEMA_VERSION) {
          throw failure(
            "unsupported",
            "filesystem schema is newer than this runtime",
            { operation: "open", providerCode: "schema-newer" }
          );
        }
        if (state.schemaVersion !== SCHEMA_VERSION) {
          throw failure(
            "unsupported",
            "filesystem schema migration is unavailable",
            { operation: "open", providerCode: "schema-migration-required" }
          );
        }
        stores.meta.put(state);
      }
      this.#mountRevision = String(state.revision);
    });
  }

  descriptor() {
    return Object.freeze({
      kind: "indexeddb",
      display: `IndexedDB:${this.#namespace}`,
      "read-only?": false,
      capabilities: CAPABILITIES,
      revision: this.#mountRevision,
      extensions: Object.freeze({ "provider/schema-version": SCHEMA_VERSION })
    });
  }

  capabilities() {
    return CAPABILITIES;
  }

  async stat(context, path) {
    path = normalizeLogicalPath(path);
    return this.#call(context, "stat", path, null, async (signal) =>
      this.#transaction([META_STORE, NODE_STORE], "readonly", signal, async (stores) => {
        const state = await this.#state(stores);
        const node = await resolveNode(stores, state, this.#namespace, path);
        return publicEntry(node, path);
      })
    );
  }

  async read(context, path) {
    path = normalizeLogicalPath(path);
    return this.#call(context, "read", path, null, async (signal) =>
      this.#transaction(
        [META_STORE, NODE_STORE, CONTENT_STORE],
        "readonly",
        signal,
        async (stores) => {
          const state = await this.#state(stores);
          const node = await resolveNode(stores, state, this.#namespace, path);
          if (node.kind === "directory") {
            throw failure("is-directory", "path is a directory", { operation: "read", path });
          }
          if (node.kind !== "file" || node.contentId == null) {
            throw failure("unsupported", "path is not a regular file", { operation: "read", path });
          }
          const content = await request(stores.contents.get(contentKey(this.#namespace, node.contentId)));
          if (content == null) {
            throw failure("io", "file content is missing", {
              operation: "read",
              path,
              providerCode: "content-missing"
            });
          }
          return copyBytes(content.bytes);
        }
      )
    );
  }

  async write(context, path, bytes, options = {}, mutation = {}) {
    path = normalizeLogicalPath(path);
    const input = copyBytes(bytes);
    const mode = options.mode ?? "replace";
    if (!new Set(["create", "replace", "append"]).has(mode)) {
      throw new TypeError("filesystem write mode must be create, replace, or append");
    }
    return this.#call(context, "write", path, null, async (signal) =>
      this.#transaction(
        [META_STORE, NODE_STORE, CONTENT_STORE],
        "readwrite",
        signal,
        async (stores) => {
          const state = await this.#state(stores);
          const parent = await ensureParent(
            stores,
            state,
            this.#namespace,
            path,
            options.parents === true,
            this.#now
          );
          let node = await childByName(stores, this.#namespace, parent.id, fileName(path));
          assertExpected(node, mutation.expectedRevision, "write", path, null);
          if (mode === "create" && node != null) {
            throw failure("already-exists", "path already exists", { operation: "write", path });
          }
          if (node?.kind === "directory") {
            throw failure("is-directory", "path is a directory", { operation: "write", path });
          }
          if (node != null && node.kind !== "file") {
            throw failure("unsupported", "path is not a regular file", { operation: "write", path });
          }

          let output = input;
          if (mode === "append" && node != null) {
            const previous = await request(
              stores.contents.get(contentKey(this.#namespace, node.contentId))
            );
            if (previous == null) {
              throw failure("io", "file content is missing", {
                operation: "write",
                path,
                providerCode: "content-missing"
              });
            }
            output = concatenate(copyBytes(previous.bytes), input);
          }
          checkFileSize(output.byteLength, this.#maxFileBytes, "write", path);
          const previousSize = node?.size ?? 0;
          checkQuota(state.bytes - previousSize + output.byteLength, this.#quotaBytes, "write", path);

          const revision = nextRevision(state);
          const id = node?.id ?? allocateNodeId(state);
          const contentId = allocateContentId(state);
          const previousContentId = node?.contentId ?? null;
          node = {
            key: nodeKey(this.#namespace, id),
            namespace: this.#namespace,
            id,
            parentId: parent.id,
            name: fileName(path),
            kind: "file",
            size: output.byteLength,
            modifiedAt: this.#now(),
            revision,
            contentId
          };
          stores.contents.put({
            key: contentKey(this.#namespace, contentId),
            namespace: this.#namespace,
            id: contentId,
            bytes: output.buffer.slice(output.byteOffset, output.byteOffset + output.byteLength)
          });
          stores.nodes.put(node);
          if (previousContentId != null) {
            stores.contents.delete(contentKey(this.#namespace, previousContentId));
          }
          state.bytes = state.bytes - previousSize + output.byteLength;
          stores.meta.put(state);
          this.#fault("write:after-records", { path, state, node });
          this.#mountRevision = String(state.revision);
          return mutationResult(path, node.revision, state.revision);
        }
      )
    );
  }

  async entriesPage(context, path, page = {}) {
    path = normalizeLogicalPath(path);
    const limit = page.limit ?? DEFAULT_PAGE_LIMIT;
    if (!Number.isSafeInteger(limit) || limit <= 0 || limit > MAX_PAGE_LIMIT) {
      throw new TypeError(`filesystem page limit must be between 1 and ${MAX_PAGE_LIMIT}`);
    }
    const after = decodePageToken(page.token ?? null);
    return this.#call(context, "entries", path, null, async (signal) =>
      this.#transaction([META_STORE, NODE_STORE], "readonly", signal, async (stores) => {
        const state = await this.#state(stores);
        const directory = await resolveNode(stores, state, this.#namespace, path);
        if (directory.kind !== "directory") {
          throw failure("not-directory", "path is not a directory", {
            operation: "entries",
            path
          });
        }
        const children = await childrenOf(stores, this.#namespace, directory.id);
        const start = after == null
          ? 0
          : children.findIndex((node) => compareTuple([node.name, node.id], after) > 0);
        const offset = start < 0 ? children.length : start;
        const selected = children.slice(offset, offset + limit);
        const entries = selected.map((node) => publicEntry(node, joinPath(path, node.name)));
        const nextToken = offset + selected.length < children.length && selected.length > 0
          ? encodePageToken([selected.at(-1).name, selected.at(-1).id])
          : null;
        return Object.freeze({ entries: Object.freeze(entries), nextToken });
      })
    );
  }

  async mkdir(context, path, options = {}, mutation = {}) {
    path = normalizeLogicalPath(path);
    return this.#call(context, "mkdir", path, null, async (signal) =>
      this.#transaction([META_STORE, NODE_STORE], "readwrite", signal, async (stores) => {
        const state = await this.#state(stores);
        const segments = pathSegments(path);
        let current = await request(stores.nodes.get(nodeKey(this.#namespace, state.rootId)));
        if (segments.length === 0) {
          assertExpected(current, mutation.expectedRevision, "mkdir", path, null);
          if (options.existsOk === false) {
            throw failure("already-exists", "mounted root already exists", {
              operation: "mkdir",
              path
            });
          }
          return mutationResult(path, current.revision, state.revision);
        }

        let created = false;
        for (let index = 0; index < segments.length; index += 1) {
          const name = segments[index];
          const final = index === segments.length - 1;
          let child = await childByName(stores, this.#namespace, current.id, name);
          if (child != null) {
            if (child.kind !== "directory") {
              throw failure("not-directory", "path component is not a directory", {
                operation: "mkdir",
                path
              });
            }
            if (final) {
              assertExpected(child, mutation.expectedRevision, "mkdir", path, null);
              if (options.existsOk === false) {
                throw failure("already-exists", "directory already exists", {
                  operation: "mkdir",
                  path
                });
              }
            }
            current = child;
            continue;
          }
          if (!final && options.parents !== true) {
            throw failure("not-found", "parent directory does not exist", {
              operation: "mkdir",
              path
            });
          }
          if (final && mutation.expectedRevision != null) {
            throw conflict("mkdir", path, null, mutation.expectedRevision, null);
          }
          child = createDirectoryNode(
            this.#namespace,
            allocateNodeId(state),
            current.id,
            name,
            nextRevision(state),
            this.#now()
          );
          stores.nodes.put(child);
          current = child;
          created = true;
        }
        if (created) {
          stores.meta.put(state);
          this.#fault("mkdir:after-records", { path, state, node: current });
          this.#mountRevision = String(state.revision);
        }
        return mutationResult(path, current.revision, state.revision);
      })
    );
  }

  async delete(context, path, options = {}, mutation = {}) {
    path = normalizeLogicalPath(path);
    return this.#call(context, "delete", path, null, async (signal) =>
      this.#transaction(
        [META_STORE, NODE_STORE, CONTENT_STORE],
        "readwrite",
        signal,
        async (stores) => {
          if (path === "/") {
            throw failure("denied", "cannot delete the mounted root", {
              operation: "delete",
              path
            });
          }
          const state = await this.#state(stores);
          let node;
          try {
            node = await resolveNode(stores, state, this.#namespace, path);
          } catch (error) {
            if (options.missingOk === true && error?.code === "file/not-found") {
              return mutationResult(path, null, state.revision);
            }
            throw error;
          }
          assertExpected(node, mutation.expectedRevision, "delete", path, null);
          if (node.kind === "directory" && (await childrenOf(stores, this.#namespace, node.id)).length > 0) {
            throw failure("directory-not-empty", "directory is not empty", {
              operation: "delete",
              path
            });
          }
          stores.nodes.delete(node.key);
          if (node.contentId != null) {
            stores.contents.delete(contentKey(this.#namespace, node.contentId));
          }
          state.bytes -= node.size ?? 0;
          nextRevision(state);
          stores.meta.put(state);
          this.#fault("delete:after-records", { path, state, node });
          this.#mountRevision = String(state.revision);
          return mutationResult(path, null, state.revision);
        }
      )
    );
  }

  async copy(context, source, target, options = {}, mutation = {}) {
    source = normalizeLogicalPath(source);
    target = normalizeLogicalPath(target);
    return this.#call(context, "copy", source, target, async (signal) =>
      this.#transaction(
        [META_STORE, NODE_STORE, CONTENT_STORE],
        "readwrite",
        signal,
        async (stores) => {
          if (source === target) {
            throw failure("already-exists", "source and target are the same path", {
              operation: "copy",
              path: source,
              target
            });
          }
          const state = await this.#state(stores);
          const sourceNode = await resolveNode(stores, state, this.#namespace, source);
          assertExpected(sourceNode, mutation.expectedRevision, "copy", source, target);
          if (sourceNode.kind === "directory") {
            throw failure("is-directory", "source is a directory", {
              operation: "copy",
              path: source,
              target
            });
          }
          if (sourceNode.kind !== "file") {
            throw failure("unsupported", "source is not a regular file", {
              operation: "copy",
              path: source,
              target
            });
          }
          const parent = await ensureParent(
            stores,
            state,
            this.#namespace,
            target,
            options.parents === true,
            this.#now
          );
          const existing = await childByName(stores, this.#namespace, parent.id, fileName(target));
          assertExpected(existing, mutation.expectedTargetRevision, "copy", source, target);
          if (existing != null && options.replace !== true) {
            throw failure("already-exists", "target already exists", {
              operation: "copy",
              path: source,
              target
            });
          }
          if (existing?.kind === "directory") {
            throw failure("is-directory", "target is a directory", {
              operation: "copy",
              path: source,
              target
            });
          }
          const sourceContent = await request(
            stores.contents.get(contentKey(this.#namespace, sourceNode.contentId))
          );
          if (sourceContent == null) {
            throw failure("io", "source content is missing", {
              operation: "copy",
              path: source,
              target,
              providerCode: "content-missing"
            });
          }
          const bytes = copyBytes(sourceContent.bytes);
          const previousSize = existing?.size ?? 0;
          checkQuota(state.bytes - previousSize + bytes.byteLength, this.#quotaBytes, "copy", target);
          const revision = nextRevision(state);
          const id = existing?.id ?? allocateNodeId(state);
          const contentId = allocateContentId(state);
          stores.contents.put({
            key: contentKey(this.#namespace, contentId),
            namespace: this.#namespace,
            id: contentId,
            bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
          });
          stores.nodes.put({
            key: nodeKey(this.#namespace, id),
            namespace: this.#namespace,
            id,
            parentId: parent.id,
            name: fileName(target),
            kind: "file",
            size: bytes.byteLength,
            modifiedAt: this.#now(),
            revision,
            contentId
          });
          if (existing?.contentId != null) {
            stores.contents.delete(contentKey(this.#namespace, existing.contentId));
          }
          state.bytes = state.bytes - previousSize + bytes.byteLength;
          stores.meta.put(state);
          this.#fault("copy:after-records", { source, target, state });
          this.#mountRevision = String(state.revision);
          return mutationResult(target, revision, state.revision);
        }
      )
    );
  }

  async move(context, source, target, options = {}, mutation = {}) {
    source = normalizeLogicalPath(source);
    target = normalizeLogicalPath(target);
    return this.#call(context, "move", source, target, async (signal) =>
      this.#transaction(
        [META_STORE, NODE_STORE, CONTENT_STORE],
        "readwrite",
        signal,
        async (stores) => {
          if (source === "/" || target === "/") {
            throw failure("denied", "cannot move the mounted root", {
              operation: "move",
              path: source,
              target
            });
          }
          const state = await this.#state(stores);
          const sourceNode = await resolveNode(stores, state, this.#namespace, source);
          assertExpected(sourceNode, mutation.expectedRevision, "move", source, target);
          if (source === target) {
            return mutationResult(target, sourceNode.revision, state.revision);
          }
          if (sourceNode.kind === "directory" && target.startsWith(`${source}/`)) {
            throw failure("invalid-path", "cannot move a directory beneath itself", {
              operation: "move",
              path: source,
              target
            });
          }
          const parent = await ensureParent(
            stores,
            state,
            this.#namespace,
            target,
            options.parents === true,
            this.#now
          );
          const existing = await childByName(stores, this.#namespace, parent.id, fileName(target));
          assertExpected(existing, mutation.expectedTargetRevision, "move", source, target);
          if (existing != null && options.replace !== true) {
            throw failure("already-exists", "target already exists", {
              operation: "move",
              path: source,
              target
            });
          }
          if (existing != null) {
            if (existing.kind !== sourceNode.kind) {
              throw failure("already-exists", "target has a different entry type", {
                operation: "move",
                path: source,
                target
              });
            }
            if (existing.kind === "directory" &&
                (await childrenOf(stores, this.#namespace, existing.id)).length > 0) {
              throw failure("directory-not-empty", "target directory is not empty", {
                operation: "move",
                path: source,
                target
              });
            }
            stores.nodes.delete(existing.key);
            if (existing.contentId != null) {
              stores.contents.delete(contentKey(this.#namespace, existing.contentId));
            }
            state.bytes -= existing.size ?? 0;
          }
          const revision = nextRevision(state);
          sourceNode.parentId = parent.id;
          sourceNode.name = fileName(target);
          sourceNode.modifiedAt = this.#now();
          sourceNode.revision = revision;
          stores.nodes.put(sourceNode);
          stores.meta.put(state);
          this.#fault("move:after-records", { source, target, state, node: sourceNode });
          this.#mountRevision = String(state.revision);
          return mutationResult(target, revision, state.revision);
        }
      )
    );
  }

  async close(context = null) {
    if (this.#closed) return;
    if (context?.signal?.aborted) {
      throw failure("cancelled", "filesystem close was cancelled", { operation: "close" });
    }
    this.#invalidate("explicit-close");
  }

  async #state(stores) {
    const state = await request(stores.meta.get(stateKey(this.#namespace)));
    if (state == null) {
      throw failure("io", "filesystem metadata is missing", {
        providerCode: "metadata-missing"
      });
    }
    return state;
  }

  async #call(context, operation, path, target, run) {
    this.#ensureOpen(operation, path, target);
    const call = prepareCallContext(context, operation, path, target);
    try {
      return await run(call.signal);
    } catch (error) {
      throw mapError(error, {
        operation,
        path,
        target,
        cancelled: call.signal?.aborted,
        timeout: call.timedOut()
      });
    } finally {
      call.dispose();
    }
  }

  async #transaction(storeNames, mode, signal, body) {
    this.#ensureOpen(null, null, null);
    if (signal?.aborted) {
      throw failure("cancelled", "filesystem operation cancelled");
    }
    let transaction;
    try {
      transaction = this.#database.transaction(storeNames, mode);
    } catch (error) {
      throw mapError(error, {});
    }
    this.#activeTransactions.add(transaction);
    const stores = {
      meta: storeNames.includes(META_STORE) ? transaction.objectStore(META_STORE) : null,
      nodes: storeNames.includes(NODE_STORE) ? transaction.objectStore(NODE_STORE) : null,
      contents: storeNames.includes(CONTENT_STORE) ? transaction.objectStore(CONTENT_STORE) : null
    };
    const done = transactionDone(transaction);
    const abort = () => {
      try {
        transaction.abort();
      } catch {
        // Completion or another abort already made the transaction terminal.
      }
    };
    signal?.addEventListener("abort", abort, { once: true });
    try {
      const result = await body(stores, transaction);
      await done;
      return result;
    } catch (error) {
      abort();
      try {
        await done;
      } catch {
        // Preserve the operation-specific failure rather than the generic abort.
      }
      throw error;
    } finally {
      signal?.removeEventListener("abort", abort);
      this.#activeTransactions.delete(transaction);
    }
  }

  #ensureOpen(operation, path, target) {
    if (this.#closed) {
      throw failure("provider-closed", "filesystem provider is closed", {
        operation,
        path,
        target
      });
    }
  }

  #invalidate(_reason) {
    if (this.#closed) return;
    this.#closed = true;
    for (const transaction of this.#activeTransactions) {
      try {
        transaction.abort();
      } catch {
        // The transaction is already terminal.
      }
    }
    this.#activeTransactions.clear();
    this.#database.close();
  }

  #fault(stage, details) {
    this.#faultInjector?.(stage, details);
  }
}

function validateConfiguration(configuration, defaultDatabase) {
  if (configuration == null || typeof configuration !== "object" || Array.isArray(configuration)) {
    throw new TypeError("filesystem configuration must be a map");
  }
  const database = requireNonEmpty(configuration.database ?? defaultDatabase, "filesystem database");
  const namespace = requireNonEmpty(configuration.namespace, "filesystem namespace");
  const schemaVersion = configuration.version ?? SCHEMA_VERSION;
  if (schemaVersion !== SCHEMA_VERSION) {
    throw new TypeError(`unsupported IndexedDB filesystem schema version: ${schemaVersion}`);
  }
  const quotaBytes = positiveSafeInteger(
    configuration.quotaBytes ?? DEFAULT_QUOTA_BYTES,
    "filesystem quota"
  );
  const maxFileBytes = positiveSafeInteger(
    configuration.maxFileBytes ?? Math.min(DEFAULT_MAX_FILE_BYTES, quotaBytes),
    "filesystem maximum file size"
  );
  if (maxFileBytes > quotaBytes) {
    throw new TypeError("filesystem maximum file size exceeds its quota");
  }
  return { database, namespace, quotaBytes, maxFileBytes, schemaVersion };
}

async function openDatabase(indexedDBFactory, name) {
  return new Promise((resolve, reject) => {
    let request_;
    try {
      request_ = indexedDBFactory.open(name, SCHEMA_VERSION);
    } catch (error) {
      reject(mapError(error, { operation: "open" }));
      return;
    }
    request_.onupgradeneeded = () => {
      const database = request_.result;
      if (!database.objectStoreNames.contains(META_STORE)) {
        database.createObjectStore(META_STORE, { keyPath: "key" });
      }
      if (!database.objectStoreNames.contains(NODE_STORE)) {
        const nodes = database.createObjectStore(NODE_STORE, { keyPath: "key" });
        nodes.createIndex(PARENT_NAME_INDEX, ["namespace", "parentId", "name"], { unique: true });
        nodes.createIndex(PARENT_INDEX, ["namespace", "parentId", "name", "id"], { unique: false });
      }
      if (!database.objectStoreNames.contains(CONTENT_STORE)) {
        database.createObjectStore(CONTENT_STORE, { keyPath: "key" });
      }
    };
    request_.onsuccess = () => resolve(request_.result);
    request_.onerror = () => reject(mapError(request_.error, { operation: "open" }));
    request_.onblocked = () => reject(failure(
      "io",
      "filesystem database upgrade is blocked",
      { operation: "open", providerCode: "upgrade-blocked", retryable: true }
    ));
  });
}

function initialState(namespace, quotaBytes, maxFileBytes) {
  return {
    key: stateKey(namespace),
    namespace,
    schemaVersion: SCHEMA_VERSION,
    rootId: "root",
    nextNode: 1,
    nextContent: 1,
    revision: 0,
    bytes: 0,
    quotaBytes,
    maxFileBytes
  };
}

function rootNode(namespace, now) {
  return {
    key: nodeKey(namespace, "root"),
    namespace,
    id: "root",
    parentId: null,
    name: "",
    kind: "directory",
    size: null,
    modifiedAt: now,
    revision: "0",
    contentId: null
  };
}

function createDirectoryNode(namespace, id, parentId, name, revision, modifiedAt) {
  return {
    key: nodeKey(namespace, id),
    namespace,
    id,
    parentId,
    name,
    kind: "directory",
    size: null,
    modifiedAt,
    revision,
    contentId: null
  };
}

async function resolveNode(stores, state, namespace, path) {
  let node = await request(stores.nodes.get(nodeKey(namespace, state.rootId)));
  if (node == null) {
    throw failure("io", "filesystem root is missing", { providerCode: "root-missing" });
  }
  for (const segment of pathSegments(path)) {
    if (node.kind !== "directory") {
      throw failure("not-directory", "path ancestor is not a directory", { path });
    }
    node = await childByName(stores, namespace, node.id, segment);
    if (node == null) {
      throw failure("not-found", "path does not exist", { path });
    }
  }
  return node;
}

async function ensureParent(stores, state, namespace, path, parents, now) {
  const segments = pathSegments(parentPath(path));
  let node = await request(stores.nodes.get(nodeKey(namespace, state.rootId)));
  for (const segment of segments) {
    let child = await childByName(stores, namespace, node.id, segment);
    if (child == null) {
      if (!parents) {
        throw failure("not-found", "parent directory does not exist", { path });
      }
      child = createDirectoryNode(
        namespace,
        allocateNodeId(state),
        node.id,
        segment,
        nextRevision(state),
        now()
      );
      stores.nodes.put(child);
    } else if (child.kind !== "directory") {
      throw failure("not-directory", "path ancestor is not a directory", { path });
    }
    node = child;
  }
  return node;
}

async function childByName(stores, namespace, parentId, name) {
  return request(stores.nodes.index(PARENT_NAME_INDEX).get([namespace, parentId, name]));
}

async function childrenOf(stores, namespace, parentId) {
  const lower = [namespace, parentId, "", ""];
  const upper = [namespace, parentId, "\uffff", "\uffff"];
  const range = IDBKeyRange.bound(lower, upper);
  const nodes = await request(stores.nodes.index(PARENT_INDEX).getAll(range));
  nodes.sort((left, right) => compareTuple([left.name, left.id], [right.name, right.id]));
  return nodes;
}

function publicEntry(node, path) {
  return Object.freeze({
    path,
    name: node.name,
    type: node.kind,
    size: node.kind === "file" ? node.size : null,
    "modified-at": node.modifiedAt ?? null,
    extensions: Object.freeze({
      "file/id": node.id,
      "file/revision": String(node.revision)
    })
  });
}

function mutationResult(path, revision, mountRevision) {
  return Object.freeze({
    path,
    revision: revision == null ? null : String(revision),
    "mount-revision": String(mountRevision),
    extensions: Object.freeze({})
  });
}

function allocateNodeId(state) {
  const id = `node-${state.nextNode}`;
  state.nextNode += 1;
  return id;
}

function allocateContentId(state) {
  const id = `content-${state.nextContent}`;
  state.nextContent += 1;
  return id;
}

function nextRevision(state) {
  state.revision += 1;
  return String(state.revision);
}

function assertExpected(node, expected, operation, path, target) {
  if (expected == null) return;
  const actual = node == null ? null : String(node.revision);
  if (actual !== String(expected)) {
    throw conflict(operation, path, target, expected, actual);
  }
}

function conflict(operation, path, target, expected, actual) {
  return failure("conflict", "filesystem revision does not match", {
    operation,
    path,
    target,
    providerCode: actual == null ? "revision-missing" : "revision-mismatch",
    retryable: true,
    expectedRevision: expected,
    actualRevision: actual
  });
}

function checkFileSize(size, maximum, operation, path) {
  if (size > maximum) {
    throw failure("quota-exceeded", "file exceeds the configured size limit", {
      operation,
      path,
      providerCode: "file-size-limit"
    });
  }
}

function checkQuota(size, maximum, operation, path) {
  if (size > maximum) {
    throw failure("quota-exceeded", "filesystem quota would be exceeded", {
      operation,
      path,
      providerCode: "mount-quota"
    });
  }
}

function prepareCallContext(context) {
  const sourceSignal = context?.signal ?? null;
  const timeoutMs = context?.timeoutMs ?? null;
  if (sourceSignal != null && typeof sourceSignal.addEventListener !== "function") {
    throw new TypeError("filesystem call signal must be an AbortSignal");
  }
  if (timeoutMs != null && (!Number.isFinite(timeoutMs) || timeoutMs < 0)) {
    throw new TypeError("filesystem timeout must be a non-negative number");
  }
  if (sourceSignal == null && timeoutMs == null) {
    return { signal: null, timedOut: () => false, dispose() {} };
  }
  const controller = new AbortController();
  let timeout = false;
  const abortFromSource = () => controller.abort(sourceSignal.reason);
  if (sourceSignal?.aborted) abortFromSource();
  else sourceSignal?.addEventListener("abort", abortFromSource, { once: true });
  const timer = timeoutMs == null ? null : setTimeout(() => {
    timeout = true;
    controller.abort(new DOMException("filesystem operation timed out", "TimeoutError"));
  }, timeoutMs);
  return {
    signal: controller.signal,
    timedOut: () => timeout,
    dispose() {
      if (timer != null) clearTimeout(timer);
      sourceSignal?.removeEventListener("abort", abortFromSource);
    }
  };
}

function transactionDone(transaction) {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error ?? new DOMException("aborted", "AbortError"));
    transaction.onerror = () => reject(transaction.error ?? new DOMException("failed", "UnknownError"));
  });
}

function request(operation) {
  return new Promise((resolve, reject) => {
    operation.onsuccess = () => resolve(operation.result ?? null);
    operation.onerror = () => reject(operation.error ?? new DOMException("failed", "UnknownError"));
  });
}

export function normalizeLogicalPath(input) {
  if (typeof input !== "string") throw new TypeError("logical path must be a string");
  if (input.includes("\0")) throw failure("invalid-path", "logical path contains NUL");
  if (input.includes("\\")) {
    throw failure("invalid-path", "logical paths use '/' rather than host separators");
  }
  const output = [];
  for (const segment of input.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (output.length === 0) {
        throw failure("outside-root", "logical path escapes above the mounted root");
      }
      output.pop();
      continue;
    }
    if (/^[A-Za-z]:/.test(segment)) {
      throw failure("invalid-path", "logical paths do not accept host drive prefixes");
    }
    output.push(segment);
  }
  return output.length === 0 ? "/" : `/${output.join("/")}`;
}

function pathSegments(path) {
  path = normalizeLogicalPath(path);
  return path === "/" ? [] : path.slice(1).split("/");
}

function parentPath(path) {
  path = normalizeLogicalPath(path);
  if (path === "/") return null;
  const separator = path.lastIndexOf("/");
  return separator === 0 ? "/" : path.slice(0, separator);
}

function fileName(path) {
  path = normalizeLogicalPath(path);
  return path === "/" ? "" : path.slice(path.lastIndexOf("/") + 1);
}

function joinPath(parent, name) {
  return normalizeLogicalPath(`${parent === "/" ? "" : parent}/${name}`);
}

function stateKey(namespace) {
  return `${namespace}:state`;
}

function nodeKey(namespace, id) {
  return `${namespace}:node:${id}`;
}

function contentKey(namespace, id) {
  return `${namespace}:content:${id}`;
}

function encodePageToken(tuple) {
  return `v1:${encodeURIComponent(JSON.stringify(tuple))}`;
}

function decodePageToken(token) {
  if (token == null) return null;
  if (typeof token !== "string" || !token.startsWith("v1:")) {
    throw new TypeError("invalid filesystem page token");
  }
  try {
    const value = JSON.parse(decodeURIComponent(token.slice(3)));
    if (!Array.isArray(value) || value.length !== 2 || value.some((item) => typeof item !== "string")) {
      throw new Error("invalid token tuple");
    }
    return value;
  } catch (error) {
    throw new TypeError("invalid filesystem page token", { cause: error });
  }
}

function compareTuple(left, right) {
  if (left[0] < right[0]) return -1;
  if (left[0] > right[0]) return 1;
  if (left[1] < right[1]) return -1;
  if (left[1] > right[1]) return 1;
  return 0;
}

function copyBytes(value) {
  if (value instanceof Uint8Array) return new Uint8Array(value);
  if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
  }
  throw new TypeError("filesystem contents must be bytes");
}

function concatenate(left, right) {
  const output = new Uint8Array(left.byteLength + right.byteLength);
  output.set(left, 0);
  output.set(right, left.byteLength);
  return output;
}

function positiveSafeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) throw new TypeError(`${label} must be positive`);
  return value;
}

function requireNonEmpty(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new TypeError(`${label} is required`);
  return value;
}

function failure(code, message, metadata = {}) {
  return new FilesystemProviderError(code, message, metadata);
}

function mapError(error, metadata) {
  if (error instanceof FilesystemProviderError) {
    if (error.operation == null) error.operation = metadata.operation ?? null;
    if (error.path == null) error.path = metadata.path ?? null;
    if (error.target == null) error.target = metadata.target ?? null;
    return error;
  }
  if (metadata.timeout) {
    return failure("timeout", "filesystem operation timed out", {
      ...metadata,
      providerCode: "TimeoutError",
      retryable: true,
      cause: error
    });
  }
  if (metadata.cancelled) {
    return failure("cancelled", "filesystem operation cancelled", {
      ...metadata,
      providerCode: "AbortError",
      cause: error
    });
  }
  const name = error?.name ?? "UnknownError";
  const mapping = {
    NotFoundError: ["not-found", false],
    ConstraintError: ["already-exists", false],
    QuotaExceededError: ["quota-exceeded", false],
    InvalidStateError: ["provider-closed", false],
    VersionError: ["unsupported", false],
    AbortError: ["io", true],
    TransactionInactiveError: ["io", false],
    UnknownError: ["io", true]
  }[name] ?? ["io", false];
  return failure(mapping[0], "IndexedDB filesystem operation failed", {
    ...metadata,
    providerCode: String(name),
    retryable: mapping[1],
    cause: error
  });
}
