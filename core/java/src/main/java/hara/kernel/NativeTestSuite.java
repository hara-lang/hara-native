package hara.kernel;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/** Parser for the host-owned, language-neutral native test-suite format. */
final class NativeTestSuite {
  static final String FORMAT = "hara-native/test-suite/1";

  record Case(String group, String id, String source, Expected expected) {}

  record Expected(String value, boolean error) {}

  private NativeTestSuite() {}

  static Map<String, List<Case>> read(Path path) throws IOException {
    Object document = new Json(Files.readString(path, StandardCharsets.UTF_8)).read();
    Map<String, Object> root = object(document, "native test suite");
    if (!FORMAT.equals(string(root, "format", "native test suite"))) {
      throw new IllegalArgumentException("native test suite format must be " + FORMAT);
    }
    Map<String, Object> groups = object(root.get("groups"), "native test suite groups");
    if (groups.isEmpty()) throw new IllegalArgumentException("native test suite must declare at least one group");
    Map<String, List<Case>> parsed = new TreeMap<>();
    for (Map.Entry<String, Object> entry : groups.entrySet()) {
      String group = entry.getKey();
      if (group.isBlank()) throw new IllegalArgumentException("native test group names must be non-empty");
      List<Object> values = array(entry.getValue(), "native test group " + group);
      if (values.isEmpty()) throw new IllegalArgumentException("native test group " + group + " must not be empty");
      List<Case> cases = new ArrayList<>();
      for (Object value : values) {
        Map<String, Object> test = object(value, "native test group " + group + " case");
        String id = string(test, "id", "native test group " + group + " case");
        String source = string(test, "source", "native test " + group + "/" + id);
        boolean hasValue = test.containsKey("expect");
        boolean hasError = test.containsKey("error");
        if (hasValue == hasError) {
          throw new IllegalArgumentException(
              "native test " + group + "/" + id + " must declare exactly one of expect or error");
        }
        String expected = string(test, hasValue ? "expect" : "error", "native test " + group + "/" + id);
        cases.add(new Case(group, id, source, new Expected(expected, hasError)));
      }
      parsed.put(group, List.copyOf(cases));
    }
    return Map.copyOf(parsed);
  }

  static List<Case> select(Map<String, List<Case>> suite, List<String> requested) {
    List<String> groups = requested.isEmpty() ? new ArrayList<>(suite.keySet()) : requested;
    List<Case> selected = new ArrayList<>();
    for (String group : groups) {
      List<Case> cases = suite.get(group);
      if (cases == null) throw new IllegalArgumentException("native test group is unknown: " + group);
      selected.addAll(cases);
    }
    if (selected.isEmpty()) throw new IllegalArgumentException("native test selection is empty");
    return List.copyOf(selected);
  }

  @SuppressWarnings("unchecked")
  private static Map<String, Object> object(Object value, String context) {
    if (!(value instanceof Map<?, ?> map)) {
      throw new IllegalArgumentException(context + " must be a JSON object");
    }
    TreeMap<String, Object> result = new TreeMap<>();
    for (Map.Entry<?, ?> entry : map.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw new IllegalArgumentException(context + " keys must be strings");
      }
      result.put(key, entry.getValue());
    }
    return result;
  }

  @SuppressWarnings("unchecked")
  private static List<Object> array(Object value, String context) {
    if (!(value instanceof List<?> values)) {
      throw new IllegalArgumentException(context + " must be a JSON array");
    }
    return (List<Object>) values;
  }

  private static String string(Map<String, Object> value, String field, String context) {
    Object result = value.get(field);
    if (!(result instanceof String string) || string.isEmpty()) {
      throw new IllegalArgumentException(context + " requires a non-empty " + field + " string");
    }
    return string;
  }

  /** Small JSON reader: suites contain only objects, arrays, and strings. */
  private static final class Json {
    private final String source;
    private int offset;

    Json(String source) {
      this.source = source;
    }

    Object read() {
      Object result = value();
      whitespace();
      if (offset != source.length()) throw error("unexpected trailing input");
      return result;
    }

    private Object value() {
      whitespace();
      if (offset == source.length()) throw error("expected value");
      return switch (source.charAt(offset)) {
        case '{' -> object();
        case '[' -> array();
        case '"' -> string();
        default -> throw error("expected object, array, or string");
      };
    }

    private Map<String, Object> object() {
      expect('{');
      TreeMap<String, Object> result = new TreeMap<>();
      whitespace();
      if (consume('}')) return result;
      while (true) {
        whitespace();
        if (offset == source.length() || source.charAt(offset) != '"') throw error("expected object key");
        String key = string();
        whitespace();
        expect(':');
        Object previous = result.put(key, value());
        if (previous != null) throw error("duplicate object key " + key);
        whitespace();
        if (consume('}')) return result;
        expect(',');
      }
    }

    private List<Object> array() {
      expect('[');
      List<Object> result = new ArrayList<>();
      whitespace();
      if (consume(']')) return result;
      while (true) {
        result.add(value());
        whitespace();
        if (consume(']')) return result;
        expect(',');
      }
    }

    private String string() {
      expect('"');
      StringBuilder result = new StringBuilder();
      while (offset < source.length()) {
        char current = source.charAt(offset++);
        if (current == '"') return result.toString();
        if (current != '\\') {
          result.append(current);
          continue;
        }
        if (offset == source.length()) throw error("unterminated escape");
        char escaped = source.charAt(offset++);
        switch (escaped) {
          case '"', '\\', '/' -> result.append(escaped);
          case 'b' -> result.append('\b');
          case 'f' -> result.append('\f');
          case 'n' -> result.append('\n');
          case 'r' -> result.append('\r');
          case 't' -> result.append('\t');
          case 'u' -> {
            if (offset + 4 > source.length()) throw error("incomplete unicode escape");
            try {
              result.append((char) Integer.parseInt(source.substring(offset, offset + 4), 16));
            } catch (NumberFormatException error) {
              throw error("invalid unicode escape");
            }
            offset += 4;
          }
          default -> throw error("invalid escape");
        }
      }
      throw error("unterminated string");
    }

    private void whitespace() {
      while (offset < source.length() && Character.isWhitespace(source.charAt(offset))) offset++;
    }

    private boolean consume(char expected) {
      if (offset < source.length() && source.charAt(offset) == expected) {
        offset++;
        return true;
      }
      return false;
    }

    private void expect(char expected) {
      whitespace();
      if (!consume(expected)) throw error("expected '" + expected + "'");
    }

    private IllegalArgumentException error(String message) {
      return new IllegalArgumentException("native test suite JSON at " + offset + ": " + message);
    }
  }
}
