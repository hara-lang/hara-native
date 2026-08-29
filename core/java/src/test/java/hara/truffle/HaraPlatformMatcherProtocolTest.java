package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public final class HaraPlatformMatcherProtocolTest {
  @Test
  public void imatchIsInstalledBeforeAnyHalResourceIsLoaded() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct PlatformMatcher [expected]) "
                      + "(extend-type PlatformMatcher IMatch "
                      + "  (match-value [matcher actual] "
                      + "    (= (:expected matcher) actual))) "
                      + "[(boolean IMatch) "
                      + " (IMatch/match-value (PlatformMatcher 42) 42)]")
              .toString());
    }
  }
}
