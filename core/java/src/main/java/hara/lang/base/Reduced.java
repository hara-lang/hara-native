package hara.lang.base;

import hara.lang.data.Symbol;
import hara.lang.data.TaggedLiteral;

/** Runtime marker used to terminate reductions without consuming the remaining iterator. */
public final class Reduced {
  private static final Symbol TAG = Symbol.create("hara.internal", "reduced");

  private Reduced() {}

  public static Object mark(Object value) {
    return new TaggedLiteral(TAG, value);
  }

  public static boolean isReduced(Object value) {
    return value instanceof TaggedLiteral tagged && TAG.equals(tagged.tag());
  }

  public static Object unreduced(Object value) {
    return isReduced(value) ? ((TaggedLiteral) value).form() : value;
  }
}
