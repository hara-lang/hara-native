package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.util.Set;
import java.util.ServiceLoader;
import java.util.stream.Collectors;
import java.util.stream.StreamSupport;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdPortableLibraryProvidersTest {
  @Test
  public void portableNamespacesDoNotRequireProviders() {
    Set<String> namespaces =
        StreamSupport.stream(
                ServiceLoader.load(HaraLibraryProvider.class).spliterator(), false)
            .map(HaraLibraryProvider::namespace)
            .collect(Collectors.toSet());

    assertFalse(namespaces.contains("std.lib.zip"));
    assertFalse(namespaces.contains("std.lib.collection"));
    assertFalse(namespaces.contains("std.block"));
    assertFalse(namespaces.contains("std.logic.kanren"));
    assertFalse(namespaces.contains("code.test"));
    assertFalse(namespaces.contains("std.foundation.string"));
    assertFalse(namespaces.contains("std.foundation.bytes"));
    assertFalse(namespaces.contains("std.foundation.promise"));
    assertFalse(namespaces.contains("std.foundation.coroutine"));
    assertTrue(namespaces.contains("std.native.String"));
    assertTrue(namespaces.contains("std.native.Bytes"));
    assertTrue(namespaces.contains("std.native.Promise"));
    assertTrue(namespaces.contains("std.native.Coroutine"));
  }

  @Test
  public void canonicalPortableLibrariesLoadAndExecuteTogether() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[[1 3] \"[1 3]\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns portable.provider-probe "
                      + "(:require [std.lib.zip :as zip] "
                      + "[std.block :as block])) "
                      + "[(zip/result "
                      + "(zip/replace-right "
                      + "(zip/step-right (zip/step-inside (zip/vector-zip [1 2]))) 3)) "
                      + "(block/string (block/parse-first \"[1 3]\"))]")
              .toString());
    }
  }
}
