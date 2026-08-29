package hara.truffle;

import java.util.Collections;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.TreeSet;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;

/** Kernel-owned trusted filesystem-factory registry. */
final class FilesystemProviderRegistry {
  private final ConcurrentHashMap<String, IFilesystemFactory> factories =
      new ConcurrentHashMap<>();

  FilesystemProviderRegistry register(IFilesystemFactory factory) {
    Objects.requireNonNull(factory, "filesystem factory");
    String kind = requireKind(factory.kind());
    IFilesystemFactory previous = factories.putIfAbsent(kind, factory);
    if (previous != null && previous != factory) {
      throw new IllegalArgumentException("FILESYSTEM_PROVIDER_EXISTS " + kind);
    }
    return this;
  }

  boolean unregister(IFilesystemFactory factory) {
    Objects.requireNonNull(factory, "filesystem factory");
    return factories.entrySet().removeIf(entry -> entry.getValue() == factory);
  }

  boolean contains(String kind) {
    return factories.containsKey(requireKind(kind));
  }

  Set<String> kinds() {
    return Collections.unmodifiableSet(new TreeSet<>(factories.keySet()));
  }

  CompletionStage<IFilesystem> open(
      String kind, IFilesystemFactory.OpenContext context, Map<String, ?> configuration) {
    String normalized = requireKind(kind);
    IFilesystemFactory factory = factories.get(normalized);
    if (factory == null) {
      return CompletableFuture.failedFuture(
          new IllegalArgumentException("FILESYSTEM_PROVIDER_NOT_FOUND " + normalized));
    }
    Map<String, ?> frozen = Map.copyOf(Objects.requireNonNull(configuration, "configuration"));
    try {
      factory.validate(frozen);
      return Objects.requireNonNull(
          factory.open(Objects.requireNonNull(context, "open context"), frozen),
          "filesystem factory open stage");
    } catch (Throwable error) {
      return CompletableFuture.failedFuture(error);
    }
  }

  private static String requireKind(String kind) {
    if (kind == null || kind.isBlank() || !kind.matches("[a-z][a-z0-9-]*")) {
      throw new IllegalArgumentException("invalid filesystem provider kind");
    }
    return kind;
  }
}
