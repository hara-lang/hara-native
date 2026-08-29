package hara.lang.data;

import static org.junit.Assert.assertEquals;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Map.Entry;
import org.junit.Test;

public class PriorityMapTest {
  @Test
  public void ordersByPriorityAndKeepsTiesStable() {
    PriorityMap.Standard<String, Integer> map = PriorityMap.Standard.<String, Integer>empty(null)
        .assoc("a", 2).assoc("b", 1).assoc("c", 1);
    assertEquals(Arrays.asList("b", "c", "a"), keys(map));
    assertEquals(Arrays.asList("b", "c", "a"), keys(map.assoc("b", 1)));
    assertEquals(Arrays.asList("c", "a", "b"), keys(map.assoc("b", 2)));
    assertEquals("b", map.peekFirst().getKey());
    assertEquals("a", map.peekLast().getKey());
    assertEquals(Arrays.asList("c", "a"), keys(map.popFirst()));
    assertEquals(Integer.valueOf(1), map.lookup("b"));
  }

  private static <K, V extends Comparable<? super V>> ArrayList<K> keys(PriorityMap.Standard<K, V> map) {
    ArrayList<K> keys = new ArrayList<>();
    for (Entry<K, V> entry : map) keys.add(entry.getKey());
    return keys;
  }
}
