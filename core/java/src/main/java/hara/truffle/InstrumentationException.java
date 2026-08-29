package hara.truffle;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;

/** Structured failure emitted by the Java instrumentation authority boundary. */
final class InstrumentationException extends IllegalArgumentException {
  private static final long serialVersionUID = 1L;

  enum Code {
    RUNTIME_CLOSED,
    INVALID_REGISTRATION,
    TRANSFORM_DEFERRED,
    INSTRUMENT_EXISTS,
    INSTRUMENT_NOT_FOUND,
    STALE_INSTRUMENT,
    TARGET_EXISTS,
    TARGET_NOT_FOUND,
    STALE_TARGET,
    CROSS_SESSION,
    CROSS_RUNTIME,
    FILTER_REJECTED,
    EVENT_TARGET_MISMATCH,
    UNSUPPORTED_CAPABILITIES,
    ATTACHMENT_NOT_FOUND,
    CONTROL_MODE_REQUIRED,
    CONTROL_LEASE_CONFLICT,
    INVALID_CONTROL_LEASE,
    UNSUPPORTED_DIRECTIVE,
    SESSION_CLOSED
  }

  private final Code code;
  private final transient Map<String, Object> evidence;

  InstrumentationException(Code code, String message) {
    this(code, message, Map.of(), null);
  }

  InstrumentationException(Code code, String message, Map<String, ?> evidence) {
    this(code, message, evidence, null);
  }

  InstrumentationException(
      Code code, String message, Map<String, ?> evidence, Throwable cause) {
    super(Objects.requireNonNull(message, "message"), cause);
    this.code = Objects.requireNonNull(code, "code");
    LinkedHashMap<String, Object> frozen = new LinkedHashMap<>();
    if (evidence != null) {
      for (Map.Entry<String, ?> entry : evidence.entrySet()) {
        frozen.put(
            Objects.requireNonNull(entry.getKey(), "evidence key"), entry.getValue());
      }
    }
    this.evidence = Collections.unmodifiableMap(frozen);
  }

  Code code() {
    return code;
  }

  Map<String, Object> evidence() {
    return evidence;
  }
}
