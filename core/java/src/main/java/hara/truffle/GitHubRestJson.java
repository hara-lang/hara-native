package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Strict JSON bridge for the GitHub REST client without another JSON dependency. */
final class GitHubRestJson {
  private GitHubRestJson() {}

  static JsonValue.Object object(byte[] bytes) {
    JsonValue value =
        StrictJson.parseValue(new String(bytes, StandardCharsets.UTF_8));
    if (!(value instanceof JsonValue.Object object)) {
      throw new IllegalArgumentException("GitHub response must be a JSON object");
    }
    return object;
  }

  static JsonValue.Object object(JsonValue.Object object, String key) {
    JsonValue value = require(object, key);
    if (!(value instanceof JsonValue.Object nested)) {
      throw new IllegalArgumentException("GitHub response field " + key + " must be an object");
    }
    return nested;
  }

  static List<JsonValue> array(JsonValue.Object object, String key) {
    JsonValue value = require(object, key);
    if (!(value instanceof JsonValue.Array array)) {
      throw new IllegalArgumentException("GitHub response field " + key + " must be an array");
    }
    return array.values();
  }

  static String string(JsonValue.Object object, String key) {
    JsonValue value = require(object, key);
    if (!(value instanceof JsonValue.String string)) {
      throw new IllegalArgumentException("GitHub response field " + key + " must be a string");
    }
    return string.value();
  }

  static String optionalString(JsonValue.Object object, String key) {
    JsonValue value = object.values().get(key);
    if (value == null || value instanceof JsonValue.Null) return null;
    if (!(value instanceof JsonValue.String string)) {
      throw new IllegalArgumentException("GitHub response field " + key + " must be a string");
    }
    return string.value();
  }

  static Long optionalLong(JsonValue.Object object, String key) {
    JsonValue value = object.values().get(key);
    if (value == null || value instanceof JsonValue.Null) return null;
    if (!(value instanceof JsonValue.Integer integer)) {
      throw new IllegalArgumentException("GitHub response field " + key + " must be an integer");
    }
    return integer.value();
  }

  static boolean bool(JsonValue.Object object, String key) {
    JsonValue value = require(object, key);
    if (!(value instanceof JsonValue.Bool bool)) {
      throw new IllegalArgumentException("GitHub response field " + key + " must be boolean");
    }
    return bool.value();
  }

  static JsonValue.Object asObject(JsonValue value, String label) {
    if (!(value instanceof JsonValue.Object object)) {
      throw new IllegalArgumentException(label + " must be an object");
    }
    return object;
  }

  static byte[] encode(Object value) {
    StringBuilder output = new StringBuilder();
    append(output, value);
    return output.toString().getBytes(StandardCharsets.UTF_8);
  }

  private static JsonValue require(JsonValue.Object object, String key) {
    JsonValue value = object.values().get(key);
    if (value == null) throw new IllegalArgumentException("GitHub response requires field " + key);
    return value;
  }

  private static void append(StringBuilder output, Object value) {
    if (value == null) {
      output.append("null");
      return;
    }
    if (value instanceof Boolean || value instanceof Byte || value instanceof Short
        || value instanceof Integer || value instanceof Long) {
      output.append(value);
      return;
    }
    if (value instanceof String string) {
      appendString(output, string);
      return;
    }
    if (value instanceof List<?> list) {
      output.append('[');
      for (int index = 0; index < list.size(); index++) {
        if (index > 0) output.append(',');
        append(output, list.get(index));
      }
      output.append(']');
      return;
    }
    if (value instanceof Map<?, ?> map) {
      output.append('{');
      int index = 0;
      for (Map.Entry<?, ?> entry : map.entrySet()) {
        if (!(entry.getKey() instanceof String key)) {
          throw new IllegalArgumentException("GitHub JSON object keys must be strings");
        }
        if (index++ > 0) output.append(',');
        appendString(output, key);
        output.append(':');
        append(output, entry.getValue());
      }
      output.append('}');
      return;
    }
    throw new IllegalArgumentException(
        "unsupported GitHub JSON value " + value.getClass().getName());
  }

  private static void appendString(StringBuilder output, String value) {
    JsonValue.requireValidUnicode(value);
    output.append('"');
    for (int index = 0; index < value.length(); index++) {
      char current = value.charAt(index);
      switch (current) {
        case '"' -> output.append("\\\"");
        case '\\' -> output.append("\\\\");
        case '\b' -> output.append("\\b");
        case '\f' -> output.append("\\f");
        case '\n' -> output.append("\\n");
        case '\r' -> output.append("\\r");
        case '\t' -> output.append("\\t");
        default -> {
          if (current < 0x20) output.append(String.format("\\u%04x", (int) current));
          else output.append(current);
        }
      }
    }
    output.append('"');
  }

  static Map<String, Object> objectMap(Object... entries) {
    if (entries.length % 2 != 0) throw new IllegalArgumentException("JSON map entries are unpaired");
    LinkedHashMap<String, Object> values = new LinkedHashMap<>();
    for (int index = 0; index < entries.length; index += 2) {
      String key = (String) entries[index];
      if (values.putIfAbsent(key, entries[index + 1]) != null) {
        throw new IllegalArgumentException("duplicate JSON key " + key);
      }
    }
    return values;
  }

  static List<Object> arrayList(Object... values) {
    ArrayList<Object> output = new ArrayList<>(values.length);
    java.util.Collections.addAll(output, values);
    return output;
  }
}
