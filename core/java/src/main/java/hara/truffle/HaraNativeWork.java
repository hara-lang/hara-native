package hara.truffle;

import hara.work.WorkPlan;
import hara.work.WorkRegistry;
import hara.work.WorkRuntime;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Installs the raw native Work surface; protocol dispatch comes from annotations. */
final class HaraNativeWork {
  private static final String NAMESPACE = "std.native.Work";

  private HaraNativeWork() {}

  static void install(HaraContext context) {
    context.defineNativeFunction(
        NAMESPACE,
        "default-host",
        arguments -> {
          requireArity("default-host", arguments, 0);
          return HaraWorkHost.instance();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "reset-host",
        arguments -> {
          requireArity("reset-host", arguments, 1);
          Object value = HaraBox.unwrap(arguments[0]);
          if (!(value instanceof HaraWorkHost host)) {
            throw new HaraException("reset-host requires a native work host");
          }
          host.reset();
          return arguments[0];
        },
        null);
    installPlanSurface(context);
    installRegistrySurface(context);
    installRuntimeSurface(context);
    context.defineNativeFunction(
        NAMESPACE,
        "current-run",
        arguments -> {
          requireArity("current-run", arguments, 0);
          HaraWorkHost.WorkContext current = HaraWorkHost.currentWorkContext();
          return current == null ? null : current.currentRun();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "cancelled?",
        arguments -> requireCurrent("cancelled?", arguments, 0).cancelled(),
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "check-cancelled",
        arguments -> {
          requireCurrent("check-cancelled", arguments, 0).checkCancelled();
          return null;
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "deadline-nanos",
        arguments -> requireCurrent("deadline-nanos", arguments, 0).deadlineNanos(),
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "emit",
        arguments -> {
          requireArity("emit", arguments, 2);
          return requireCurrent("emit", arguments, 2).emit(arguments[0], arguments[1]);
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "submit-child",
        arguments -> {
          if (arguments.length == 2) {
            return requireCurrent("submit-child", arguments, 2)
                .submitChild(arguments[0], arguments[1], null);
          }
          if (arguments.length == 3) {
            return requireCurrent("submit-child", arguments, 3)
                .submitChild(arguments[0], arguments[1], arguments[2]);
          }
          throw new HaraException("submit-child expects 2 or 3 arguments");
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "on-close",
        arguments -> {
          requireArity("on-close", arguments, 1);
          Object function = arguments[0];
          Object wrapper =
              context.libraryFunction(
                  NAMESPACE + "/on-close-finalizer",
                  ignored ->
                      context.invokeCallable(
                          function,
                          new Object[] {
                            requireCurrent("on-close", new Object[0], 0).currentRun()
                          }));
          return requireCurrent("on-close", arguments, 1).onClose(wrapper);
        },
        null);
  }

  private static void installPlanSurface(HaraContext context) {
    context.defineNativeFunction(
        NAMESPACE,
        "plan?",
        arguments -> {
          requireArity("plan?", arguments, 1);
          try {
            WorkPlan.fromValue(HaraBox.unwrap(arguments[0]));
            return true;
          } catch (RuntimeException error) {
            return false;
          }
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "configured",
        arguments -> {
          requireArity("configured", arguments, 2);
          return planValue(
              WorkPlan.configured(
                  WorkPlan.Operation.from(HaraBox.unwrap(arguments[0])),
                  WorkPlan.asMap(HaraBox.unwrap(arguments[1]), "work plan fields")));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "pure",
        arguments -> {
          requireArity("pure", arguments, 1);
          return planValue(WorkPlan.pure(WorkPlan.target(HaraBox.unwrap(arguments[0]))));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "step",
        arguments -> {
          requireArity("step", arguments, 1);
          return planValue(WorkPlan.step(WorkPlan.target(HaraBox.unwrap(arguments[0]))));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "chain",
        arguments -> {
          requireArity("chain", arguments, 1);
          return planValue(WorkPlan.chain(plans(arguments[0])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "all",
        arguments -> {
          requireArity("all", arguments, 1);
          return planValue(WorkPlan.all(plans(arguments[0])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "each",
        arguments -> {
          requireArity("each", arguments, 1);
          return planValue(WorkPlan.each(plan(arguments[0])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "filter",
        arguments -> {
          requireArity("filter", arguments, 1);
          return planValue(WorkPlan.filter(plan(arguments[0])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "fold",
        arguments -> {
          requireArity("fold", arguments, 2);
          return planValue(WorkPlan.fold(HaraBox.unwrap(arguments[0]), plan(arguments[1])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "choose",
        arguments -> {
          requireArity("choose", arguments, 2);
          Map<Object, WorkPlan> choices = new LinkedHashMap<>();
          WorkPlan.asMap(HaraBox.unwrap(arguments[1]), "work choices")
              .forEach((key, value) -> choices.put(key, plan(value)));
          return planValue(WorkPlan.choose(plan(arguments[0]), choices));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "graph",
        arguments -> {
          requireArity("graph", arguments, 2);
          Map<Object, WorkPlan> nodes = new LinkedHashMap<>();
          WorkPlan.asMap(HaraBox.unwrap(arguments[0]), "work graph nodes")
              .forEach((key, value) -> nodes.put(key, plan(value)));
          return planValue(
              WorkPlan.graph(
                  nodes, WorkPlan.asList(HaraBox.unwrap(arguments[1]), "work graph order")));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "batch",
        arguments -> {
          requireArity("batch", arguments, 1);
          return planValue(WorkPlan.batch(plan(arguments[0])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "bind",
        arguments -> {
          requireArity("bind", arguments, 2);
          return planValue(WorkPlan.bind(plan(arguments[0]), WorkPlan.target(HaraBox.unwrap(arguments[1]))));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "ensure",
        arguments -> {
          requireArity("ensure", arguments, 2);
          return planValue(WorkPlan.ensure(plan(arguments[0]), plan(arguments[1])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "await",
        arguments -> {
          requireArity("await", arguments, 1);
          return planValue(WorkPlan.await(HaraBox.unwrap(arguments[0])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "encode-hta",
        arguments -> {
          requireArity("encode-hta", arguments, 1);
          return plan(arguments[0]).encodeHta();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "decode-hta",
        arguments -> {
          requireArity("decode-hta", arguments, 1);
          Object value = HaraBox.unwrap(arguments[0]);
          if (!(value instanceof byte[] bytes)) throw new HaraException("decode-hta expects bytes");
          return planValue(WorkPlan.decodeHta(bytes));
        },
        null);
  }

  private static void installRegistrySurface(HaraContext context) {
    context.defineNativeFunction(
        NAMESPACE,
        "new-registry",
        arguments -> {
          requireArity("new-registry", arguments, 0);
          return new WorkRegistry();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "bind-target",
        arguments -> {
          requireArity("bind-target", arguments, 3);
          WorkRegistry registry = registry(arguments[0]);
          String name = WorkPlan.target(HaraBox.unwrap(arguments[1]));
          Object function = HaraBox.unwrap(arguments[2]);
          registry.bind(
              name,
              function,
              (input, ignored) ->
                  context.promiseFuture(
                      context.invokeInContext(
                          () -> context.invokeCallable(function, new Object[] {input}))));
          return arguments[0];
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "unbind-target",
        arguments -> {
          requireArity("unbind-target", arguments, 2);
          return registry(arguments[0]).unbind(WorkPlan.target(HaraBox.unwrap(arguments[1])));
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "target",
        arguments -> {
          requireArity("target", arguments, 2);
          return registry(arguments[0])
              .source(WorkPlan.target(HaraBox.unwrap(arguments[1])))
              .orElse(null);
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "target-names",
        arguments -> {
          requireArity("target-names", arguments, 1);
          return HaraPersistentValues.normalize(registry(arguments[0]).targetNames());
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "reset-registry",
        arguments -> {
          requireArity("reset-registry", arguments, 1);
          registry(arguments[0]).reset();
          return arguments[0];
        },
        null);
  }

  private static void installRuntimeSurface(HaraContext context) {
    context.defineNativeFunction(
        NAMESPACE,
        "new-runtime",
        arguments -> {
          if (arguments.length == 1) return new WorkRuntime(registry(arguments[0]));
          if (arguments.length == 2) {
            Object function = HaraBox.unwrap(arguments[1]);
            if (function == null) return new WorkRuntime(registry(arguments[0]));
            return new WorkRuntime(
                registry(arguments[0]),
                (wait, ignored) ->
                    context.promiseFuture(
                        context.invokeInContext(
                            () -> context.invokeCallable(function, new Object[] {wait}))));
          }
          throw new HaraException("new-runtime expects a registry and optional suspension target");
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "runtime-registry",
        arguments -> {
          requireArity("runtime-registry", arguments, 1);
          return runtime(arguments[0]).registry();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "evaluate",
        arguments -> {
          requireArity("evaluate", arguments, 3);
          return context.promiseValue(
              runtime(arguments[0])
                  .evaluate(plan(arguments[1]), HaraBox.unwrap(arguments[2]))
                  .toCompletableFuture());
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "reset-runtime",
        arguments -> {
          requireArity("reset-runtime", arguments, 1);
          runtime(arguments[0]).reset();
          return arguments[0];
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "submit-plan",
        arguments -> {
          if (arguments.length != 4 && arguments.length != 5) {
            throw new HaraException("submit-plan expects host, runtime, plan, input, and optional options");
          }
          Object host = HaraBox.unwrap(arguments[0]);
          if (!(host instanceof HaraWorkHost workHost)) {
            throw new HaraException("submit-plan requires a native work host");
          }
          return workHost.submitPlan(
              context,
              runtime(arguments[1]),
              plan(arguments[2]),
              HaraBox.unwrap(arguments[3]),
              arguments.length == 5 ? HaraBox.unwrap(arguments[4]) : null);
        },
        null);
  }

  private static Object planValue(WorkPlan plan) {
    return HaraPersistentValues.normalize(plan.value());
  }

  private static WorkPlan plan(Object value) {
    return WorkPlan.fromValue(HaraBox.unwrap(value));
  }

  private static List<WorkPlan> plans(Object value) {
    return WorkPlan.asList(HaraBox.unwrap(value), "work children").stream()
        .map(HaraNativeWork::plan)
        .toList();
  }

  private static WorkRegistry registry(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof WorkRegistry registry) return registry;
    throw new HaraException("native WorkRegistry operation requires a native WorkRegistry");
  }

  private static WorkRuntime runtime(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof WorkRuntime runtime) return runtime;
    throw new HaraException("native WorkRuntime operation requires a native WorkRuntime");
  }

  private static HaraWorkHost.WorkContext requireCurrent(
      String name, Object[] arguments, int arity) {
    requireArity(name, arguments, arity);
    HaraWorkHost.WorkContext workContext = HaraWorkHost.currentWorkContext();
    if (workContext == null) {
      throw new HaraException(name + " requires an active native work context");
    }
    return workContext;
  }

  private static void requireArity(String name, Object[] arguments, int arity) {
    if (arguments.length != arity) {
      throw new HaraException(name + " expects " + arity + " arguments");
    }
  }
}
