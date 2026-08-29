package hara.truffle;

import hara.kernel.builtin.BuiltinStruct;
import hara.lang.base.Iter;
import hara.lang.data.Trie;
import java.util.Iterator;
import java.util.Map.Entry;

/** Native implementation owner for specialised persistent collection constructors. */
public final class StdNativeAlgo {
  private StdNativeAlgo() {}

  static void install(HaraContext context, String namespace) {
    HaraNativeLibrary.function(context, namespace, "deque", StdNativeAlgo::deque,
        "Creates a persistent finger-tree deque.", "[& values]");
    HaraNativeLibrary.function(context, namespace, "ordered-map", StdNativeAlgo::orderedMap,
        "Creates an insertion-ordered persistent map.", "[& entries]");
    HaraNativeLibrary.function(context, namespace, "ordered-set", StdNativeAlgo::orderedSet,
        "Creates an insertion-ordered persistent set.", "[& values]");
    HaraNativeLibrary.function(context, namespace, "priority-map", StdNativeAlgo::priorityMap,
        "Creates a stable persistent priority map.", "[& entries]");
    HaraNativeLibrary.function(context, namespace, "queue", StdNativeAlgo::queue,
        "Creates a persistent queue.", "[& values]");
    HaraNativeLibrary.function(context, namespace, "sorted-map", StdNativeAlgo::sortedMap,
        "Creates a key-sorted persistent map.", "[& entries]");
    HaraNativeLibrary.function(context, namespace, "sorted-set", StdNativeAlgo::sortedSet,
        "Creates a value-sorted persistent set.", "[& values]");
    HaraNativeLibrary.function(context, namespace, "trie", StdNativeAlgo::trie,
        "Creates a persistent trie from string key/value entries.", "[& entries]");
  }

  public static Object deque(HaraContext context, Object[] values) {
    return BuiltinStruct.deque(values);
  }

  public static Object orderedMap(HaraContext context, Object[] values) {
    return BuiltinStruct.orderedMap(values);
  }

  public static Object orderedSet(HaraContext context, Object[] values) {
    return BuiltinStruct.orderedSet(values);
  }

  public static Object priorityMap(HaraContext context, Object[] values) {
    return BuiltinStruct.priorityMap(values);
  }

  public static Object queue(HaraContext context, Object[] values) {
    return BuiltinStruct.queue(values);
  }

  public static Object sortedMap(HaraContext context, Object[] values) {
    return BuiltinStruct.sortedMap(values);
  }

  public static Object sortedSet(HaraContext context, Object[] values) {
    return BuiltinStruct.sortedSet(values);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static Object trie(HaraContext context, Object[] values) {
    Trie<Object> trie = new Trie.Standard<>();
    Iterator<Entry> entries = Iter.partitionPair(Iter.iter(values));
    while (entries.hasNext()) {
      Entry entry = entries.next();
      Object key = HaraBox.unwrap(entry.getKey());
      if (!(key instanceof String)) {
        throw new HaraException("trie expects string keys");
      }
      trie = trie.assoc((String) key, HaraBox.unwrap(entry.getValue()));
    }
    return trie;
  }
}
