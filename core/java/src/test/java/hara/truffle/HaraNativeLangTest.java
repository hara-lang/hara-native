package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import hara.lang.data.Keyword;
import hara.lang.data.Map;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

/** Verifies the reversible, process-local host-value substrate for std.lang. */
public class HaraNativeLangTest {
  @Test
  public void runtimeInternBindsLiveValues() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "(ns intern.contract) (def ^{:doc \"baseline\"} slot nil) (def source 19)");
      assertEquals(42, context.eval(HaraLanguage.ID,
          "(let [offset 40 value {:callback (fn [x] (+ offset x)) :source (var source)}]"
          + " (try (let [published (Runtime/intern 'intern.contract 'slot value)]"
          + " ((:callback (IDeref/deref published)) 2))"
          + " (finally (Runtime/intern 'intern.contract 'slot nil))))").asInt());
      assertTrue(context.eval(HaraLanguage.ID, "slot").isNull());
      assertEquals(19, context.eval(HaraLanguage.ID,
          "(ns intern.referrer (:require [intern.contract :refer [source]]))"
          + " (Runtime/intern 'intern.referrer 'source 88)"
          + " (IDeref/deref (var intern.contract/source))").asInt());
      assertEquals(88, context.eval(HaraLanguage.ID,
          "(IDeref/deref (var intern.referrer/source))").asInt());
      context.eval(HaraLanguage.ID, "(ns intern.contract)");
      assertEquals("script entry", context.eval(HaraLanguage.ID,
          "(let [before (IObjType/meta (var slot))]"
          + " (try (let [published (Runtime/intern 'intern.contract"
          + " (IObjType/with-meta 'slot {:doc \"script entry\" :arglists '([x])}) 42)]"
          + " (Runtime/intern 'intern.contract 'slot 43) (:doc (IObjType/meta published)))"
          + " (finally (Runtime/intern 'intern.contract (IObjType/with-meta 'slot before) nil))))").asString());
      assertEquals("baseline", context.eval(HaraLanguage.ID,
          "(:doc (IObjType/meta (var slot)))").asString());
      assertTrue(context.eval(HaraLanguage.ID,
          "(try (let [published (Runtime/intern 'intern.contract 'slot (var source))]"
          + " (= (IDeref/deref published) (var source)))"
          + " (finally (Runtime/intern 'intern.contract 'slot nil)))").asBoolean());
      assertEquals(43, context.eval(HaraLanguage.ID,
          "(try (Runtime/intern 'intern.contract 'slot 42)"
          + " (Runtime/intern 'intern.contract 'slot 43) (IDeref/deref (var slot))"
          + " (finally (Runtime/intern 'intern.contract 'slot nil)))").asInt());
      for (String invalid : new String[] {
          "(Runtime/intern 'intern.contract 'other/slot 1)",
          "(Runtime/intern 'intern.contract :slot 1)",
          "(Runtime/intern 'intern.contract 'slot)",
          "(Runtime/intern-var 'intern.contract 'slot 1)"}) {
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, invalid));
      }
      assertTrue(context.eval(HaraLanguage.ID, "slot").isNull());
    }
  }

  @Test
  public void macroInvocationPreservesCallMetadata() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID,
          "(defmacro capture-call-meta [] (Base/list 'quote (:- (IObjType/meta &form))))");
      assertEquals(":raw", context.eval(HaraLanguage.ID,
          "^{:- :raw} (capture-call-meta)").toString());
      assertEquals(":raw", context.eval(HaraLanguage.ID,
          "(let [x 1] ^{:- :raw} (capture-call-meta))").toString());
      context.eval(HaraLanguage.ID,
          "(defn captured-call [] ^{:- :raw} (capture-call-meta))");
      assertEquals(":raw", context.eval(HaraLanguage.ID, "(captured-call)").toString());
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (capture-call-meta))").asBoolean());
      assertEquals("(quote :raw)", context.eval(HaraLanguage.ID,
          "(Runtime/macroexpand-1 '^{:- :raw} (capture-call-meta))").toString());
    }
  }

  @Test
  public void cacheIdentityDistinguishesEqualMapsAndRecords() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID,
          "(def IdentityEntry (Base/struct (Base/current-namespace) 'IdentityEntry '[value] nil))");
      assertEquals("[true false true true false true true true false false false]",
          context.eval(HaraLanguage.ID,
              "(let [a {:value 1} b {:value 1} r (IdentityEntry 1) s (IdentityEntry 1) "
              + "saved (Base/atom [a r])] "
              + "[(Base/identical? a a) (Base/identical? a b) (= a b) "
              + "(Base/identical? r r) (Base/identical? r s) (= r s) "
              + "(Base/identical? a (INth/nth (IDeref/deref saved) 0)) "
              + "(Base/identical? r (INth/nth (IDeref/deref saved) 1)) "
              + "(Base/identical? a (IObjType/with-meta a {:doc \"new\"})) "
              + "(Base/identical? a (IAssoc/assoc a :value 2)) "
              + "(Base/identical? a r)])").toString());
      assertTrue(assertThrows(PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(Base/identical? 1 1)"))
          .getMessage().contains("expects map or record objects"));
      assertTrue(assertThrows(PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(Base/identical? {})"))
          .getMessage().contains("expects two objects"));
    }
  }

  @Test
  public void iobjtypeMetadataRetainsLiveRecordsAndCallbacks() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID,
          "(def ParentSnapshot (Base/struct (Base/current-namespace) 'ParentSnapshot '[book] nil))");
      assertEquals("[true true 42 43 nil true]", context.eval(HaraLanguage.ID,
          "(let [offset 40 callback (fn [x] (+ offset x)) "
          + "parent (ParentSnapshot {:emit callback}) original {:value 1} "
          + "child (IObjType/with-meta original {:parent parent :assign/fn callback}) "
          + "metadata (IObjType/meta child) restored (:parent metadata)] "
          + "[(= parent restored) (= (Base/type parent) (Base/type restored)) "
          + "((:emit (:book restored)) 2) ((:assign/fn metadata) 3) "
          + "(IObjType/meta original) (= child original)])").toString());
      assertEquals("[(a :as [1 2 3]) true]", context.eval(HaraLanguage.ID,
          "(let [callback (fn [sym] (list sym :as [1 2 3])) "
          + "form (IObjType/with-meta '(sym :as [1 2 3]) {:assign/fn callback})] "
          + "[((:assign/fn (IObjType/meta form)) 'a) "
          + "(= form '(sym :as [1 2 3]))])").toString());
    }
  }

  @Test
  public void evaluatesCapturedFunctionsInNamespaceAndRestoresOnFailure() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID,
          "(ns scope.target) (def value 99) (ns scope.inner) (def value 100) "
          + "(ns scope.caller) (def value 7) "
          + "(defn observe [] [(Runtime/current) value (Runtime/eval 'value)])");
      assertEquals("[[42 [scope.target 7 99] [scope.inner 7 100] [scope.target 7 99]] scope.caller 7]",
          context.eval(HaraLanguage.ID,
              "(let [captured (Base/atom 42)] "
              + "[(Runtime/eval-in 'scope.target (fn [] [(IDeref/deref captured) (observe) "
              + "(Runtime/eval-in 'scope.inner (fn [] (observe))) (observe)])) "
              + "(Runtime/current) value])").toString());
      assertEquals("[nil false scope.caller]", context.eval(HaraLanguage.ID,
          "[(Runtime/eval-in 'scope.target (fn [] nil)) "
          + "(Runtime/eval-in 'scope.target (fn [] false)) (Runtime/current)]").toString());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID,
          "(Runtime/eval-in 'scope.target (fn [x] x))"));
      assertEquals("scope.caller", context.eval(HaraLanguage.ID, "(Runtime/current)").toString());
      assertEquals("[:caught scope.target]", context.eval(HaraLanguage.ID,
          "(Runtime/eval-in 'scope.target (fn [] "
          + "[(try (Runtime/eval-in 'scope.inner (fn [] (throw :scope-failed))) "
          + "(catch e :caught)) (Runtime/current)]))").toString());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID,
          "(Runtime/eval-in 'scope.target (fn [] (throw :scope-failed)))"));
      assertEquals("scope.caller", context.eval(HaraLanguage.ID, "(Runtime/current)").toString());
      assertEquals(99, context.eval(HaraLanguage.ID,
          "(Runtime/eval-in 'scope.target '[value])").asInt());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID,
          "(let [isolated 42] (Runtime/eval-in 'scope.target '[isolated]))"));
      assertEquals("scope.caller", context.eval(HaraLanguage.ID, "(Runtime/current)").toString());
    }
  }

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

  @Test
  public void nativeEdnReadsSpannedFormsWithoutEvaluation() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [node (std.protocol.inth.INth/nth"
                      + "              (std.native.Edn/read-forms-spanned \"(subject/run (stop))\") 0)"
                      + "      start (std.protocol.ilookup.ILookup/lookup node :start)"
                      + "      end (std.protocol.ilookup.ILookup/lookup node :end)"
                      + "      children (std.protocol.ilookup.ILookup/lookup node :children)"
                      + "      inner (std.protocol.inth.INth/nth children 1)"
                      + "      stop (std.protocol.inth.INth/nth"
                      + "            (std.protocol.ilookup.ILookup/lookup inner :children) 0)]"
                      + " (= [(std.protocol.ilookup.ILookup/lookup start :offset)"
                      + "     (std.protocol.ilookup.ILookup/lookup end :offset)"
                      + "     (std.protocol.ilookup.ILookup/lookup"
                      + "       (std.protocol.ilookup.ILookup/lookup"
                      + "         (std.protocol.inth.INth/nth children 0) :start) :offset)"
                      + "     (std.protocol.ilookup.ILookup/lookup"
                      + "       (std.protocol.ilookup.ILookup/lookup stop :start) :offset)]"
                      + "    [0 20 1 14]))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [node (std.protocol.inth.INth/nth"
                      + "              (std.native.Edn/read-forms-spanned"
                      + "                \"^{:refer demo.subject/run}\\n(Test/check [])\") 0)"
                      + "      children (std.protocol.ilookup.ILookup/lookup node :children)"
                      + "      metadata (std.protocol.inth.INth/nth children 0)]"
                      + " (= [(std.protocol.icount.ICount/count children)"
                      + "     (std.native.Edn/write"
                      + "       (std.protocol.ilookup.ILookup/lookup metadata :form))]"
                      + "    [2 \"{:refer demo.subject/run}\"]))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [forms (std.native.Edn/read-forms-spanned \"λ (stop)\")"
                      + "      symbol (std.protocol.inth.INth/nth forms 0)"
                      + "      call (std.protocol.inth.INth/nth forms 1)"
                      + "      stop (std.protocol.inth.INth/nth"
                      + "            (std.protocol.ilookup.ILookup/lookup call :children) 0)]"
                      + " (= [(std.protocol.ilookup.ILookup/lookup"
                      + "       (std.protocol.ilookup.ILookup/lookup symbol :start) :offset)"
                      + "     (std.protocol.ilookup.ILookup/lookup"
                      + "       (std.protocol.ilookup.ILookup/lookup symbol :end) :offset)"
                      + "     (std.protocol.ilookup.ILookup/lookup"
                      + "       (std.protocol.ilookup.ILookup/lookup call :start) :offset)"
                      + "     (std.protocol.ilookup.ILookup/lookup"
                      + "       (std.protocol.ilookup.ILookup/lookup stop :start) :offset)]"
                      + "    [0 2 3 4]))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (std.native.Edn/read-forms-spanned \"(throw (ex :demo {}))\")"
                      + " true (catch error false))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (std.native.Edn/read-forms-spanned \"(\")"
                      + " false (catch error true))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (std.native.Edn/read-forms-spanned \"  ; comment\")"
                      + " false (catch error true))")
              .asBoolean());
    }
  }

  @SuppressWarnings("rawtypes")
  private static long stateLong(Object library, String name) {
    IMapType state = (IMapType) HaraNativeLang.invoke("Library", "state", new Object[] {library});
    return ((Number) state.lookup(Keyword.create(name))).longValue();
  }
}
