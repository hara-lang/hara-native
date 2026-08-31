package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Process-local host values used by the Hara-owned {@code std.lang} compiler pipeline.
 *
 * <p>This class deliberately owns identity, immutable value boundaries, and reversible lifecycle
 * state only. Grammar callbacks, lowering, emission, and target-specific behavior remain guest
 * Hara values stored in the configuration maps; the host never invokes or interprets them.
 */
final class HaraNativeLang {
  private static final String NAMESPACE = "std.lang";

  private HaraNativeLang() {}

  static void install(HaraContext context) {
    for (hara.lang.declaration.HaraNativeBinding binding : HaraNativeDeclarations.bindings()) {
      if (!NAMESPACE.equals(binding.namespace())) continue;
      for (String method : HaraNativeDeclarations.methods(binding)) {
        HaraNativeLibrary.function(
            context,
            binding.namespace() + "." + binding.name(),
            method,
            (ignored, arguments) -> invoke(binding.name(), method, arguments),
            "",
            new String[0]);
      }
    }
  }

  static Object invoke(String type, String operation, Object[] values) {
    return switch (type) {
      case "BookMeta", "BookEntry", "BookModule", "Book", "Compilation" ->
          immutable(type, operation, values, "data");
      case "Compiler" -> immutable(type, operation, values, "config");
      case "Library" -> library(operation, values);
      case "Snapshot" -> snapshot(operation, values);
      case "Runtime" -> runtime(operation, values);
      case "Harness" -> harness(operation, values);
      default -> throw failure(type, operation, "is not installed");
    };
  }

  private static Object immutable(String type, String operation, Object[] values, String accessor) {
    return switch (operation) {
      case "create" -> {
        requireArity(type, operation, values, 1);
        yield new ImmutableValue(type, config(values[0], type + "/create"));
      }
      default -> {
        if (!accessor.equals(operation)) throw failure(type, operation, "is not installed");
        requireArity(type, operation, values, 1);
        ImmutableValue value = immutableValue(values[0], type, operation);
        yield value.data;
      }
    };
  }

  private static Object library(String operation, Object[] values) {
    return switch (operation) {
      case "create" -> {
        requireArity("Library", operation, values, 1);
        yield new Library(config(values[0], "Library/create"));
      }
      case "config" -> {
        requireArity("Library", operation, values, 1);
        yield libraryValue(values[0], operation).config;
      }
      case "install" -> {
        requireArity("Library", operation, values, 2);
        Library library = libraryValue(values[0], operation);
        ImmutableValue book = immutableValue(values[1], "Book", operation);
        library.books.put(bookKey(book.data, operation), book);
        library.revision++;
        yield values[0];
      }
      case "remove" -> {
        requireArity("Library", operation, values, 2);
        Library library = libraryValue(values[0], operation);
        Object removed = library.books.remove(bookKeyArgument(values[1], operation));
        if (removed != null) library.revision++;
        yield removed;
      }
      case "resolve" -> {
        requireArity("Library", operation, values, 2);
        yield libraryValue(values[0], operation).books.get(bookKeyArgument(values[1], operation));
      }
      case "books" -> {
        requireArity("Library", operation, values, 1);
        yield new ArrayList<>(libraryValue(values[0], operation).books.values());
      }
      case "snapshot" -> {
        requireArity("Library", operation, values, 1);
        Library library = libraryValue(values[0], operation);
        yield Snapshot.library(library);
      }
      case "restore" -> {
        requireArity("Library", operation, values, 2);
        Library library = libraryValue(values[0], operation);
        Snapshot snapshot = snapshotValue(values[1], operation);
        if (snapshot.owner != library || snapshot.kind != SnapshotKind.LIBRARY) {
          throw failure("Library", operation, "requires a snapshot from the same Library");
        }
        library.restore(snapshot.books, snapshot.libraryRevision);
        yield values[0];
      }
      case "reset" -> {
        requireArity("Library", operation, values, 1);
        Library library = libraryValue(values[0], operation);
        library.restore(Map.of(), 0);
        yield values[0];
      }
      case "state" -> {
        requireArity("Library", operation, values, 1);
        yield libraryState(libraryValue(values[0], operation));
      }
      default -> throw failure("Library", operation, "is not installed");
    };
  }

  private static Object snapshot(String operation, Object[] values) {
    if (!"data".equals(operation)) throw failure("Snapshot", operation, "is not installed");
    requireArity("Snapshot", operation, values, 1);
    return snapshotValue(values[0], operation).data();
  }

  private static Object runtime(String operation, Object[] values) {
    return switch (operation) {
      case "create" -> {
        requireArity("Runtime", operation, values, 1);
        yield new Runtime(config(values[0], "Runtime/create"));
      }
      case "config" -> {
        requireArity("Runtime", operation, values, 1);
        yield runtimeValue(values[0], operation).config;
      }
      case "state" -> {
        requireArity("Runtime", operation, values, 1);
        yield runtimeState(runtimeValue(values[0], operation));
      }
      case "reset" -> {
        requireArity("Runtime", operation, values, 1);
        Runtime runtime = runtimeValue(values[0], operation);
        runtime.reset();
        yield values[0];
      }
      case "close" -> {
        requireArity("Runtime", operation, values, 1);
        Runtime runtime = runtimeValue(values[0], operation);
        runtime.close();
        yield values[0];
      }
      case "closed?" -> {
        requireArity("Runtime", operation, values, 1);
        yield runtimeValue(values[0], operation).closed;
      }
      default -> throw failure("Runtime", operation, "is not installed");
    };
  }

  private static Object harness(String operation, Object[] values) {
    return switch (operation) {
      case "create" -> {
        requireArity("Harness", operation, values, 1);
        Object config = config(values[0], "Harness/create");
        yield new Harness(
            config,
            optionalLibrary(config, "library", operation),
            optionalRuntime(config, "runtime", operation));
      }
      case "config" -> {
        requireArity("Harness", operation, values, 1);
        yield harnessValue(values[0], operation).config;
      }
      case "library" -> {
        requireArity("Harness", operation, values, 1);
        yield harnessValue(values[0], operation).library;
      }
      case "runtime" -> {
        requireArity("Harness", operation, values, 1);
        yield harnessValue(values[0], operation).runtime;
      }
      case "snapshot" -> {
        requireArity("Harness", operation, values, 1);
        Harness harness = harnessValue(values[0], operation);
        yield Snapshot.harness(harness);
      }
      case "restore" -> {
        requireArity("Harness", operation, values, 2);
        Harness harness = harnessValue(values[0], operation);
        Snapshot snapshot = snapshotValue(values[1], operation);
        if (snapshot.owner != harness || snapshot.kind != SnapshotKind.HARNESS) {
          throw failure("Harness", operation, "requires a snapshot from the same Harness");
        }
        harness.library.restore(snapshot.books, snapshot.libraryRevision);
        harness.runtime.restore(snapshot.runtimeClosed, snapshot.runtimeRevision);
        harness.closed = snapshot.harnessClosed;
        yield values[0];
      }
      case "reset" -> {
        requireArity("Harness", operation, values, 1);
        Harness harness = harnessValue(values[0], operation);
        harness.library.restore(Map.of(), 0);
        harness.runtime.reset();
        harness.closed = false;
        yield values[0];
      }
      case "close" -> {
        requireArity("Harness", operation, values, 1);
        Harness harness = harnessValue(values[0], operation);
        harness.closed = true;
        harness.runtime.close();
        yield values[0];
      }
      case "closed?" -> {
        requireArity("Harness", operation, values, 1);
        yield harnessValue(values[0], operation).closed;
      }
      case "state" -> {
        requireArity("Harness", operation, values, 1);
        yield harnessState(harnessValue(values[0], operation));
      }
      default -> throw failure("Harness", operation, "is not installed");
    };
  }

  private static Object optionalLibrary(Object config, String name, String operation) {
    Object value = lookup(config, name);
    if (value == null) return new Library(emptyMap());
    return libraryValue(value, "Harness/" + operation);
  }

  private static Object optionalRuntime(Object config, String name, String operation) {
    Object value = lookup(config, name);
    if (value == null) return new Runtime(emptyMap());
    return runtimeValue(value, "Harness/" + operation);
  }

  private static Object libraryState(Library library) {
    return map(
        "revision", library.revision,
        "book-count", (long) library.books.size(),
        "books", new ArrayList<>(library.books.values()));
  }

  private static Object runtimeState(Runtime runtime) {
    return map("state", Keyword.create(runtime.closed ? "closed" : "ready"), "revision", runtime.revision);
  }

  private static Object harnessState(Harness harness) {
    return map(
        "state", Keyword.create(harness.closed ? "closed" : "ready"),
        "library", libraryState(harness.library),
        "runtime", runtimeState(harness.runtime));
  }

  private static Object map(Object... values) {
    LinkedHashMap<Object, Object> output = new LinkedHashMap<>();
    for (int index = 0; index < values.length; index += 2) {
      output.put(Keyword.create((String) values[index]), values[index + 1]);
    }
    return HaraPersistentValues.normalize(output);
  }

  private static Object emptyMap() {
    return HaraPersistentValues.normalize(Map.of());
  }

  private static Object config(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof IMapType<?, ?>)) {
      throw new HaraException("std.lang." + operation + " expects a configuration map");
    }
    return raw;
  }

  private static Object lookup(Object config, String name) {
    return HaraBox.unwrap(HaraContext.lookupValue((IMapType<?, ?>) config, Keyword.create(name)));
  }

  private static Object bookKey(Object config, String operation) {
    Object coordinate = lookup(config, "coordinate");
    if (coordinate != null) return coordinate;
    throw failure("Library", operation, "requires Book :coordinate");
  }

  private static Object bookKeyArgument(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof ImmutableValue book && "Book".equals(book.type)) return bookKey(book.data, operation);
    if (raw instanceof IMapType<?, ?>) return bookKey(raw, operation);
    return raw;
  }

  private static ImmutableValue immutableValue(Object value, String type, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof ImmutableValue immutable && type.equals(immutable.type)) return immutable;
    throw failure(type, operation, "expects a std.lang." + type + " value");
  }

  private static Library libraryValue(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Library library) return library;
    throw failure("Library", operation, "expects a std.lang.Library value");
  }

  private static Snapshot snapshotValue(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Snapshot snapshot) return snapshot;
    throw failure("Snapshot", operation, "expects a std.lang.Snapshot value");
  }

  private static Runtime runtimeValue(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Runtime runtime) return runtime;
    throw failure("Runtime", operation, "expects a std.lang.Runtime value");
  }

  private static Harness harnessValue(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Harness harness) return harness;
    throw failure("Harness", operation, "expects a std.lang.Harness value");
  }

  private static void requireArity(String type, String operation, Object[] values, int expected) {
    if (values.length != expected) {
      throw new HaraException(
          "std.lang." + type + "/" + operation + " expects " + expected + " argument"
              + (expected == 1 ? "" : "s"));
    }
  }

  private static HaraException failure(String type, String operation, String message) {
    return new HaraException("std.lang." + type + "/" + operation + " " + message);
  }

  static final class ImmutableValue {
    private final String type;
    private final Object data;

    private ImmutableValue(String type, Object data) {
      this.type = type;
      this.data = data;
    }

    String type() {
      return type;
    }
  }

  static final class Library {
    private final Object config;
    private final LinkedHashMap<Object, ImmutableValue> books = new LinkedHashMap<>();
    private long revision;

    private Library(Object config) {
      this.config = config;
    }

    private void restore(Map<Object, ImmutableValue> books, long revision) {
      this.books.clear();
      this.books.putAll(books);
      this.revision = revision;
    }
  }

  private enum SnapshotKind {
    LIBRARY,
    HARNESS
  }

  static final class Snapshot {
    private final SnapshotKind kind;
    private final Object owner;
    private final LinkedHashMap<Object, ImmutableValue> books;
    private final long libraryRevision;
    private final boolean runtimeClosed;
    private final long runtimeRevision;
    private final boolean harnessClosed;

    private Snapshot(
        SnapshotKind kind,
        Object owner,
        Map<Object, ImmutableValue> books,
        long libraryRevision,
        boolean runtimeClosed,
        long runtimeRevision,
        boolean harnessClosed) {
      this.kind = kind;
      this.owner = owner;
      this.books = new LinkedHashMap<>(books);
      this.libraryRevision = libraryRevision;
      this.runtimeClosed = runtimeClosed;
      this.runtimeRevision = runtimeRevision;
      this.harnessClosed = harnessClosed;
    }

    private static Snapshot library(Library library) {
      return new Snapshot(
          SnapshotKind.LIBRARY,
          library,
          library.books,
          library.revision,
          false,
          0,
          false);
    }

    private static Snapshot harness(Harness harness) {
      return new Snapshot(
          SnapshotKind.HARNESS,
          harness,
          harness.library.books,
          harness.library.revision,
          harness.runtime.closed,
          harness.runtime.revision,
          harness.closed);
    }

    private Object data() {
      return map(
          "kind", Keyword.create(kind == SnapshotKind.LIBRARY ? "library" : "harness"),
          "library-revision", libraryRevision,
          "books", new ArrayList<>(books.values()),
          "runtime-closed?", runtimeClosed,
          "runtime-revision", runtimeRevision,
          "harness-closed?", harnessClosed);
    }
  }

  static final class Runtime {
    private final Object config;
    private boolean closed;
    private long revision;

    private Runtime(Object config) {
      this.config = config;
    }

    private void reset() {
      closed = false;
      revision = 0;
    }

    private void close() {
      closed = true;
    }

    private void restore(boolean closed, long revision) {
      this.closed = closed;
      this.revision = revision;
    }
  }

  static final class Harness {
    private final Object config;
    private final Library library;
    private final Runtime runtime;
    private boolean closed;

    private Harness(Object config, Object library, Object runtime) {
      this.config = config;
      this.library = (Library) library;
      this.runtime = (Runtime) runtime;
    }
  }
}
