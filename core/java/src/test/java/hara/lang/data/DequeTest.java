package hara.lang.data;

import static org.junit.Assert.assertEquals;
import java.util.ArrayList;
import java.util.Arrays;
import org.junit.Test;

public class DequeTest {
  @Test
  public void supportsPersistentOperationsAtBothEnds() {
    Deque.Standard<Integer> original = Deque.Standard.from(null, 1, 2, 3);
    Deque.Standard<Integer> changed = original.pushFirst(0).pushLast(4).popFirst().popLast();
    assertEquals(Arrays.asList(1, 2, 3), values(original));
    assertEquals(Arrays.asList(1, 2, 3), values(changed));
    assertEquals(Integer.valueOf(2), original.nth(1));
    assertEquals(Arrays.asList(1, 9, 3), values(original.assoc(1L, 9)));
  }

  @Test
  public void fingerTreeMatchesTwoEndedModel() {
    Deque.Standard<Integer> deque = Deque.Standard.empty(null);
    java.util.ArrayDeque<Integer> model = new java.util.ArrayDeque<>();
    for (int index = 0; index < 2000; index++) {
      if (index % 3 == 0) { deque = deque.pushFirst(index); model.addFirst(index); }
      else { deque = deque.pushLast(index); model.addLast(index); }
    }
    for (int index = 0; index < 1500; index++) {
      if (index % 2 == 0) { deque = deque.popFirst(); model.pollFirst(); }
      else { deque = deque.popLast(); model.pollLast(); }
    }
    assertEquals(new ArrayList<>(model), values(deque));
  }

  private static <E> ArrayList<E> values(Deque.Standard<E> deque) {
    ArrayList<E> values = new ArrayList<>();
    deque.iterator().forEachRemaining(values::add);
    return values;
  }
}
