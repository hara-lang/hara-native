package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import hara.lang.data.Keyword;
import hara.lang.data.Map;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Verifies the reversible, process-local host-value substrate for std.lang. */
public class HaraNativeLangTest {
  @Test
  public void annotatedLangTypesDoNotExpandTheClosedNativeInventory() {
    assertEquals(
        "std.lang.Book",
        HaraNativeDeclarations.qualifiedName(HaraNativeDeclarations.binding("std.lang", "Book")));
    assertFalse(HaraNativeDeclarations.METHODS.containsKey("Book"));

    HaraNativeType descriptor =
        new HaraNativeType("std.lang", "Book", java.util.List.of("create", "data"));
    assertEquals("std.lang", descriptor.getNamespace());
    assertEquals("#<native-type std.lang.Book>", descriptor.display());
  }

  @Test
  public void librarySnapshotsRestoreAnExactBookBaseline() {
    Object book =
        HaraNativeLang.invoke(
            "Book",
            "create",
            new Object[] {Map.Standard.from(null, Keyword.create("coordinate"), Symbol.create("demo"))});
    Object library = HaraNativeLang.invoke("Library", "create", new Object[] {Map.Standard.EMPTY});

    assertSame(library, HaraNativeLang.invoke("Library", "install", new Object[] {library, book}));
    Object snapshot = HaraNativeLang.invoke("Library", "snapshot", new Object[] {library});
    assertSame(book, HaraNativeLang.invoke("Library", "remove", new Object[] {library, Symbol.create("demo")}));
    assertEquals(0L, stateLong(library, "book-count"));

    assertSame(
        library, HaraNativeLang.invoke("Library", "restore", new Object[] {library, snapshot}));
    assertSame(book, HaraNativeLang.invoke("Library", "resolve", new Object[] {library, Symbol.create("demo")}));
    assertEquals(1L, stateLong(library, "book-count"));
    assertEquals(1L, stateLong(library, "revision"));
  }

  @Test
  public void libraryRejectsBooksWithoutVersionedCoordinates() {
    Object book =
        HaraNativeLang.invoke(
            "Book",
            "create",
            new Object[] {Map.Standard.from(null, Keyword.create("id"), Symbol.create("demo"))});
    Object library = HaraNativeLang.invoke("Library", "create", new Object[] {Map.Standard.EMPTY});

    try {
      HaraNativeLang.invoke("Library", "install", new Object[] {library, book});
      fail("expected an explicit Book coordinate requirement");
    } catch (HaraException expected) {
      assertTrue(expected.getMessage().contains("Book :coordinate"));
    }
  }

  @Test
  public void harnessResetAndCloseAreIdempotent() {
    Object harness = HaraNativeLang.invoke("Harness", "create", new Object[] {Map.Standard.EMPTY});

    assertFalse((Boolean) HaraNativeLang.invoke("Harness", "closed?", new Object[] {harness}));
    assertSame(harness, HaraNativeLang.invoke("Harness", "close", new Object[] {harness}));
    assertSame(harness, HaraNativeLang.invoke("Harness", "close", new Object[] {harness}));
    assertTrue((Boolean) HaraNativeLang.invoke("Harness", "closed?", new Object[] {harness}));
    assertSame(harness, HaraNativeLang.invoke("Harness", "reset", new Object[] {harness}));
    assertFalse((Boolean) HaraNativeLang.invoke("Harness", "closed?", new Object[] {harness}));
    assertEquals(0L, stateLong(HaraNativeLang.invoke("Harness", "library", new Object[] {harness}), "book-count"));
  }

  @Test
  public void qualifiedLangSurfaceIsCallableWithoutCreatingGlobalAliases() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[true 1 false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [book (std.lang.Book/create {:coordinate 'demo/book}) "
                      + "library (std.lang.Library/create {}) "
                      + "_ (std.lang.Library/install library book) "
                      + "snapshot (std.lang.Library/snapshot library) "
                      + "_ (std.lang.Library/remove library 'demo/book) "
                      + "_ (std.lang.Library/restore library snapshot) "
                      + "harness (std.lang.Harness/create {}) "
                      + "_ (std.lang.Harness/close harness) "
                      + "_ (std.lang.Harness/reset harness)] "
                      + "[(std.native.Base/instance? std.lang.Book book) "
                      + "(std.protocol.ilookup.ILookup/lookup "
                      + "(std.lang.Library/state library) :book-count) "
                      + "(std.lang.Harness/closed? harness)])")
              .toString());
    }
  }

  @SuppressWarnings("rawtypes")
  private static long stateLong(Object library, String name) {
    IMapType state = (IMapType) HaraNativeLang.invoke("Library", "state", new Object[] {library});
    return ((Number) state.lookup(Keyword.create(name))).longValue();
  }
}
