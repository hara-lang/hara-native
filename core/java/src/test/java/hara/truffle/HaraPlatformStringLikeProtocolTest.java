package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public final class HaraPlatformStringLikeProtocolTest {
  @Test
  public void istringlikeIsInstalledBeforeAnyHalResourceIsLoaded() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[true \"hello/world\" :hello/world \"custom\" \"restored\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct WrappedName [value]) "
                      + "(extend-type WrappedName IStringLike "
                      + "  (to-string [wrapped] (:value wrapped)) "
                      + "  (from-string [wrapped text] (WrappedName text))) "
                      + "[(satisfies? IStringLike :hello) "
                      + " (IStringLike/to-string :hello/world) "
                      + " (IStringLike/from-string :sample \"hello/world\") "
                      + " (IStringLike/to-string (WrappedName \"custom\")) "
                      + " (:value (IStringLike/from-string "
                      + "          (WrappedName \"\") \"restored\"))]")
              .toString());
    }
  }
}
