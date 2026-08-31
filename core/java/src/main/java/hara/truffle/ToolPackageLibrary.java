package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.truffle.bytecode.HbxBundleCodec;
import java.util.ArrayList;
import java.util.List;

/** Portable HBX0 package provider shared with the Rust runtime. */
public final class ToolPackageLibrary {
  private ToolPackageLibrary() {}

  static void install(HaraContext context, String namespace) {
    HaraNativeLibrary.function(context, namespace, "provider", (ignored, arguments) -> provider(namespace, arguments),
        "Returns exact HBX0 package capabilities.", "[]");
    HaraNativeLibrary.function(context, namespace, "validate", (ignored, arguments) -> validate(namespace, arguments),
        "Authenticates and validates HBX0 and every nested HBC0 module.", "[bytes]");
    HaraNativeLibrary.function(context, namespace, "inspect", (ignored, arguments) -> inspect(namespace, arguments),
        "Returns portable HBX0 package metadata.", "[bytes]");
    HaraNativeLibrary.function(context, namespace, "pack", (ignored, arguments) -> pack(namespace, arguments),
        "Creates a deterministic HBX0 package from module descriptors.", "[modules]");
    HaraNativeLibrary.function(context, namespace, "unpack", (ignored, arguments) -> unpack(namespace, arguments),
        "Returns validated HBX0 module descriptors.", "[bytes]");
  }

  private static Object provider(String namespace, Object[] arguments) {
    expectArity(namespace, "provider", arguments, 0);
    return map("provider/id", keyword("rust"),
        "provider/operations", keywords("validate", "inspect", "pack", "unpack", "conform"),
        "provider/formats", map("hbx", keywords("validate", "inspect", "pack", "unpack", "conform")));
  }

  private static Object validate(String namespace, Object[] arguments) {
    expectArity(namespace, "validate", arguments, 1);
    HbxBundleCodec.decode(bytes(namespace, arguments[0], "validate"));
    return Boolean.TRUE;
  }

  private static Object inspect(String namespace, Object[] arguments) {
    expectArity(namespace, "inspect", arguments, 1);
    var modules = HbxBundleCodec.decode(bytes(namespace, arguments[0], "inspect"));
    ArrayList<String> resources = new ArrayList<>();
    for (var module : modules) resources.add(module.resource());
    return map("package/format", keyword("hbx"), "package/version", 0L,
        "modules/count", (long) modules.size(), "modules/resources", vector(resources.toArray()));
  }

  private static Object pack(String namespace, Object[] arguments) {
    expectArity(namespace, "pack", arguments, 1);
    return HbxBundleCodec.encode(modules(namespace, arguments[0]));
  }

  private static Object unpack(String namespace, Object[] arguments) {
    expectArity(namespace, "unpack", arguments, 1);
    var modules = HbxBundleCodec.decode(bytes(namespace, arguments[0], "unpack"));
    Object[] values = new Object[modules.size()];
    for (int index = 0; index < modules.size(); index++) {
      var module = modules.get(index);
      values[index] = map("module/resource", module.resource(),
          "module/namespace-form", module.namespaceForm(),
          "module/source-digest", module.sourceDigest(),
          "module/dependencies", vector(module.dependencies().toArray()),
          "module/eager", module.eager(), "module/artifact", module.artifact());
    }
    return vector(values);
  }

  private static List<HbxBundleCodec.Module> modules(String namespace, Object value) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof ILinearType<?> items)) {
      throw new HaraException(namespace + "/pack expects a vector of modules");
    }
    ArrayList<HbxBundleCodec.Module> modules = new ArrayList<>(Math.toIntExact(items.count()));
    for (int index = 0; index < items.count(); index++) {
      modules.add(module(namespace, items.nth(index)));
    }
    return List.copyOf(modules);
  }

  @SuppressWarnings("rawtypes")
  private static HbxBundleCodec.Module module(String namespace, Object value) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof IMapType map)) {
      throw new HaraException(namespace + "/pack expects module descriptors as maps");
    }
    String resource = string(namespace, map.lookup(keyword("module/resource")), ":module/resource");
    String namespaceForm =
        string(namespace, map.lookup(keyword("module/namespace-form")), ":module/namespace-form");
    byte[] sourceDigest =
        bytes(namespace, required(namespace, map.lookup(keyword("module/source-digest")), ":module/source-digest"), "pack");
    if (sourceDigest.length != 32) {
      throw new HaraException(namespace + "/pack expects a 32-byte :module/source-digest");
    }
    Object dependenciesValue = map.lookup(keyword("module/dependencies"));
    Object dependenciesRaw = HaraBox.unwrap(dependenciesValue);
    if (!(dependenciesRaw instanceof ILinearType<?> dependencies)) {
      throw new HaraException(namespace + "/pack expects :module/dependencies as a vector");
    }
    ArrayList<String> dependencyNames = new ArrayList<>(Math.toIntExact(dependencies.count()));
    for (int index = 0; index < dependencies.count(); index++) {
      Object dependency = HaraBox.unwrap(dependencies.nth(index));
      if (!(dependency instanceof String text)) {
        throw new HaraException(namespace + "/pack expects String dependencies");
      }
      dependencyNames.add(text);
    }
    Object eager = HaraBox.unwrap(map.lookup(keyword("module/eager")));
    if (!(eager instanceof Boolean eagerValue)) {
      throw new HaraException(namespace + "/pack expects :module/eager as a boolean");
    }
    byte[] artifact =
        bytes(namespace, required(namespace, map.lookup(keyword("module/artifact")), ":module/artifact"), "pack");
    return new HbxBundleCodec.Module(
        resource, namespaceForm, sourceDigest, dependencyNames, eagerValue, artifact);
  }

  private static Object required(String namespace, Object value, String field) {
    if (value == null || value == HaraNull.SINGLETON) {
      throw new HaraException(namespace + "/pack requires " + field);
    }
    return value;
  }

  private static String string(String namespace, Object value, String field) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof String text && !text.isEmpty()) return text;
    throw new HaraException(namespace + "/pack expects " + field + " as a non-empty String");
  }

  private static byte[] bytes(String namespace, Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof byte[] bytes) return bytes.clone();
    throw new HaraException(namespace + "/" + operation + " expects Bytes");
  }
  private static void expectArity(String namespace, String operation, Object[] arguments, int arity) {
    if (arguments.length != arity) throw new HaraException(namespace + "/" + operation + " expects " + arity + " arguments");
  }
  private static Keyword keyword(String value) { return Keyword.create(value); }
  private static Object vector(Object... values) { return hara.lang.data.Vector.Standard.from(null, values); }
  private static Object keywords(String... values) {
    Object[] out = new Object[values.length];
    for (int index = 0; index < values.length; index++) out[index] = keyword(values[index]);
    return vector(out);
  }
  private static Object map(Object... entries) {
    Object[] values = new Object[entries.length];
    for (int index = 0; index < entries.length; index += 2) {
      values[index] = keyword((String) entries[index]); values[index + 1] = entries[index + 1];
    }
    return hara.lang.data.OrderedMap.Standard.from(null, values);
  }
}
