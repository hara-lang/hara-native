package hara.work;

import hara.lang.data.Keyword;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

/** Asynchronous evaluator for {@link WorkPlan} using explicit named targets. */
public final class WorkRuntime {
  public record Event(Keyword type, Map<Object, Object> data) {}

  /** Execution-local cancellation and event boundary. */
  public static final class Context {
    private final AtomicBoolean cancelled;
    private final Consumer<Event> events;

    public Context() {
      this(new AtomicBoolean(false), ignored -> {});
    }

    public Context(AtomicBoolean cancelled, Consumer<Event> events) {
      this.cancelled = Objects.requireNonNull(cancelled, "cancelled");
      this.events = Objects.requireNonNull(events, "events");
    }

    public boolean cancelled() {
      return cancelled.get();
    }

    public void cancel() {
      cancelled.set(true);
    }

    public void checkCancelled() {
      if (cancelled()) throw new CancellationException("work/cancelled");
    }

    public void emit(String type, Map<Object, Object> data) {
      events.accept(new Event(Keyword.create(type), Map.copyOf(data)));
    }
  }

  @FunctionalInterface
  public interface Suspension {
    CompletionStage<Object> await(Object wait, Context context);
  }

  private final WorkRegistry registry;
  private final Suspension suspension;

  public WorkRuntime(WorkRegistry registry) {
    this(registry, null);
  }

  public WorkRuntime(WorkRegistry registry, Suspension suspension) {
    this.registry = Objects.requireNonNull(registry, "registry");
    this.suspension = suspension;
  }

  public WorkRegistry registry() {
    return registry;
  }

  /** Resets process-local bindings. Calling it repeatedly restores the same baseline. */
  public void reset() {
    registry.reset();
  }

  public CompletionStage<Object> evaluate(WorkPlan plan, Object input) {
    return evaluate(plan, input, new Context());
  }

  public CompletionStage<Object> evaluate(WorkPlan plan, Object input, Context context) {
    return execute(plan.value(), input, context, 0);
  }

  private CompletionStage<Object> execute(Object value, Object input, Context context, int bindDepth) {
    try {
      WorkPlan.validate(value);
      context.checkCancelled();
      WorkPlan.Operation operation = WorkPlan.Operation.from(WorkPlan.field(value, WorkPlan.OP_KEY));
      context.emit("work/node-started", Map.of(Keyword.create("operation"), operation.keyword()));
      CompletionStage<Object> result =
          switch (operation) {
            case PURE, STEP -> target(value, input, context);
            case CHAIN -> chain(WorkPlan.asList(WorkPlan.field(value, WorkPlan.CHILDREN_KEY), "work children"), input, context, bindDepth);
            case ALL -> all(WorkPlan.asList(WorkPlan.field(value, WorkPlan.CHILDREN_KEY), "work children"), input, context, bindDepth);
            case EACH -> each(WorkPlan.field(value, WorkPlan.CHILD_KEY), input, context, bindDepth, false);
            case FILTER -> each(WorkPlan.field(value, WorkPlan.CHILD_KEY), input, context, bindDepth, true);
            case FOLD -> fold(value, input, context, bindDepth);
            case CHOOSE -> choose(value, input, context, bindDepth);
            case BIND -> bind(value, input, context, bindDepth);
            case ENSURE -> ensure(value, input, context, bindDepth);
            case AWAIT -> await(value, context);
            case GRAPH -> graph(value, input, context, bindDepth);
            case BATCH -> batch(value, input, context, bindDepth);
          };
      return result.thenApply(
          output -> {
            context.emit("work/node-completed", Map.of(Keyword.create("operation"), operation.keyword()));
            return output;
          });
    } catch (Throwable error) {
      return failed(error);
    }
  }

  private CompletionStage<Object> target(Object plan, Object input, Context context) {
    String name = WorkPlan.target(WorkPlan.field(plan, WorkPlan.TARGET_KEY));
    return registry.target(name).<CompletionStage<Object>>map(target -> target.run(input, context))
        .orElseGet(() -> failed(new IllegalStateException("work/target-unavailable: " + name)));
  }

  private CompletionStage<Object> chain(List<Object> children, Object input, Context context, int depth) {
    CompletionStage<Object> result = CompletableFuture.completedFuture(input);
    for (Object child : children) result = result.thenCompose(value -> execute(child, value, context, depth));
    return result;
  }

  private CompletionStage<Object> all(List<Object> children, Object input, Context context, int depth) {
    CompletionStage<List<Object>> result = CompletableFuture.completedFuture(new ArrayList<>());
    for (Object child : children) {
      result = result.thenCompose(output -> execute(child, input, context, depth).thenApply(value -> { output.add(value); return output; }));
    }
    return result.thenApply(List::copyOf);
  }

  private CompletionStage<Object> each(Object child, Object input, Context context, int depth, boolean filtering) {
    CompletionStage<List<Object>> result = CompletableFuture.completedFuture(new ArrayList<>());
    for (Object item : WorkPlan.asList(input, "work input")) {
      result = result.thenCompose(output -> execute(child, item, context, depth).thenApply(value -> {
        if (!filtering || truthy(value)) output.add(filtering ? item : value);
        return output;
      }));
    }
    return result.thenApply(List::copyOf);
  }

  private CompletionStage<Object> fold(Object plan, Object input, Context context, int depth) {
    Object reducer = WorkPlan.field(plan, WorkPlan.REDUCER_KEY);
    CompletionStage<Object> result = CompletableFuture.completedFuture(WorkPlan.field(plan, WorkPlan.INITIAL_KEY));
    for (Object item : WorkPlan.asList(input, "work input")) {
      result = result.thenCompose(acc -> execute(reducer, map("acc", acc, "item", item), context, depth));
    }
    return result;
  }

  private CompletionStage<Object> choose(Object plan, Object input, Context context, int depth) {
    Object selector = WorkPlan.field(plan, WorkPlan.SELECTOR_KEY);
    Map<Object, Object> choices = WorkPlan.asMap(WorkPlan.field(plan, WorkPlan.CHOICES_KEY), "work choices");
    return execute(selector, input, context, depth).thenCompose(selected -> {
      Object child = choices.get(selected);
      return child == null ? failed(new IllegalStateException("work/choice-missing")) : execute(child, input, context, depth);
    });
  }

  private CompletionStage<Object> bind(Object plan, Object input, Context context, int depth) {
    if (depth >= 64) return failed(new IllegalStateException("work/bind-depth-exceeded"));
    Object source = WorkPlan.field(plan, WorkPlan.SOURCE_KEY);
    String name = WorkPlan.target(WorkPlan.field(plan, WorkPlan.CONTINUATION_KEY));
    return execute(source, input, context, depth).thenCompose(value -> registry.target(name)
        .<CompletionStage<Object>>map(target -> target.run(value, context))
        .orElseGet(() -> failed(new IllegalStateException("work/target-unavailable: " + name)))
        .thenCompose(produced -> {
          try { return execute(WorkPlan.fromValue(produced).value(), value, context, depth + 1); }
          catch (Throwable error) { return failed(new IllegalStateException("work/bind-target-returned-non-plan", error)); }
        }));
  }

  private CompletionStage<Object> ensure(Object plan, Object input, Context context, int depth) {
    Object body = WorkPlan.field(plan, WorkPlan.CHILD_KEY);
    Object cleanup = WorkPlan.field(plan, WorkPlan.CLEANUP_KEY);
    return execute(body, input, context, depth).handle((value, error) -> new Outcome(value, error))
        .thenCompose(outcome -> execute(cleanup, map("work/input", input, "work/body-status", outcome.error == null ? Keyword.create("completed") : Keyword.create("failed"), "work/body-result", outcome.value), context, depth)
            .thenCompose(ignored -> outcome.error == null ? CompletableFuture.completedFuture(outcome.value) : failed(outcome.error)));
  }

  private CompletionStage<Object> await(Object plan, Context context) {
    if (suspension == null) return failed(new IllegalStateException("work/suspension-unavailable"));
    return suspension.await(WorkPlan.field(plan, WorkPlan.WAIT_KEY), context);
  }

  private CompletionStage<Object> graph(Object plan, Object input, Context context, int depth) {
    Map<Object, Object> fields = WorkPlan.asMap(plan, "work plan");
    Map<Object, Object> nodes = WorkPlan.asMap(fields.get(Keyword.create("work", "nodes")), "work graph nodes");
    List<Object> order = WorkPlan.asList(fields.get(Keyword.create("work", "order")), "work graph order");
    CompletionStage<Map<Object, Object>> result = CompletableFuture.completedFuture(new LinkedHashMap<>());
    for (Object id : order) {
      result = result.thenCompose(output -> execute(nodes.get(id), input, context, depth).thenApply(value -> { output.put(id, value); return output; }));
    }
    return result.thenApply(Map::copyOf);
  }

  private CompletionStage<Object> batch(Object plan, Object input, Context context, int depth) {
    Object process = WorkPlan.field(plan, Keyword.create("work", "process"));
    return each(process, input, context, depth, false);
  }

  private record Outcome(Object value, Throwable error) {}

  private static boolean truthy(Object value) {
    return value != null && !Boolean.FALSE.equals(value);
  }

  private static Map<Object, Object> map(String firstKey, Object firstValue, String secondKey, Object secondValue) {
    return map(firstKey, firstValue, secondKey, secondValue, null, null);
  }

  private static Map<Object, Object> map(String firstKey, Object firstValue, String secondKey, Object secondValue, String thirdKey, Object thirdValue) {
    Map<Object, Object> values = new LinkedHashMap<>();
    values.put(Keyword.create(firstKey), firstValue);
    values.put(Keyword.create(secondKey), secondValue);
    if (thirdKey != null) values.put(Keyword.create(thirdKey), thirdValue);
    return values;
  }

  private static <T> CompletionStage<T> failed(Throwable error) {
    CompletableFuture<T> result = new CompletableFuture<>();
    result.completeExceptionally(error instanceof CompletionException ? error : new CompletionException(error));
    return result;
  }
}
