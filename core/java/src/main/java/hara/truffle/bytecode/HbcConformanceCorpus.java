package hara.truffle.bytecode;

import hara.lang.base.G;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.graalvm.polyglot.Value;

/** Decoders for source-free bytecode conformance artifacts. */
public final class HbcConformanceCorpus {
  private static final byte[] MAGIC = {'H', 'C', 'C', '0'};
  private static final byte[] NATIVE_PROTOCOL_MAGIC = {'H', 'N', 'C', '1'};
  private static final String ERROR_EXPECTATION_PREFIX = "!error:";

  private HbcConformanceCorpus() {}

  public record Case(String id, String expectedDisplay, byte[] artifact) {
    public Case {
      artifact = artifact.clone();
    }

    @Override
    public byte[] artifact() {
      return artifact.clone();
    }
  }

  public record Suite(String id, byte[] setup, List<Case> cases) {
    public Suite {
      setup = setup.clone();
      cases = List.copyOf(cases);
    }

    @Override
    public byte[] setup() {
      return setup.clone();
    }
  }

  public static List<Case> decode(byte[] corpus) {
    if (corpus.length < 36 || !Arrays.equals(MAGIC, Arrays.copyOf(corpus, 4))) {
      throw new HbcFormatException("invalid bytecode conformance corpus header");
    }
    byte[] payload = Arrays.copyOfRange(corpus, 36, corpus.length);
    if (!MessageDigest.isEqual(Arrays.copyOfRange(corpus, 4, 36), sha256(payload))) {
      throw new HbcFormatException("bytecode conformance corpus checksum mismatch");
    }
    ByteBuffer input = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN);
    int count = size(input);
    ArrayList<Case> cases = new ArrayList<>(count);
    for (int i = 0; i < count; i++) {
      cases.add(new Case(text(input), text(input), bytes(input)));
    }
    if (input.hasRemaining()) {
      throw new HbcFormatException("trailing bytes in bytecode conformance corpus");
    }
    return List.copyOf(cases);
  }

  public static List<Suite> decodeNativeProtocol(byte[] corpus) {
    if (corpus.length < 36 || !Arrays.equals(NATIVE_PROTOCOL_MAGIC, Arrays.copyOf(corpus, 4))) {
      throw new HbcFormatException("invalid native/protocol conformance corpus header");
    }
    byte[] payload = Arrays.copyOfRange(corpus, 36, corpus.length);
    if (!MessageDigest.isEqual(Arrays.copyOfRange(corpus, 4, 36), sha256(payload))) {
      throw new HbcFormatException("native/protocol conformance corpus checksum mismatch");
    }
    ByteBuffer input = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN);
    int suiteCount = size(input);
    if (suiteCount != 2) {
      throw new HbcFormatException("native/protocol conformance corpus must contain two suites");
    }
    ArrayList<Suite> suites = new ArrayList<>(suiteCount);
    for (int suiteIndex = 0; suiteIndex < suiteCount; suiteIndex++) {
      String suiteId = text(input);
      byte[] setup = bytes(input);
      int caseCount = size(input);
      if (caseCount == 0) {
        throw new HbcFormatException("native/protocol conformance suite has no cases: " + suiteId);
      }
      ArrayList<Case> cases = new ArrayList<>(caseCount);
      for (int caseIndex = 0; caseIndex < caseCount; caseIndex++) {
        cases.add(new Case(text(input), text(input), bytes(input)));
      }
      suites.add(new Suite(suiteId, setup, cases));
    }
    if (input.hasRemaining()) {
      throw new HbcFormatException("trailing bytes in native/protocol conformance corpus");
    }
    if (!"native".equals(suites.get(0).id()) || !"protocol".equals(suites.get(1).id())) {
      throw new HbcFormatException("native/protocol conformance suites must be native then protocol");
    }
    return List.copyOf(suites);
  }

  /** Returns the Hara display form for a value exported through the polyglot boundary. */
  public static String display(Value value) {
    if (value.isNull()) return "nil";
    if (value.isString()) return G.display(value.asString());
    if (value.isNumber()) {
      Object exported = value.as(Object.class);
      if (exported instanceof Double || exported instanceof Float) return G.display(exported);
    }
    return value.toString();
  }

  /** HNC1 reserves a textual expectation prefix for a normalized runtime failure. */
  public static String expectedErrorCategory(String expectation) {
    return expectation.startsWith(ERROR_EXPECTATION_PREFIX)
        ? expectation.substring(ERROR_EXPECTATION_PREFIX.length())
        : null;
  }

  /** Maps portable protocol prefixes and normalized native argument failures. */
  public static String normalizedErrorCategory(Throwable failure) {
    for (Throwable current = failure; current != null; current = current.getCause()) {
      String message = current.getMessage();
      if (message == null) continue;
      int protocolArity = message.indexOf("protocol/arity:");
      if (protocolArity >= 0) return "protocol/arity";
      int unsupported = message.indexOf("protocol/unsupported-receiver:");
      if (unsupported >= 0) return "protocol/unsupported-receiver";
      if (message.contains("Ex$Arity") || message.contains("Wrong number of args")) {
        return "native/arity";
      }
      if (message.startsWith("Expected ") && message.contains(" arguments, received ")) {
        return "native/arity";
      }
      if (message.contains("expects")) {
        boolean arity =
            message.contains("expects no ")
                || message.contains("expects one ")
                || message.contains("expects two ")
                || message.contains("expects three ")
                || message.contains("expects four ")
                || message.contains("expects at least ");
        if (arity) return "native/arity";
        if (message.contains("number")
            || message.contains("numeric")
            || message.contains("integer")
            || message.contains("string")) {
          return "native/type";
        }
        return "native/arity";
      }
    }
    return null;
  }

  private static String text(ByteBuffer input) {
    return new String(bytes(input), StandardCharsets.UTF_8);
  }

  private static byte[] bytes(ByteBuffer input) {
    int size = size(input);
    if (input.remaining() < size) {
      throw new HbcFormatException("truncated bytecode conformance corpus");
    }
    byte[] value = new byte[size];
    input.get(value);
    return value;
  }

  private static int size(ByteBuffer input) {
    if (input.remaining() < 4) {
      throw new HbcFormatException("truncated bytecode conformance corpus");
    }
    int value = input.getInt();
    if (value < 0) throw new HbcFormatException("bytecode conformance corpus length overflow");
    return value;
  }

  private static byte[] sha256(byte[] value) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(value);
    } catch (NoSuchAlgorithmException impossible) {
      throw new AssertionError(impossible);
    }
  }
}
