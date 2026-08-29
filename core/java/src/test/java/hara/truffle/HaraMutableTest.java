package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;

public class HaraMutableTest {
  @Test
  public void definesParallelConstructorsAndTheAssociativeReadSurface() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(defmutable Cursor [x y]) "
                  + "(let [direct (Cursor 1 2) "
                  + "      arrow (->Cursor 3 4) "
                  + "      mapped (map->Cursor {:x 5 :extra 9})] "
                  + "  [(field direct :x) "
                  + "   (field arrow :y) "
                  + "   (:x mapped) "
                  + "   (:y mapped) "
                  + "   (count direct) "
                  + "   (keys direct) "
                  + "   (vals direct)])");
      assertEquals(1L, result.getArrayElement(0).asLong());
      assertEquals(4L, result.getArrayElement(1).asLong());
      assertEquals(5L, result.getArrayElement(2).asLong());
      assertTrue(result.getArrayElement(3).isNull());
      assertEquals(2L, result.getArrayElement(4).asLong());
      assertEquals("[:x :y]", result.getArrayElement(5).toString());
      assertEquals("[1 2]", result.getArrayElement(6).toString());
    }
  }

  @Test
  public void mutationUsesReferenceIdentityAndEvaluatesEachSideOnceInOrder() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(defmutable Cursor [x]) "
                  + "(let [cursor (Cursor 1) alias cursor order (atom []) "
                  + "      replacement "
                  + "        (set! (field (do (swap! order conj :receiver) cursor) :x) "
                  + "              (do (swap! order conj :replacement) 10))] "
                  + "  [replacement @order (field alias :x) "
                  + "   (= cursor alias) (= cursor (Cursor 10))])");
      assertEquals(10L, result.getArrayElement(0).asLong());
      assertEquals("[:receiver :replacement]", result.getArrayElement(1).toString());
      assertEquals(10L, result.getArrayElement(2).asLong());
      assertTrue(result.getArrayElement(3).asBoolean());
      assertFalse(result.getArrayElement(4).asBoolean());
    }
  }

  @Test
  public void metadataProtocolsCallableCatchAndSnapshotKeepTheNamedValueContract() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(defmutable Cursor [x] "
                  + "  ICount (count [self] (field self :x)) "
                  + "  IFn (invoke [self amount] (+ (field self :x) amount))) "
                  + "(let [cursor (Cursor 40) tagged (with-meta cursor {:tag :cursor}) "
                  + "      snapshot (into {} cursor)] "
                  + "  (set! (field tagged :x) 41) "
                  + "  [(meta tagged) (= cursor tagged) (ICount/count cursor) "
                  + "   (cursor 1) snapshot (into {} cursor) "
                  + "   (try (throw cursor) (catch Cursor value (field value :x)))])");
      assertEquals("{:tag :cursor}", result.getArrayElement(0).toString());
      assertTrue(result.getArrayElement(1).asBoolean());
      assertEquals(41L, result.getArrayElement(2).asLong());
      assertEquals(42L, result.getArrayElement(3).asLong());
      assertEquals("{:x 40}", result.getArrayElement(4).toString());
      assertEquals("{:x 41}", result.getArrayElement(5).toString());
      assertEquals(41L, result.getArrayElement(6).asLong());
    }
  }

  @Test
  public void persistentUpdatesAndFieldOnStructsAreRejected() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "(defmutable Cursor [x]) (defstruct Point [x])");
      for (String source :
          new String[] {
            "(assoc (Cursor 1) :x 2)",
            "(dissoc (Cursor 1) :x)",
            "(assoc-in (Cursor {:nested 1}) [:x :nested] 2)",
            "(field (Point 1) :x)",
            "(field (Cursor 1) :missing)",
            "(set! (field (Cursor 1) :missing) 2)"
          }) {
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, source));
      }
    }
  }

  @Test
  public void polyglotMembersReadAndWriteDeclaredFieldsOnly() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value cursor = context.eval(HaraLanguage.ID, "(defmutable Cursor [x y]) (Cursor 1 2)");
      assertTrue(cursor.hasMembers());
      assertEquals(1L, cursor.getMember("x").asLong());
      cursor.putMember("x", 10L);
      assertEquals(10L, cursor.getMember("x").asLong());
      assertThrows(UnsupportedOperationException.class, () -> cursor.putMember("missing", 1L));
    }
  }
}
