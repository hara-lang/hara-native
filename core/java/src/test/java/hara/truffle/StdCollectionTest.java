package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdCollectionTest {
  @Test
  public void ownsSpecialisedPersistentCollectionConstructors() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns collection.java (:require [std.lib.collection :as collection]))");

      assertEquals(
          "[:std.native.Deque :std.native.OrderedMap :std.native.OrderedSet :std.native.PriorityMap :std.native.Queue :std.native.SortedMap :std.native.SortedSet :std.native.Trie]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(type (collection/deque 1))"
                      + " (type (collection/ordered-map :a 1))"
                      + " (type (collection/ordered-set 1))"
                      + " (type (collection/priority-map :a 2))"
                      + " (type (collection/queue 1))"
                      + " (type (collection/sorted-map :b 2 :a 1))"
                      + " (type (collection/sorted-set 2 1))"
                      + " (type (collection/trie \"alpha\" 7))]")
              .toString());
      assertEquals(
          "true",
          context.eval(HaraLanguage.ID, "(Algo/deque? (Algo/deque 1 2))").toString());
      assertEquals(
          "[true true true true true true true true false false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(collection/deque? (collection/deque))"
                      + " (collection/ordered-map? (collection/ordered-map))"
                      + " (collection/ordered-set? (collection/ordered-set))"
                      + " (collection/priority-map? (collection/priority-map))"
                      + " (collection/queue? (collection/queue))"
                      + " (collection/sorted-map? (collection/sorted-map))"
                      + " (collection/sorted-set? (collection/sorted-set))"
                      + " (collection/trie? (collection/trie))"
                      + " (collection/deque? [])"
                      + " (collection/priority-map? {})]")
              .toString());
      assertEquals(
          "[1 3 [0 1 2 3] [1 2 3 4] [2 3] [1 2] [1 9 3] [1 2 3]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [value (collection/deque 1 2 3)]"
                      + " [(collection/peek-first value) (collection/peek-last value)"
                      + " (collection/push-first value 0) (collection/push-last value 4)"
                      + " (collection/pop-first value) (collection/pop-last value)"
                      + " (assoc value 1 9) value])")
              .toString());
      assertEquals(
          "[[:b :c :a] [:b 1] [:a 2] [:c :a :b] [:c :a] [:b :c]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [value (collection/priority-map :a 2 :b 1 :c 1)]"
                      + " [(keys value) (collection/peek-first value) (collection/peek-last value)"
                      + " (keys (assoc value :b 2)) (keys (collection/pop-first value))"
                      + " (keys (collection/pop-last value))])")
              .toString());
      assertEquals(
          "[[:b :a] [:a :b] 5 7]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(keys (collection/ordered-map :b 2 :a 1))"
                      + " (keys (collection/sorted-map :b 2 :a 1))"
                      + " (nth (collection/queue 4 5 6) 1)"
                      + " (get (collection/trie \"alpha\" 7) \"alpha\")]")
              .toString());
      assertEquals(
          "std.lib.collection/ordered-map",
          context
              .eval(
                  HaraLanguage.ID,
                  "(str (var-sym (resolve (quote collection/ordered-map))))")
              .asString());

      assertThrows(RuntimeException.class, () -> context.eval(HaraLanguage.ID, "(ordered-map)"));
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(std.foundation/ordered-map)"));
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(collection/trie :alpha 7)"));
    }
  }
}
