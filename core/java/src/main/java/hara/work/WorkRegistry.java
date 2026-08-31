package hara.work;

import java.util.Optional;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;

/** Process-local bindings for named portable-plan targets. */
public final class WorkRegistry {
  @FunctionalInterface
  public interface Target {
    CompletionStage<Object> run(Object input, WorkRuntime.Context context);
  }

  private record Binding(Object source, Target target) {}

  private final ConcurrentMap<String, Binding> targets = new ConcurrentHashMap<>();

  public void bind(String name, Target target) {
    bind(name, target, target);
  }

  public void bind(String name, Object source, Target target) {
    targets.put(nonBlank(name), new Binding(source, target));
  }

  public boolean unbind(String name) {
    return targets.remove(name) != null;
  }

  public Optional<Target> target(String name) {
    return Optional.ofNullable(targets.get(name)).map(Binding::target);
  }

  public Optional<Object> source(String name) {
    return Optional.ofNullable(targets.get(name)).map(Binding::source);
  }

  public java.util.List<String> targetNames() {
    return targets.keySet().stream().sorted().toList();
  }

  /** Clears all local bindings and is idempotent. */
  public void reset() {
    targets.clear();
  }

  static String nonBlank(String value) {
    if (value == null || value.trim().isEmpty()) throw WorkPlan.invalid("work target cannot be blank");
    return value;
  }
}
