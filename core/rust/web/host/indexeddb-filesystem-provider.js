import {
  FilesystemProviderError,
  createIndexedDbFilesystemFactory as createRawIndexedDbFilesystemFactory,
  normalizeLogicalPath
} from "./indexeddb-filesystem.js";

export { FilesystemProviderError, normalizeLogicalPath };

/**
 * Public IndexedDB provider factory.
 *
 * The raw storage implementation is wrapped here so every advertised option is
 * either honored or rejected before a transaction begins. This prevents a
 * backend from silently weakening portable filesystem semantics.
 */
export function createIndexedDbFilesystemFactory(options = {}) {
  const factory = createRawIndexedDbFilesystemFactory(options);
  return Object.freeze({
    kind: "indexeddb",
    validate(configuration = {}) {
      return factory.validate(configuration);
    },
    async open(configuration = {}) {
      return guardFilesystem(await factory.open(configuration));
    }
  });
}

export async function openIndexedDbFilesystem(configuration, options = {}) {
  return createIndexedDbFilesystemFactory(options).open(configuration);
}

function guardFilesystem(filesystem) {
  return Object.freeze({
    descriptor: () => filesystem.descriptor(),
    capabilities: () => filesystem.capabilities(),
    stat: (...args) => filesystem.stat(...args),
    read: (...args) => filesystem.read(...args),
    write: (...args) => filesystem.write(...args),
    entriesPage: (...args) => filesystem.entriesPage(...args),
    mkdir: (...args) => filesystem.mkdir(...args),
    delete: (...args) => filesystem.delete(...args),

    copy(context, source, target, options = {}, mutation = {}) {
      if (options.preserveModified === true) {
        return Promise.reject(
          unsupported(
            "copy",
            source,
            target,
            "preserve-modified-unavailable",
            "IndexedDB cannot preserve a provider-supplied modification time"
          )
        );
      }
      return filesystem.copy(context, source, target, options, mutation);
    },

    move(context, source, target, options = {}, mutation = {}) {
      if (options.atomic === true) {
        return Promise.reject(
          unsupported(
            "move",
            source,
            target,
            "atomic-move-unavailable",
            "IndexedDB does not expose the portable atomic-move capability"
          )
        );
      }
      return filesystem.move(context, source, target, options, mutation);
    },

    close: (...args) => filesystem.close(...args)
  });
}

function unsupported(operation, path, target, providerCode, message) {
  return new FilesystemProviderError("unsupported", message, {
    operation,
    path,
    target,
    providerCode,
    retryable: false
  });
}
