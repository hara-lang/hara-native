package hara.truffle;

import hara.lang.data.Keyword;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;

/** Projects provider-neutral filesystem values onto the existing public Hara data shape. */
final class FilesystemHaraValues {
  private FilesystemHaraValues() {}

  static Object entry(IFilesystem.Entry entry) {
    ArrayList<Object> extensions = new ArrayList<>();
    entry.extensions().entrySet().stream()
        .sorted(Map.Entry.comparingByKey())
        .forEach(
            value -> {
              extensions.add(keyword(value.getKey()));
              extensions.add(HaraPersistentValues.normalize(value.getValue()));
            });
    if (entry.id() != null) {
      extensions.add(Keyword.create("file", "id"));
      extensions.add(entry.id());
    }
    if (entry.revision() != null) {
      extensions.add(Keyword.create("file", "revision"));
      extensions.add(entry.revision());
    }
    if (entry.capabilities() != null) {
      Object[] capabilities =
          entry.capabilities().values().stream()
              .sorted(Comparator.comparing(IFilesystem.Capability::keyword))
              .map(capability -> Keyword.create(capability.keyword()))
              .toArray();
      extensions.add(Keyword.create("provider", "capabilities"));
      extensions.add(hara.lang.data.Set.Standard.from(null, capabilities));
    }
    return hara.lang.data.Map.Standard.from(
        null,
        Keyword.create("path"),
        entry.path(),
        Keyword.create("name"),
        entry.name(),
        Keyword.create("type"),
        Keyword.create(entry.type().keyword()),
        Keyword.create("size"),
        entry.size(),
        Keyword.create("modified-at"),
        entry.modifiedAt(),
        Keyword.create("extensions"),
        hara.lang.data.Map.Standard.from(null, extensions.toArray()));
  }

  static Object entries(List<IFilesystem.Entry> entries) {
    return hara.lang.data.Vector.Standard.from(
        null, entries.stream().map(FilesystemHaraValues::entry).toArray());
  }

  static Object paths(List<String> paths) {
    return hara.lang.data.Vector.Standard.from(null, paths.toArray());
  }

  private static Keyword keyword(String value) {
    int separator = value.indexOf('/');
    if (separator > 0 && separator < value.length() - 1) {
      return Keyword.create(value.substring(0, separator), value.substring(separator + 1));
    }
    return Keyword.create(value);
  }
}
