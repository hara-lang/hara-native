package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.data.Pointer;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Context-owned implementation of the portable {@code std.native.Command} type.
 *
 * <p>The command model owns route parsing and lifecycle state only. Guest code owns command
 * behavior through a route {@code :handler}, which receives a normalized request and returns the
 * portable {@code {:stdout :stderr :exit}} response.
 */
final class HaraNativeCommand {
  private final HaraContext context;
  private final Map<Long, App> apps = new HashMap<>();
  private final Map<Long, RequestRecord> requests = new HashMap<>();
  private final Map<Long, SnapshotRecord> snapshots = new HashMap<>();
  private long nextApp = 1;
  private long nextRequest = 1;
  private long nextSnapshot = 1;

  HaraNativeCommand(HaraContext context) {
    this.context = context;
  }

  Object invoke(String operation, Object[] values) {
    return switch (operation) {
      case "create" -> {
        requireArity(operation, values, 1);
        AppConfig config = appConfig(values[0], operation);
        long id = nextApp++;
        apps.put(id, new App(id, config));
        yield pointer("app", Map.of(key("id"), id));
      }
      case "config" -> {
        requireArity(operation, values, 1);
        App app = app(values[0], operation);
        yield map(key("id"), Symbol.create(app.config.id), key("desc"), app.config.desc);
      }
      case "install" -> {
        requireArity(operation, values, 2);
        App app = app(values[0], operation);
        Route route = route(values[1], operation);
        long handle = app.install(route);
        yield pointer("route", Map.of(key("id"), handle, key("app"), app.handle));
      }
      case "uninstall" -> {
        requireArity(operation, values, 2);
        App app = app(values[0], operation);
        yield app.uninstall(routeHandle(values[1], app.handle, operation));
      }
      case "routes" -> {
        requireArity(operation, values, 1);
        App app = app(values[0], operation);
        app.requireOpen();
        yield vector(app.routes.stream().map(this::routeValue).toList());
      }
      case "snapshot" -> {
        requireArity(operation, values, 1);
        App app = app(values[0], operation);
        app.requireOpen();
        long id = nextSnapshot++;
        snapshots.put(id, new SnapshotRecord(app.handle, app.snapshot()));
        yield pointer("snapshot", Map.of(key("id"), id, key("app"), app.handle));
      }
      case "restore" -> {
        requireArity(operation, values, 2);
        App app = app(values[0], operation);
        long snapshot = handle(values[1], "snapshot", operation);
        SnapshotRecord record = requiredSnapshot(snapshot, operation);
        if (record.app != app.handle) {
          throw failure(operation, "snapshot belongs to a different application");
        }
        app.restore(record.routes);
        yield values[0];
      }
      case "reset" -> {
        requireArity(operation, values, 1);
        App app = app(values[0], operation);
        app.reset();
        requests.values().removeIf(request -> request.app == app.handle);
        yield values[0];
      }
      case "closed?" -> {
        requireArity(operation, values, 1);
        yield app(values[0], operation).closed;
      }
      case "close" -> {
        requireArity(operation, values, 1);
        App app = app(values[0], operation);
        app.close();
        requests.values().removeIf(request -> request.app == app.handle);
        yield null;
      }
      case "parse" -> {
        requireArity(operation, values, 2);
        yield parse(app(values[0], operation), values[1], operation);
      }
      case "dispatch" -> {
        requireArity(operation, values, 2);
        yield dispatch(app(values[0], operation), values[1], operation);
      }
      case "run" -> {
        requireArity(operation, values, 2);
        App app = app(values[0], operation);
        try {
          Object request = parse(app, values[1], operation);
          try {
            yield dispatch(app, request, operation);
          } catch (RuntimeException error) {
            yield response(1, error);
          }
        } catch (RuntimeException error) {
          yield response(2, error);
        }
      }
      default -> throw failure(operation, "unknown operation");
    };
  }

  private Object parse(App app, Object invocation, String operation) {
    Invocation parsed = invocation(invocation, operation);
    Request request = app.parse(parsed.argv);
    long id = nextRequest++;
    requests.put(id, new RequestRecord(app.handle, request, parsed.context));
    return requestValue(id, request, parsed.context);
  }

  private Object dispatch(App app, Object requestValue, String operation) {
    long id = requestId(requestValue, operation);
    RequestRecord record = requiredRequest(id, operation);
    if (record.app != app.handle) {
      throw failure(operation, "request belongs to a different application");
    }
    Object handler = app.handler(record.request);
    Object output = invokeHandler(handler, requestValue(id, record.request, record.context));
    return checkedResponse(output, operation);
  }

  private Object invokeHandler(Object handler, Object request) {
    Object raw = HaraBox.unwrap(handler);
    if (raw instanceof HbcMachine.HbcClosure closure) {
      return HaraBox.unwrap(closure.invokeInterpreted(new Object[] {request}));
    }
    return context.invokeCallable(raw, new Object[] {request});
  }

  private App app(Object value, String operation) {
    long handle = handle(value, "app", operation);
    App app = apps.get(handle);
    if (app == null) throw failure(operation, "application was not found");
    return app;
  }

  private RequestRecord requiredRequest(long id, String operation) {
    RequestRecord request = requests.get(id);
    if (request == null) throw failure(operation, "request was not found");
    return request;
  }

  private SnapshotRecord requiredSnapshot(long id, String operation) {
    SnapshotRecord snapshot = snapshots.get(id);
    if (snapshot == null) throw failure(operation, "snapshot was not found");
    return snapshot;
  }

  private long routeHandle(Object value, long app, String operation) {
    long route = handle(value, "route", operation);
    Pointer pointer = pointerValue(value, "route", operation);
    Object owner = pointer.lookup(key("app"));
    if (!(owner instanceof Number number) || number.longValue() != app) {
      throw failure(operation, "route belongs to a different application");
    }
    return route;
  }

  private long requestId(Object value, String operation) {
    IMapType<?, ?> request = requiredMap(value, operation, "a Command/parse request");
    return handle(HaraContext.lookupValue(request, key("command/request")), "request", operation);
  }

  private long handle(Object value, String kind, String operation) {
    Pointer pointer = pointerValue(value, kind, operation);
    Object id = pointer.lookup(key("id"));
    if (!(id instanceof Number number) || number.longValue() <= 0) {
      throw failure(operation, "received an invalid command/" + kind + " handle");
    }
    return number.longValue();
  }

  private Pointer pointerValue(Object value, String kind, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof Pointer pointer)
        || !Keyword.create("command", kind).equals(pointer.context())) {
      throw failure(operation, "expects a command/" + kind + " handle");
    }
    return pointer;
  }

  private AppConfig appConfig(Object value, String operation) {
    IMapType<?, ?> config = requiredMap(value, operation, "a config map");
    Object id = HaraContext.lookupValue(config, key("id"));
    if (!(id instanceof Symbol symbol) || symbol.display().isBlank()) {
      throw failure(operation, "config :id must be a symbol");
    }
    Object desc = HaraContext.lookupValue(config, key("desc"));
    if (!(desc instanceof String text) || text.isBlank()) {
      throw failure(operation, "config :desc must be a non-empty string");
    }
    return new AppConfig(symbol.display(), text);
  }

  private Route route(Object value, String operation) {
    IMapType<?, ?> descriptor = requiredMap(value, operation, "a route map");
    String id = keywordValue(HaraContext.lookupValue(descriptor, key("id")), operation, "route :id");
    List<String> path = strings(HaraContext.lookupValue(descriptor, key("path")), operation, "route :path");
    Object rawAliases = HaraContext.lookupValue(descriptor, key("aliases"));
    List<List<String>> aliases = new ArrayList<>();
    if (rawAliases != null) {
      for (Object alias : sequence(rawAliases, operation, "route :aliases")) {
        aliases.add(strings(alias, operation, "route :aliases"));
      }
    }
    Object desc = HaraContext.lookupValue(descriptor, key("desc"));
    if (!(desc instanceof String text)) throw failure(operation, "route :desc must be a string");
    Object handler = HaraContext.lookupValue(descriptor, key("handler"));
    if (!context.isFunctionValue(handler)) {
      throw failure(operation, "route :handler must be a function");
    }
    List<Option> options = new ArrayList<>();
    Object rawOptions = HaraContext.lookupValue(descriptor, key("options"));
    if (rawOptions != null) {
      for (Object option : sequence(rawOptions, operation, "route :options")) {
        options.add(option(option, operation));
      }
    }
    List<Argument> arguments = new ArrayList<>();
    Object rawArguments = HaraContext.lookupValue(descriptor, key("arguments"));
    if (rawArguments != null) {
      for (Object argument : sequence(rawArguments, operation, "route :arguments")) {
        arguments.add(argument(argument, operation));
      }
    }
    boolean passthrough =
        bool(
            HaraContext.lookupValue(descriptor, key("passthrough?")),
            false,
            operation,
            "route :passthrough?");
    return new Route(0, id, path, aliases, text, options, arguments, passthrough, handler);
  }

  private Option option(Object value, String operation) {
    IMapType<?, ?> descriptor = requiredMap(value, operation, "an :options map");
    String id = keywordValue(HaraContext.lookupValue(descriptor, key("id")), operation, "option :id");
    Object rawLong = HaraContext.lookupValue(descriptor, key("long"));
    String longName = rawLong == null ? "--" + id : string(rawLong, operation, "option :long");
    Object rawShort = HaraContext.lookupValue(descriptor, key("short"));
    Character shortName = null;
    if (rawShort != null) {
      String shortText = string(rawShort, operation, "option :short");
      if (shortText.codePointCount(0, shortText.length()) != 1) {
        throw failure(operation, "option :short must be one character");
      }
      shortName = shortText.charAt(0);
    }
    String type = keywordValue(HaraContext.lookupValue(descriptor, key("type")), operation, "option :type");
    if (!"boolean".equals(type) && !"string".equals(type)) {
      throw failure(operation, "option :type must be :boolean or :string");
    }
    boolean many = bool(HaraContext.lookupValue(descriptor, key("many?")), false, operation, "option :many?");
    Parsed defaultValue = null;
    Object rawDefault = HaraContext.lookupValue(descriptor, key("default"));
    if (rawDefault != null) {
      defaultValue = parsedDefault(rawDefault, type, many, operation, id);
    }
    return new Option(id, longName, shortName, type, many, defaultValue);
  }

  private Argument argument(Object value, String operation) {
    IMapType<?, ?> descriptor = requiredMap(value, operation, "an :arguments map");
    return new Argument(
        keywordValue(HaraContext.lookupValue(descriptor, key("id")), operation, "argument :id"),
        bool(HaraContext.lookupValue(descriptor, key("required?")), true, operation, "argument :required?"),
        bool(HaraContext.lookupValue(descriptor, key("many?")), false, operation, "argument :many?"));
  }

  private Invocation invocation(Object value, String operation) {
    IMapType<?, ?> map = requiredMap(value, operation, "an invocation map");
    List<String> argv = strings(HaraContext.lookupValue(map, key("argv")), operation, "invocation :argv");
    Object rawContext = HaraContext.lookupValue(map, key("context"));
    Object commandContext = rawContext == null ? hara.lang.data.Map.Standard.EMPTY : HaraBox.unwrap(rawContext);
    requiredMap(commandContext, operation, "invocation :context");
    return new Invocation(argv, commandContext);
  }

  private Parsed parsedDefault(Object value, String type, boolean many, String operation, String id) {
    Object raw = HaraBox.unwrap(value);
    if ("boolean".equals(type)) {
      if (many || !(raw instanceof Boolean booleanValue)) {
        throw failure(operation, "default for option " + id + " has the wrong type");
      }
      return Parsed.bool(booleanValue);
    }
    if (many) return Parsed.strings(strings(raw, operation, "option :default"));
    if (!(raw instanceof String stringValue)) {
      throw failure(operation, "default for option " + id + " has the wrong type");
    }
    return Parsed.string(stringValue);
  }

  private Object requestValue(long requestId, Request request, Object requestContext) {
    return map(
        key("app/id"), Symbol.create(request.appId),
        key("route/id"), key(request.routeId),
        key("route/path"), vector(request.routePath),
        key("argv"), vector(request.argv),
        key("arguments"), parsedMap(request.arguments),
        key("options"), parsedMap(request.options),
        key("context"), requestContext,
        key("command/request"), pointer("request", Map.of(key("id"), requestId)));
  }

  private Object routeValue(Route route) {
    List<Object> options = new ArrayList<>();
    for (Option option : route.options) {
      List<Object> entries = new ArrayList<>();
      entries.add(key("id")); entries.add(key(option.id));
      entries.add(key("long")); entries.add(option.longName);
      entries.add(key("short")); entries.add(option.shortName == null ? null : option.shortName.toString());
      entries.add(key("type")); entries.add(key(option.type));
      entries.add(key("many?")); entries.add(option.many);
      if (option.defaultValue != null) {
        entries.add(key("default"));
        entries.add(option.defaultValue.value());
      }
      options.add(map(entries.toArray()));
    }
    List<Object> arguments = new ArrayList<>();
    for (Argument argument : route.arguments) {
      arguments.add(map(
          key("id"), key(argument.id),
          key("required?"), argument.required,
          key("many?"), argument.many));
    }
    List<Object> aliases = new ArrayList<>();
    for (List<String> alias : route.aliases) aliases.add(vector(alias));
    return map(
        key("id"), key(route.id),
        key("path"), vector(route.path),
        key("aliases"), vector(aliases),
        key("desc"), route.desc,
        key("passthrough?"), route.passthrough,
        key("options"), vector(options),
        key("arguments"), vector(arguments));
  }

  private Object checkedResponse(Object value, String operation) {
    IMapType<?, ?> response = requiredMap(value, operation, "a response map");
    if (response.count() != 3) {
      throw failure(operation, "response must contain only :stdout, :stderr, and :exit");
    }
    String stdout = string(responseField(response, "stdout"), operation, "response :stdout");
    String stderr = string(responseField(response, "stderr"), operation, "response :stderr");
    Object rawExit = responseField(response, "exit");
    if (!(rawExit instanceof Number number)
        || number.doubleValue() != Math.rint(number.doubleValue())
        || number.longValue() < 0
        || number.longValue() > 255) {
      throw failure(operation, "response :exit must be an integer between 0 and 255");
    }
    return map(key("stdout"), stdout, key("stderr"), stderr, key("exit"), number.longValue());
  }

  private Object responseField(IMapType<?, ?> response, String name) {
    Object value = HaraContext.lookupValue(response, key(name));
    if (value == null) {
      throw failure("dispatch", "response is missing :" + name);
    }
    return HaraBox.unwrap(value);
  }

  private Object response(long exit, RuntimeException error) {
    String message = error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
    return map(key("stdout"), "", key("stderr"), message + "\n", key("exit"), exit);
  }

  private Object parsedMap(Map<String, Parsed> values) {
    List<Object> entries = new ArrayList<>();
    values.forEach((id, value) -> { entries.add(key(id)); entries.add(value.value()); });
    return map(entries.toArray());
  }

  private static Object map(Object... entries) {
    return hara.lang.data.Map.Standard.from(null, entries);
  }

  private static Object vector(List<?> values) {
    return hara.lang.data.Vector.Standard.from(null, values.toArray());
  }

  private static Object pointer(String kind, Map<Object, Object> fields) {
    return new Pointer(Keyword.create("command", kind), fields);
  }

  private static Keyword key(String value) {
    return Keyword.create(value);
  }

  private static IMapType<?, ?> requiredMap(Object value, String operation, String expectation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof IMapType<?, ?> map) return map;
    throw failure(operation, "expects " + expectation);
  }

  private static List<Object> sequence(Object value, String operation, String field) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof ILinearType<?> sequence)) {
      throw failure(operation, field + " must be a vector");
    }
    List<Object> values = new ArrayList<>();
    for (Object item : sequence) values.add(HaraBox.unwrap(item));
    return values;
  }

  private static List<String> strings(Object value, String operation, String field) {
    List<String> values = new ArrayList<>();
    for (Object item : sequence(value, operation, field)) values.add(string(item, operation, field));
    return values;
  }

  private static String string(Object value, String operation, String field) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof String text) return text;
    throw failure(operation, field + " must be a string");
  }

  private static String keywordValue(Object value, String operation, String field) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Keyword keyword && !keyword.getName().isBlank()) return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
    throw failure(operation, field + " must be a keyword");
  }

  private static boolean bool(Object value, boolean fallback, String operation, String field) {
    if (value == null) return fallback;
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Boolean bool) return bool;
    throw failure(operation, field + " must be a boolean");
  }

  private static void requireArity(String operation, Object[] values, int expected) {
    if (values.length != expected) {
      throw failure(operation, "expects " + expected + " argument" + (expected == 1 ? "" : "s"));
    }
  }

  private static HaraException failure(String operation, String message) {
    return new HaraException("std.native.Command/" + operation + " " + message);
  }

  private record AppConfig(String id, String desc) {}

  private record Invocation(List<String> argv, Object context) {}

  private record SnapshotRecord(long app, List<Route> routes) {}

  private record RequestRecord(long app, Request request, Object context) {}

  private record Argument(String id, boolean required, boolean many) {}

  private record Option(
      String id, String longName, Character shortName, String type, boolean many, Parsed defaultValue) {}

  private record Route(
      long handle,
      String id,
      List<String> path,
      List<List<String>> aliases,
      String desc,
      List<Option> options,
      List<Argument> arguments,
      boolean passthrough,
      Object handler) {
    Route withHandle(long handle) {
      return new Route(handle, id, List.copyOf(path), aliases.stream().map(List::copyOf).toList(), desc,
          List.copyOf(options), List.copyOf(arguments), passthrough, handler);
    }

    List<List<String>> paths() {
      List<List<String>> result = new ArrayList<>();
      result.add(path);
      result.addAll(aliases);
      return result;
    }
  }

  private record Request(
      String appId,
      long route,
      String routeId,
      List<String> routePath,
      List<String> argv,
      Map<String, Parsed> arguments,
      Map<String, Parsed> options,
      long generation) {}

  private record Parsed(Kind kind, Object raw) {
    enum Kind { BOOLEAN, STRING, STRINGS }

    static Parsed bool(boolean value) { return new Parsed(Kind.BOOLEAN, value); }
    static Parsed string(String value) { return new Parsed(Kind.STRING, value); }
    static Parsed strings(List<String> values) { return new Parsed(Kind.STRINGS, List.copyOf(values)); }

    Object value() {
      return kind == Kind.STRINGS ? vector((List<?>) raw) : raw;
    }
  }

  private static final class App {
    final long handle;
    final AppConfig config;
    final List<Route> routes = new ArrayList<>();
    long nextRoute = 1;
    long generation;
    boolean closed;

    App(long handle, AppConfig config) {
      this.handle = handle;
      this.config = config;
    }

    void requireOpen() {
      if (closed) throw commandFailure(":command/closed", "application is closed");
    }

    long install(Route candidate) {
      requireOpen();
      validateRoute(candidate);
      for (Route existing : routes) {
        if (existing.id.equals(candidate.id)) {
          throw commandFailure(":command/route-conflict", "route id is already installed: " + candidate.id);
        }
        for (List<String> path : existing.paths()) {
          for (List<String> proposed : candidate.paths()) {
            if (path.equals(proposed)) {
              throw commandFailure(
                  ":command/route-conflict",
                  "route " + candidate.id + " conflicts with " + existing.id + " at " + displayPath(path));
            }
          }
        }
      }
      long handle = nextRoute++;
      routes.add(candidate.withHandle(handle));
      generation++;
      return handle;
    }

    boolean uninstall(long handle) {
      requireOpen();
      boolean removed = routes.removeIf(route -> route.handle == handle);
      if (removed) generation++;
      return removed;
    }

    List<Route> snapshot() {
      requireOpen();
      return List.copyOf(routes);
    }

    void restore(List<Route> snapshot) {
      requireOpen();
      routes.clear();
      routes.addAll(snapshot);
      nextRoute = routes.stream().mapToLong(route -> route.handle).max().orElse(0) + 1;
      generation++;
    }

    void reset() {
      requireOpen();
      if (!routes.isEmpty()) {
        routes.clear();
        generation++;
      }
    }

    void close() {
      if (closed) return;
      routes.clear();
      generation++;
      closed = true;
    }

    Request parse(List<String> argv) {
      requireOpen();
      Match match = match(argv);
      ParsedRoute parsed = parseArguments(match.route, argv.subList(match.pathLength, argv.size()));
      return new Request(
          config.id,
          match.route.handle,
          match.route.id,
          match.route.path,
          List.copyOf(argv),
          parsed.arguments,
          parsed.options,
          generation);
    }

    Object handler(Request request) {
      requireOpen();
      if (!config.id.equals(request.appId)) {
        throw commandFailure(":command/foreign-request", "request belongs to a different application");
      }
      if (request.generation != generation) {
        throw commandFailure(":command/stale-request", "routes changed after this request was parsed");
      }
      for (Route route : routes) if (route.handle == request.route) return route.handler;
      throw commandFailure(":command/stale-request", "the matched route is no longer installed");
    }

    private Match match(List<String> argv) {
      Route matched = null;
      int length = -1;
      for (Route route : routes) {
        for (List<String> candidate : route.paths()) {
          if (candidate.isEmpty()) {
            if (argv.isEmpty()) { matched = route; length = 0; }
          } else if (argv.size() >= candidate.size() && argv.subList(0, candidate.size()).equals(candidate)
              && candidate.size() > length) {
            matched = route;
            length = candidate.size();
          }
        }
      }
      if (matched == null) {
        throw commandFailure(
            ":command/unknown-route",
            argv.isEmpty() ? "no root route is installed" : "unknown command: " + argv.get(0));
      }
      return new Match(matched, length);
    }
  }

  private record Match(Route route, int pathLength) {}

  private record ParsedRoute(Map<String, Parsed> options, Map<String, Parsed> arguments) {}

  private static ParsedRoute parseArguments(Route route, List<String> argv) {
    Map<String, Parsed> options = new LinkedHashMap<>();
    for (Option option : route.options) {
      options.put(option.id, option.defaultValue == null ? defaultValue(option) : option.defaultValue);
    }
    Set<String> supplied = new HashSet<>();
    if (route.passthrough) {
      return new ParsedRoute(Map.copyOf(options), parsePositionals(route, argv));
    }
    List<String> positional = new ArrayList<>();
    boolean optionsEnabled = true;
    for (int index = 0; index < argv.size();) {
      String value = argv.get(index);
      if (optionsEnabled && "--".equals(value)) {
        optionsEnabled = false;
        index++;
      } else if (optionsEnabled && value.startsWith("--") && value.length() > 2) {
        int equals = value.indexOf('=');
        String name = equals < 0 ? value : value.substring(0, equals);
        String inline = equals < 0 ? null : value.substring(equals + 1);
        Option option = route.options.stream().filter(candidate -> candidate.longName.equals(name)).findFirst()
            .orElseThrow(() -> commandFailure(":command/unknown-option", "unknown option: " + name));
        index = parseOption(option, inline, argv, index, options, supplied);
      } else if (optionsEnabled && value.startsWith("-") && value.length() == 2) {
        char name = value.charAt(1);
        Option option = route.options.stream().filter(candidate -> candidate.shortName != null && candidate.shortName == name)
            .findFirst()
            .orElseThrow(() -> commandFailure(":command/unknown-option", "unknown option: -" + name));
        index = parseOption(option, null, argv, index, options, supplied);
      } else {
        positional.add(value);
        index++;
      }
    }
    return new ParsedRoute(Map.copyOf(options), parsePositionals(route, positional));
  }

  private static Map<String, Parsed> parsePositionals(Route route, List<String> positional) {
    Map<String, Parsed> arguments = new LinkedHashMap<>();
    int cursor = 0;
    for (Argument argument : route.arguments) {
      if (argument.many) {
        List<String> values = positional.subList(cursor, positional.size());
        if (argument.required && values.isEmpty()) {
          throw commandFailure(":command/missing-argument", "missing argument: " + argument.id);
        }
        arguments.put(argument.id, Parsed.strings(values));
        cursor = positional.size();
        break;
      }
      if (cursor < positional.size()) {
        arguments.put(argument.id, Parsed.string(positional.get(cursor++)));
      } else if (argument.required) {
        throw commandFailure(":command/missing-argument", "missing argument: " + argument.id);
      } else {
        arguments.put(argument.id, Parsed.string(""));
      }
    }
    if (cursor != positional.size()) {
      throw commandFailure(":command/unexpected-argument", "unexpected argument: " + positional.get(cursor));
    }
    return Map.copyOf(arguments);
  }

  private static int parseOption(
      Option option,
      String inline,
      List<String> argv,
      int index,
      Map<String, Parsed> output,
      Set<String> supplied) {
    if ("boolean".equals(option.type)) {
      if (inline != null) {
        throw commandFailure(":command/invalid-option", option.longName + " does not accept a value");
      }
      if (!supplied.add(option.id)) {
        throw commandFailure(":command/duplicate-option", option.longName + " may be supplied only once");
      }
      output.put(option.id, Parsed.bool(true));
      return index + 1;
    }
    String value;
    if (inline != null) {
      value = inline;
    } else {
      if (index + 1 >= argv.size()) {
        throw commandFailure(":command/missing-option-value", option.longName + " requires a value");
      }
      value = argv.get(++index);
    }
    if (option.many) {
      Parsed current = output.get(option.id);
      List<String> values = new ArrayList<>((List<String>) current.raw);
      values.add(value);
      output.put(option.id, Parsed.strings(values));
    } else {
      if (!supplied.add(option.id)) {
        throw commandFailure(":command/duplicate-option", option.longName + " may be supplied only once");
      }
      output.put(option.id, Parsed.string(value));
    }
    return index + 1;
  }

  private static Parsed defaultValue(Option option) {
    return switch (option.type) {
      case "boolean" -> Parsed.bool(false);
      case "string" -> option.many ? Parsed.strings(List.of()) : Parsed.string("");
      default -> throw new IllegalStateException("validated option type");
    };
  }

  private static void validateRoute(Route route) {
    if (route.id.isBlank()) throw commandFailure(":command/invalid-route", ":id must be non-empty");
    if (route.desc.isBlank()) throw commandFailure(":command/invalid-route", ":desc must be non-empty");
    for (List<String> path : route.paths()) {
      if (path.stream().anyMatch(String::isBlank)) {
        throw commandFailure(":command/invalid-route", ":path and :aliases may not contain empty segments");
      }
    }
    Set<String> ids = new HashSet<>();
    Set<String> longs = new HashSet<>();
    Set<Character> shorts = new HashSet<>();
    for (Option option : route.options) {
      if (option.id.isBlank() || !ids.add(option.id)) {
        throw commandFailure(":command/invalid-route", "option ids must be unique and non-empty");
      }
      if (!option.longName.startsWith("--") || option.longName.length() <= 2 || !longs.add(option.longName)) {
        throw commandFailure(":command/invalid-route", "invalid or duplicate long option: " + option.longName);
      }
      if (option.shortName != null && !shorts.add(option.shortName)) {
        throw commandFailure(":command/invalid-route", "duplicate short option: -" + option.shortName);
      }
      if (option.many && "boolean".equals(option.type)) {
        throw commandFailure(":command/invalid-route", "boolean options may not be :many?");
      }
    }
    if (route.passthrough && !route.options.isEmpty()) {
      throw commandFailure(":command/invalid-route", ":passthrough? routes may not declare options");
    }
    Set<String> argumentIds = new HashSet<>();
    for (int index = 0; index < route.arguments.size(); index++) {
      Argument argument = route.arguments.get(index);
      if (argument.id.isBlank() || !argumentIds.add(argument.id)) {
        throw commandFailure(":command/invalid-route", "argument ids must be unique and non-empty");
      }
      if (argument.many && index + 1 != route.arguments.size()) {
        throw commandFailure(":command/invalid-route", "only the final positional argument may be :many?");
      }
    }
  }

  private static HaraException commandFailure(String code, String message) {
    return new HaraException(code + ": " + message);
  }

  private static String displayPath(List<String> path) {
    return path.isEmpty() ? "<root>" : String.join(" ", path);
  }
}
