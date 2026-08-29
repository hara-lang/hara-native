package hara.truffle;

import hara.lang.data.Keyword;
import hara.truffle.bytecode.HbxBundleCodec;
import java.util.ArrayList;

/** Read-only HBX0 provider for Truffle; package construction remains Rust-owned. */
public final class ToolPackageLibrary {
  private ToolPackageLibrary() {}

  static void install(HaraContext context, String namespace) {
    HaraNativeLibrary.function(context, namespace, "provider", ToolPackageLibrary::provider,
        "Returns exact HBX0 package capabilities.", "[]");
    HaraNativeLibrary.function(context, namespace, "validate", ToolPackageLibrary::validate,
        "Authenticates and validates HBX0 and every nested HBC0 module.", "[bytes]");
    HaraNativeLibrary.function(context, namespace, "inspect", ToolPackageLibrary::inspect,
        "Returns portable HBX0 package metadata.", "[bytes]");
    HaraNativeLibrary.function(context, namespace, "unpack", ToolPackageLibrary::unpack,
        "Returns validated HBX0 module descriptors.", "[bytes]");
  }

  public static Object provider(HaraContext context, Object[] arguments) {
    expectArity("provider", arguments, 0);
    return map("provider/id", keyword("truffle"),
        "provider/operations", keywords("validate", "inspect", "unpack", "conform"),
        "provider/formats", map("hbx", keywords("validate", "inspect", "unpack", "conform")));
  }

  public static Object validate(HaraContext context, Object[] arguments) {
    expectArity("validate", arguments, 1);
    HbxBundleCodec.decode(bytes(arguments[0], "validate"));
    return Boolean.TRUE;
  }

  public static Object inspect(HaraContext context, Object[] arguments) {
    expectArity("inspect", arguments, 1);
    var modules = HbxBundleCodec.decode(bytes(arguments[0], "inspect"));
    ArrayList<String> resources = new ArrayList<>();
    for (var module : modules) resources.add(module.resource());
    return map("package/format", keyword("hbx"), "package/version", 0L,
        "modules/count", (long) modules.size(), "modules/resources", vector(resources.toArray()));
  }

  public static Object unpack(HaraContext context, Object[] arguments) {
    expectArity("unpack", arguments, 1);
    var modules = HbxBundleCodec.decode(bytes(arguments[0], "unpack"));
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

  private static byte[] bytes(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof byte[] bytes) return bytes.clone();
    throw new HaraException("tool.package.provider/" + operation + " expects Bytes");
  }
  private static void expectArity(String operation, Object[] arguments, int arity) {
    if (arguments.length != arity) throw new HaraException("tool.package.provider/" + operation + " expects " + arity + " arguments");
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
