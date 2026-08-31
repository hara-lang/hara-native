package hara.work;

import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import hara.truffle.HtaValueCodec;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Canonical closure-free work plan shared by Hara, Rust, and JVM hosts.
 *
 * <p>Only data and stable target IDs cross the HTA0 boundary. Local callbacks belong in a
 * {@link WorkRegistry} and are therefore not serializable.
 */
public final class WorkPlan {
  public static final long VERSION = 1L;
  static final Keyword VERSION_KEY = Keyword.create("work", "plan-version");
  static final Keyword OP_KEY = Keyword.create("work", "op");
  static final Keyword TARGET_KEY = Keyword.create("work", "target");
  static final Keyword CHILDREN_KEY = Keyword.create("work", "children");
  static final Keyword CHILD_KEY = Keyword.create("work", "child");
  static final Keyword SOURCE_KEY = Keyword.create("work", "source");
  static final Keyword CONTINUATION_KEY = Keyword.create("work", "continuation-target");
  static final Keyword CLEANUP_KEY = Keyword.create("work", "cleanup");
  static final Keyword INITIAL_KEY = Keyword.create("work", "initial");
  static final Keyword REDUCER_KEY = Keyword.create("work", "reducer");
  static final Keyword SELECTOR_KEY = Keyword.create("work", "selector");
  static final Keyword CHOICES_KEY = Keyword.create("work", "choices");
  static final Keyword WAIT_KEY = Keyword.create("work", "wait");
  static final Keyword NODES_KEY = Keyword.create("work", "nodes");
  static final Keyword ORDER_KEY = Keyword.create("work", "order");
  static final Keyword PROCESS_KEY = Keyword.create("work", "process");

  public enum Operation {
    PURE,
    STEP,
    CHAIN,
    EACH,
    FILTER,
    FOLD,
    ALL,
    CHOOSE,
    GRAPH,
    BATCH,
    BIND,
    ENSURE,
    AWAIT;

    public Keyword keyword() {
      return Keyword.create(name().toLowerCase());
    }

    public static Operation from(Object value) {
      if (!(value instanceof Keyword keyword)) throw invalid("work operation must be a keyword");
      try {
        return Operation.valueOf(keyword.getName().toUpperCase());
      } catch (IllegalArgumentException error) {
        throw unsupported(namespacedName(keyword));
      }
    }
  }

  private final Object value;

  private WorkPlan(Object value) {
    this.value = value;
  }

  public static WorkPlan fromValue(Object value) {
    validate(value);
    return new WorkPlan(value);
  }

  public Object value() {
    return value;
  }

  public Operation operation() {
    return Operation.from(field(value, OP_KEY));
  }

  public byte[] encodeHta() {
    return HtaValueCodec.encode(value);
  }

  public static WorkPlan decodeHta(byte[] bytes) {
    return fromValue(HtaValueCodec.decodeCanonical(bytes));
  }

  public static WorkPlan pure(String target) {
    return leaf(Operation.PURE, target);
  }

  public static WorkPlan step(String target) {
    return leaf(Operation.STEP, target);
  }

  public static WorkPlan leaf(Operation operation, String target) {
    if (operation != Operation.PURE && operation != Operation.STEP) {
      throw invalid("only pure and step are leaf operations");
    }
    return fromValue(map(VERSION_KEY, VERSION, OP_KEY, operation.keyword(), TARGET_KEY, target(target)));
  }

  public static WorkPlan chain(List<WorkPlan> children) {
    return children(Operation.CHAIN, children);
  }

  public static WorkPlan all(List<WorkPlan> children) {
    return children(Operation.ALL, children);
  }

  public static WorkPlan each(WorkPlan child) {
    return child(Operation.EACH, child);
  }

  public static WorkPlan filter(WorkPlan child) {
    return child(Operation.FILTER, child);
  }

  public static WorkPlan children(Operation operation, List<WorkPlan> children) {
    List<Object> values = children.stream().map(WorkPlan::value).toList();
    return fromValue(map(VERSION_KEY, VERSION, OP_KEY, operation.keyword(), CHILDREN_KEY, values));
  }

  public static WorkPlan child(Operation operation, WorkPlan child) {
    return fromValue(map(VERSION_KEY, VERSION, OP_KEY, operation.keyword(), CHILD_KEY, child.value));
  }

  public static WorkPlan fold(Object initial, WorkPlan reducer) {
    return fromValue(
        map(VERSION_KEY, VERSION, OP_KEY, Operation.FOLD.keyword(), INITIAL_KEY, initial, REDUCER_KEY, reducer.value));
  }

  public static WorkPlan choose(WorkPlan selector, Map<?, WorkPlan> choices) {
    Map<Object, Object> values = new LinkedHashMap<>();
    choices.forEach((key, plan) -> values.put(key, plan.value));
    return fromValue(
        map(VERSION_KEY, VERSION, OP_KEY, Operation.CHOOSE.keyword(), SELECTOR_KEY, selector.value, CHOICES_KEY, values));
  }

  public static WorkPlan bind(WorkPlan source, String continuationTarget) {
    return fromValue(
        map(VERSION_KEY, VERSION, OP_KEY, Operation.BIND.keyword(), SOURCE_KEY, source.value, CONTINUATION_KEY, target(continuationTarget)));
  }

  public static WorkPlan ensure(WorkPlan body, WorkPlan cleanup) {
    return fromValue(
        map(VERSION_KEY, VERSION, OP_KEY, Operation.ENSURE.keyword(), CHILD_KEY, body.value, CLEANUP_KEY, cleanup.value));
  }

  public static WorkPlan await(Object wait) {
    return fromValue(map(VERSION_KEY, VERSION, OP_KEY, Operation.AWAIT.keyword(), WAIT_KEY, wait));
  }

  public static WorkPlan graph(Map<?, WorkPlan> nodes, List<?> order) {
    Map<Object, Object> values = new LinkedHashMap<>();
    nodes.forEach((id, plan) -> values.put(id, plan.value));
    return configured(Operation.GRAPH, map(NODES_KEY, values, ORDER_KEY, List.copyOf(order)));
  }

  public static WorkPlan batch(WorkPlan process) {
    return configured(Operation.BATCH, map(PROCESS_KEY, process.value));
  }

  /** Builder for graph and batch envelopes, whose operation-specific fields remain portable data. */
  public static WorkPlan configured(Operation operation, Map<?, ?> fields) {
    Map<Object, Object> values = new LinkedHashMap<>(fields);
    values.put(VERSION_KEY, VERSION);
    values.put(OP_KEY, operation.keyword());
    return fromValue(values);
  }

  @SuppressWarnings("unchecked")
  public static Object field(Object value, Object key) {
    if (value instanceof java.util.Map<?, ?> map) return map.get(key);
    if (value instanceof IMapType<?, ?> map) return ((IMapType<Object, Object>) map).lookup(key);
    return null;
  }

  public static Map<Object, Object> asMap(Object value, String description) {
    if (value instanceof java.util.Map<?, ?> map) {
      Map<Object, Object> output = new LinkedHashMap<>();
      map.forEach((key, item) -> output.put(key, item));
      return output;
    }
    if (value instanceof IMapType<?, ?> map) {
      Map<Object, Object> output = new LinkedHashMap<>();
      map.iterator().forEachRemaining(entry -> output.put(entry.getKey(), entry.getValue()));
      return output;
    }
    throw invalid(description + " must be a map");
  }

  public static List<Object> asList(Object value, String description) {
    if (value instanceof List<?> list) return new ArrayList<>(list);
    if (value instanceof hara.lang.protocol.ILinearType linear) {
      List<Object> output = new ArrayList<>();
      for (int index = 0; index < linear.count(); index++) output.add(linear.nth(index));
      return output;
    }
    throw invalid(description + " must be a vector");
  }

  public static String target(Object value) {
    if (value instanceof String text && !text.trim().isEmpty()) return text;
    if (value instanceof Keyword keyword) return nonBlank(namespacedName(keyword));
    if (value instanceof hara.lang.data.Symbol symbol) return nonBlank(namespacedName(symbol));
    throw invalid("work target must be a non-blank string, keyword, or symbol");
  }

  private static String namespacedName(hara.lang.protocol.INamespaced value) {
    String namespace = value.getNamespace();
    return namespace == null ? value.getName() : namespace + "/" + value.getName();
  }

  private static String nonBlank(String value) {
    if (value.trim().isEmpty()) throw invalid("work target must be a non-blank string, keyword, or symbol");
    return value;
  }

  private static Map<Object, Object> map(Object... entries) {
    Map<Object, Object> output = new LinkedHashMap<>();
    for (int index = 0; index < entries.length; index += 2) output.put(entries[index], entries[index + 1]);
    return output;
  }

  public static void validate(Object value) {
    Map<Object, Object> fields = asMap(value, "work plan");
    if (!Long.valueOf(VERSION).equals(field(fields, VERSION_KEY))) throw invalid("unsupported plan version");
    Operation operation = Operation.from(field(fields, OP_KEY));
    switch (operation) {
      case PURE, STEP -> target(required(fields, TARGET_KEY));
      case CHAIN, ALL -> asList(required(fields, CHILDREN_KEY), "work children").forEach(WorkPlan::validate);
      case EACH, FILTER -> validate(required(fields, CHILD_KEY));
      case FOLD -> validate(required(fields, REDUCER_KEY));
      case CHOOSE -> {
        validate(required(fields, SELECTOR_KEY));
        asMap(required(fields, CHOICES_KEY), "work choices").values().forEach(WorkPlan::validate);
      }
      case BIND -> {
        validate(required(fields, SOURCE_KEY));
        target(required(fields, CONTINUATION_KEY));
      }
      case ENSURE -> {
        validate(required(fields, CHILD_KEY));
        validate(required(fields, CLEANUP_KEY));
      }
      case AWAIT -> required(fields, WAIT_KEY);
      case GRAPH -> {
        Map<Object, Object> nodes = asMap(required(fields, NODES_KEY), "work graph nodes");
        nodes.values().forEach(WorkPlan::validate);
        for (Object id : asList(required(fields, ORDER_KEY), "work graph order")) {
          Object child = nodes.get(id);
          if (child == null) throw invalid("work graph order refers to an unknown node");
          validate(child);
        }
      }
      case BATCH -> validate(required(fields, PROCESS_KEY));
    }
  }

  private static Object required(Map<Object, Object> fields, Object key) {
    Object value = field(fields, key);
    if (value == null) throw invalid("missing " + key);
    return value;
  }

  public static IllegalArgumentException invalid(String message) {
    return new IllegalArgumentException("work/plan-invalid: " + message);
  }

  static UnsupportedOperationException unsupported(String operation) {
    return new UnsupportedOperationException("work/plan-unsupported: " + operation);
  }
}
