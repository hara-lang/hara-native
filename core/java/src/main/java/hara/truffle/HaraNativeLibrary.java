package hara.truffle;

import hara.lang.block.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.IMetadata;
import java.util.ArrayList;

/** Registers native functions explicitly, without reflective host annotations. */
final class HaraNativeLibrary {
  @FunctionalInterface
  interface NativeFunction {
    Object apply(HaraContext context, Object[] arguments);
  }

  private HaraNativeLibrary() {}

  static void function(
      HaraContext context,
      String namespace,
      String name,
      NativeFunction implementation,
      String doc,
      String... arglists) {
    context.defineNativeFunction(
        namespace,
        name,
        arguments ->
            HaraPersistentValues.normalize(implementation.apply(context, arguments)),
        metadata(doc, arglists));
  }

  private static IMetadata metadata(String doc, String[] arglists) {
    ArrayList<Object> entries = new ArrayList<>();
    if (!doc.isEmpty()) {
      entries.add(Keyword.create("doc"));
      entries.add(doc);
    }
    if (arglists.length > 0) {
      ArrayList<Object> parsed = new ArrayList<>();
      for (String arglist : arglists) parsed.add(Parser.parseString(arglist));
      entries.add(Keyword.create("arglists"));
      entries.add(hara.lang.data.List.Standard.from(null, parsed.toArray()));
    }
    return entries.isEmpty()
        ? hara.lang.data.Map.Standard.EMPTY
        : hara.lang.data.Map.Standard.from(null, entries.toArray());
  }
}
