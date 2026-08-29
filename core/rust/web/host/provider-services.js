import { HtaKeyword } from "../packages/hta/index.js";
import { createIndexedDbFilesystemHost } from "./indexeddb-filesystem-host.js";
import { normalizeLogicalPath } from "./indexeddb-filesystem.js";
import {
  createHostDescription,
  createHostServices as createCompatibilityHostServices
} from "./services.js";

const DEFAULT_DATABASE = "hara-studio";
const MEMORY_CAPABILITIES = Object.freeze([
  "read", "write", "entries", "mkdir", "delete"
]);

export { createHostDescription };

/**
 * Composes the stable host-service families with provider-neutral filesystem
 * routing. Existing memory mounts retain their compact compatibility provider;
 * IndexedDB mounts use the transactional provider and trusted factory.
 */
export function createHostServices(options = {}) {
  const compatibility = createCompatibilityHostServices(options);
  const compatibilityFilesystem = compatibility.filesystemHost;
  const routes = new WeakMap();
  let indexedDbFilesystem = null;

  function indexedDbHost() {
    indexedDbFilesystem ??= createIndexedDbFilesystemHost({
      factory: options.indexedDbFilesystemFactory,
      indexedDB: options.indexedDB,
      databaseName:
        options.filesystemDatabaseName ??
        `${options.dbName ?? DEFAULT_DATABASE}-filesystems`,
      now: options.filesystemNow,
      faultInjector: options.filesystemFaultInjector
    });
    return indexedDbFilesystem;
  }

  function routesFor(context, create = false) {
    if (!context || (typeof context !== "object" && typeof context !== "function")) {
      throw new TypeError("filesystem kernel context must be an object");
    }
    let contextRoutes = routes.get(context);
    if (!contextRoutes && create) {
      contextRoutes = new Map();
      routes.set(context, contextRoutes);
    }
    return contextRoutes;
  }

  function requireRoute(context, mountId) {
    const provider = routesFor(context)?.get(mountId);
    if (!provider) throw new Error(`file/mount-closed:${mountId}`);
    return provider;
  }

  const filesystemHost = Object.freeze({
    async register(context, mountId, descriptor = {}) {
      const provider = descriptor.provider ?? descriptor.kind ?? "memory";
      if (provider === "indexeddb") {
        await indexedDbHost().register(context, mountId, descriptor);
      } else if (provider === "memory") {
        await compatibilityFilesystem.register(context, mountId, descriptor);
      } else {
        throw new Error(`filesystem/provider-unsupported:${provider}`);
      }
      routesFor(context, true).set(mountId, provider);
      return true;
    },

    descriptor(context, mountId) {
      const provider = requireRoute(context, mountId);
      if (provider === "indexeddb") return indexedDbHost().descriptor(context, mountId);
      return Object.freeze({
        kind: "memory",
        display: "Memory filesystem",
        "read-only?": false,
        capabilities: MEMORY_CAPABILITIES,
        revision: null,
        extensions: Object.freeze({})
      });
    },

    capabilities(context, mountId) {
      return requireRoute(context, mountId) === "indexeddb"
        ? indexedDbHost().capabilities(context, mountId)
        : MEMORY_CAPABILITIES;
    },

    invoke(context, mountId, method, args = [], callContext = null) {
      return requireRoute(context, mountId) === "indexeddb"
        ? indexedDbHost().invoke(context, mountId, method, args, callContext)
        : compatibilityFilesystem.invoke(context, mountId, method, args);
    },

    async close(context, mountId) {
      const provider = requireRoute(context, mountId);
      if (provider === "indexeddb") {
        await indexedDbHost().close(context, mountId);
      } else {
        await compatibilityFilesystem.close(context, mountId);
      }
      const contextRoutes = routesFor(context);
      contextRoutes.delete(mountId);
      if (contextRoutes.size === 0) routes.delete(context);
      return true;
    },

    async closeContext(context) {
      const contextRoutes = routesFor(context);
      if (!contextRoutes) return 0;
      const entries = [...contextRoutes.entries()];
      routes.delete(context);
      await Promise.allSettled(entries.map(([mountId, provider]) =>
        provider === "indexeddb"
          ? indexedDbHost().close(context, mountId)
          : compatibilityFilesystem.close(context, mountId)
      ));
      return entries.length;
    }
  });

  const services = { ...compatibility };
  services["file/read"] = function(path) {
    requireFilesystemGrant(options, this);
    return filesystemHost.invoke(this.kernelContext, this.mountId, "read", [path]);
  };
  services["file/write"] = async function(path, bytes, value = null) {
    requireFilesystemGrant(options, this);
    const parsed = optionMap(value);
    const result = await filesystemHost.invoke(this.kernelContext, this.mountId, "write", [
      path,
      bytes,
      {
        mode: keywordName(parsed.mode) ?? "replace",
        parents: booleanOption(parsed, "parents?", false)
      },
      mutationOptions(parsed)
    ]);
    return result?.path ?? normalizeLogicalPath(path);
  };
  services["file/exists"] = function(path) {
    requireFilesystemGrant(options, this);
    return filesystemHost.invoke(this.kernelContext, this.mountId, "exists", [path]);
  };
  services["file/exists?"] = services["file/exists"];
  services["file/stat"] = async function(path) {
    requireFilesystemGrant(options, this);
    return entryToHta(await filesystemHost.invoke(
      this.kernelContext, this.mountId, "stat", [path]
    ));
  };
  services["file/entries"] = async function(path) {
    requireFilesystemGrant(options, this);
    return (await filesystemHost.invoke(this.kernelContext, this.mountId, "entries", [path]))
      .map(entryToHta);
  };
  services["file/list"] = function(path) {
    requireFilesystemGrant(options, this);
    return filesystemHost.invoke(this.kernelContext, this.mountId, "list", [path]);
  };
  services["file/walk"] = function(path) {
    requireFilesystemGrant(options, this);
    return filesystemHost.invoke(this.kernelContext, this.mountId, "walk", [path]);
  };
  services["file/mkdir"] = async function(path, value = null) {
    requireFilesystemGrant(options, this);
    const parsed = optionMap(value);
    const result = await filesystemHost.invoke(this.kernelContext, this.mountId, "mkdir", [
      path,
      {
        parents: booleanOption(parsed, "parents?", true),
        existsOk: booleanOption(parsed, "exists-ok?", true)
      },
      mutationOptions(parsed)
    ]);
    return result?.path ?? normalizeLogicalPath(path);
  };
  services["file/delete"] = async function(path, value = null) {
    requireFilesystemGrant(options, this);
    const parsed = optionMap(value);
    const result = await filesystemHost.invoke(this.kernelContext, this.mountId, "delete", [
      path,
      { missingOk: booleanOption(parsed, "missing-ok?", false) },
      mutationOptions(parsed)
    ]);
    return result?.path ?? normalizeLogicalPath(path);
  };
  services["file/copy"] = async function(source, target, value = null) {
    requireFilesystemGrant(options, this);
    const parsed = optionMap(value);
    const result = await filesystemHost.invoke(this.kernelContext, this.mountId, "copy", [
      source,
      target,
      {
        replace: booleanOption(parsed, "replace?", false),
        parents: booleanOption(parsed, "parents?", false),
        preserveModified: booleanOption(parsed, "preserve-modified?", false)
      },
      mutationOptions(parsed)
    ]);
    return result.path;
  };
  services["file/move"] = async function(source, target, value = null) {
    requireFilesystemGrant(options, this);
    const parsed = optionMap(value);
    const result = await filesystemHost.invoke(this.kernelContext, this.mountId, "move", [
      source,
      target,
      {
        replace: booleanOption(parsed, "replace?", false),
        parents: booleanOption(parsed, "parents?", false),
        atomic: booleanOption(parsed, "atomic?", false)
      },
      mutationOptions(parsed)
    ]);
    return result.path;
  };

  Object.defineProperty(services, "filesystemHost", {
    value: filesystemHost,
    enumerable: false
  });
  return services;
}

function requireFilesystemGrant(options, invocation) {
  let requested;
  if (typeof options.grantsForSession === "function") {
    requested = options.grantsForSession(invocation?.sessionId ?? "ROOT");
  } else if (options.grantedCapabilities != null) {
    requested = options.grantedCapabilities;
  } else {
    return;
  }
  const granted = new Set(
    [...(requested ?? [])]
      .map((value) => String(value?.name ?? value).replace(/^:/, ""))
      .filter(Boolean)
  );
  if (!granted.has("filesystem")) {
    throw new Error("host/capability-denied:filesystem");
  }
}

function optionMap(value) {
  if (value == null) return {};
  if (value instanceof Map) {
    return Object.fromEntries([...value].map(([key, entry]) => [
      String(key?.name ?? key).replace(/^:/, ""),
      entry
    ]));
  }
  if (typeof value === "object" && !Array.isArray(value)) return { ...value };
  throw new TypeError("filesystem options must be a map");
}

function booleanOption(options, key, fallback) {
  const value = options[key];
  if (value == null) return fallback;
  if (typeof value !== "boolean") throw new TypeError(`filesystem option ${key} must be boolean`);
  return value;
}

function keywordName(value) {
  if (value == null) return null;
  return String(value?.name ?? value).replace(/^:/, "");
}

function mutationOptions(options) {
  return {
    expectedRevision: options["expected-revision"] ?? null,
    expectedTargetRevision: options["expected-target-revision"] ?? null
  };
}

function entryToHta(entry) {
  return new Map([
    [new HtaKeyword("path"), entry.path],
    [new HtaKeyword("name"), entry.name],
    [new HtaKeyword("type"), new HtaKeyword(entry.type)],
    [new HtaKeyword("size"), entry.size],
    [new HtaKeyword("modified-at"), entry["modified-at"]],
    [new HtaKeyword("extensions"), new Map(
      Object.entries(entry.extensions ?? {}).map(([key, value]) => [new HtaKeyword(key), value])
    )]
  ]);
}
