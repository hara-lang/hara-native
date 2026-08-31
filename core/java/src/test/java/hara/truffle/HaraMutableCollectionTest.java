package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Direct reader and protocol behavior for the native mutable collection markers. */
public final class HaraMutableCollectionTest {
  @Test
  public void mutableMarkersAndReaderTagsSupportLookup() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[true true 2 :missing 42 :missing 7 43]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [array #arr[1 (+ 1 1)]\n"
                      + "      object #obj{\"answer\" (+ 40 2)}]\n"
                      + "  [(satisfies? ILookup (Arr/new 1 2))\n"
                      + "   (satisfies? ILookup (Obj/new \"answer\" 42))\n"
                      + "   (ILookup/lookup array 1)\n"
                      + "   (ILookup/lookup array 9 :missing)\n"
                      + "   (ILookup/lookup object \"answer\")\n"
                      + "   (ILookup/lookup object \"missing\" :missing)\n"
                      + "   (do (Arr/set array 0 7) (ILookup/lookup array 0))\n"
                      + "   (do (Obj/set object \"answer\" 43) (ILookup/lookup object \"answer\"))])")
              .toString());
    }
  }

  @Test
  public void mutableReaderTagsRoundTripThroughDisplay() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[#arr[1 2] #obj{\"answer\" 42} #obj{}]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[#arr[1 (+ 1 1)] #obj {\"answer\" (+ 40 2)} #obj {}]")
              .toString());
    }
  }

  @Test
  public void uuidReaderTagCreatesAndPrintsUuidValues() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[true :std.native.UUID #uuid \"00000000-0000-0000-0000-000000000000\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(= #uuid \"00000000-0000-0000-0000-000000000000\"\n"
                      + "    (std.native.Base/uuid \"00000000-0000-0000-0000-000000000000\"))\n"
                      + " (std.native.Base/type #uuid \"00000000-0000-0000-0000-000000000000\")\n"
                      + " #uuid \"00000000-0000-0000-0000-000000000000\"]")
              .toString());
      assertEquals(
          "#uuid \"00000000-0000-0000-0000-000000000000\"",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.native.Printer/capture\n"
                      + "  (fn [] (std.native.Printer/p #uuid \"00000000-0000-0000-0000-000000000000\")))")
              .toString());
    }
  }
}
