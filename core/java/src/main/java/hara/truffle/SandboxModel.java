package hara.truffle;

import java.util.Objects;
import java.util.List;

/** Immutable data contracts for Kernel-managed sandboxes. */
final class SandboxModel {
  static final String SPEC_PROTOCOL = "hara.sandbox/0-alpha";

  private SandboxModel() {}

  record SandboxId(long value) implements Comparable<SandboxId> {
    SandboxId {
      if (value <= 0) throw new IllegalArgumentException("INVALID_SANDBOX_ID");
    }

    @Override
    public int compareTo(SandboxId other) {
      return Long.compare(value, other.value);
    }

    @Override
    public String toString() {
      return Long.toString(value);
    }
  }

  record EvaluationId(long value) {
    EvaluationId {
      if (value <= 0) throw new IllegalArgumentException("INVALID_EVALUATION_ID");
    }
  }

  enum SandboxState {
    OPEN,
    RUNNING,
    CANCELLING,
    CANCELLED,
    FAILED,
    CLOSED
  }

  record SandboxLimits(
      int sourceBytes,
      int resultBytes,
      int outputBytes,
      long evaluationMillis,
      long memoryBytes,
      int activeEvaluations) {
    SandboxLimits {
      if (sourceBytes <= 0
          || resultBytes <= 0
          || outputBytes <= 0
          || evaluationMillis <= 0
          || memoryBytes <= 0
          || activeEvaluations != 1) {
        throw new SandboxException(ErrorCode.INVALID_SPEC, "invalid sandbox limits");
      }
    }

    static SandboxLimits defaults() {
      return new SandboxLimits(
          64 * 1024, 1024 * 1024, 1024 * 1024, 5_000, 64L * 1024 * 1024, 1);
    }
  }

  record SandboxSpec(
      String protocol,
      String provider,
      String runtime,
      String entryNamespace,
      List<BundleReference> bundles,
      SessionModel.SessionMountId mount,
      Object providerOptions,
      SandboxLimits limits) {
    SandboxSpec {
      Objects.requireNonNull(protocol, "protocol");
      Objects.requireNonNull(provider, "provider");
      Objects.requireNonNull(runtime, "runtime");
      Objects.requireNonNull(entryNamespace, "entryNamespace");
      bundles = List.copyOf(bundles);
      Objects.requireNonNull(providerOptions, "providerOptions");
      Objects.requireNonNull(limits, "limits");
      if (!SPEC_PROTOCOL.equals(protocol)) {
        throw new SandboxException(ErrorCode.INVALID_SPEC, "unsupported sandbox protocol");
      }
      if (provider.isEmpty() || runtime.isEmpty()) {
        throw new SandboxException(ErrorCode.INVALID_SPEC, "provider and runtime are required");
      }
      try {
        SessionModel.SessionId.parse(entryNamespace);
      } catch (IllegalArgumentException error) {
        throw new SandboxException(ErrorCode.INVALID_SPEC, "invalid entry namespace");
      }
    }

    SandboxSpec(
        String protocol,
        String provider,
        String runtime,
        String entryNamespace,
        SandboxLimits limits) {
      this(
          protocol,
          provider,
          runtime,
          entryNamespace,
          List.of(),
          null,
          HaraPersistentValues.normalize(java.util.Map.of()),
          limits);
    }

    static SandboxSpec inProcess() {
      return new SandboxSpec(
          SPEC_PROTOCOL,
          "in-process",
          "hara.standard/0-alpha",
          "user",
          SandboxLimits.defaults());
    }
  }

  record BundleReference(String digest, String format) {
    BundleReference {
      if (digest == null
          || !digest.matches("sha256:[0-9a-f]{64}")
          || format == null
          || format.isEmpty()) {
        throw new SandboxException(ErrorCode.INVALID_SPEC, "invalid sandbox bundle reference");
      }
    }
  }

  record SandboxError(ErrorCode code, String message) {}

  record SandboxStatus(
      SandboxId id,
      String provider,
      SandboxState state,
      boolean secure,
      boolean evaluationActive,
      SandboxError error) {}

  enum ErrorCode {
    INVALID_SPEC,
    PROVIDER_NOT_FOUND,
    PROVIDER_UNAVAILABLE,
    BUNDLE_NOT_FOUND,
    BUNDLE_DIGEST_MISMATCH,
    MOUNT_NOT_FOUND,
    NOT_FOUND,
    CLOSED,
    BUSY,
    CANCELLED,
    TIMEOUT,
    LIMIT_EXCEEDED,
    EVALUATION_FAILED,
    RESULT_NOT_TRANSFERABLE,
    TRANSPORT_FAILED,
    PROVIDER_FAILED,
    UNSUPPORTED
  }

  static final class SandboxException extends RuntimeException {
    private final ErrorCode code;

    SandboxException(ErrorCode code, String message) {
      super(message);
      this.code = code;
    }

    ErrorCode code() {
      return code;
    }
  }
}
