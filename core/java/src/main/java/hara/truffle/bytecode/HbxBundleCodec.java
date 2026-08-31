package hara.truffle.bytecode;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.TreeMap;

/** Decoder for Rust's deterministic HBX0 indexed standard-library container. */
public final class HbxBundleCodec {
  private static final byte[] MAGIC = {'H', 'B', 'X', '0'};

  private HbxBundleCodec() {}

  public record Module(
      String resource,
      String namespaceForm,
      byte[] sourceDigest,
      List<String> dependencies,
      boolean eager,
      byte[] artifact) {
    public Module {
      sourceDigest = sourceDigest.clone();
      dependencies = List.copyOf(dependencies);
      artifact = artifact.clone();
    }

    @Override
    public byte[] sourceDigest() {
      return sourceDigest.clone();
    }

    @Override
    public byte[] artifact() {
      return artifact.clone();
    }
  }

  public static List<Module> decode(byte[] bundle) {
    if (bundle.length < 36 || !Arrays.equals(MAGIC, Arrays.copyOf(bundle, 4))) {
      throw new HbcFormatException("invalid HBX0 bytecode bundle header");
    }
    byte[] payload = Arrays.copyOfRange(bundle, 36, bundle.length);
    if (!MessageDigest.isEqual(Arrays.copyOfRange(bundle, 4, 36), sha256(payload))) {
      throw new HbcFormatException("HBX0 bytecode bundle checksum mismatch");
    }
    ByteBuffer input = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN);
    int count = size(input);
    ArrayList<Module> modules = new ArrayList<>(count);
    for (int i = 0; i < count; i++) {
      String resource = text(input);
      String namespaceForm = text(input);
      byte[] sourceDigest = fixed(input, 32);
      int dependencyCount = size(input);
      ArrayList<String> dependencies = new ArrayList<>(dependencyCount);
      for (int dependency = 0; dependency < dependencyCount; dependency++) {
        dependencies.add(text(input));
      }
      if (!input.hasRemaining()) {
        throw new HbcFormatException("truncated HBX0 bytecode bundle");
      }
      int eager = Byte.toUnsignedInt(input.get());
      if (eager > 1) {
        throw new HbcFormatException("HBX0 bytecode bundle contains invalid eager flag");
      }
      modules.add(
          new Module(
              resource,
              namespaceForm,
              sourceDigest,
              dependencies,
              eager == 1,
              bytes(input)));
    }
    if (input.hasRemaining()) throw new HbcFormatException("trailing bytes in HBX0 bytecode bundle");
    validate(modules);
    return List.copyOf(modules);
  }

  /** Encodes module descriptors into the deterministic HBX0 interchange container. */
  public static byte[] encode(List<Module> modules) {
    List<Module> canonical = canonicalModules(modules);
    ByteArrayOutputStream payload = new ByteArrayOutputStream();
    writeSize(payload, canonical.size());
    for (Module module : canonical) {
      writeBytes(payload, module.resource().getBytes(StandardCharsets.UTF_8));
      writeBytes(payload, module.namespaceForm().getBytes(StandardCharsets.UTF_8));
      payload.writeBytes(module.sourceDigest());
      writeSize(payload, module.dependencies().size());
      for (String dependency : module.dependencies()) {
        writeBytes(payload, dependency.getBytes(StandardCharsets.UTF_8));
      }
      payload.write(module.eager() ? 1 : 0);
      writeBytes(payload, module.artifact());
    }
    byte[] encodedPayload = payload.toByteArray();
    ByteArrayOutputStream output = new ByteArrayOutputStream(MAGIC.length + 32 + encodedPayload.length);
    output.writeBytes(MAGIC);
    output.writeBytes(sha256(encodedPayload));
    output.writeBytes(encodedPayload);
    return output.toByteArray();
  }

  private static List<Module> canonicalModules(List<Module> modules) {
    TreeMap<String, Module> remaining = new TreeMap<>();
    for (Module module : modules) {
      ArrayList<String> dependencies = new ArrayList<>(module.dependencies());
      dependencies.sort(String::compareTo);
      for (int index = 1; index < dependencies.size(); index++) {
        if (dependencies.get(index - 1).equals(dependencies.get(index))) {
          throw new HbcFormatException(module.resource() + ": duplicate HBX0 dependency");
        }
      }
      Module normalized =
          new Module(
              module.resource(),
              module.namespaceForm(),
              module.sourceDigest(),
              dependencies,
              module.eager(),
              module.artifact());
      if (remaining.put(normalized.resource(), normalized) != null) {
        throw new HbcFormatException("duplicate HBX0 module: " + normalized.resource());
      }
    }
    ArrayList<Module> ordered = new ArrayList<>(remaining.size());
    while (!remaining.isEmpty()) {
      String available = null;
      for (Map.Entry<String, Module> entry : remaining.entrySet()) {
        boolean ready = true;
        for (String dependency : entry.getValue().dependencies()) {
          if (remaining.containsKey(dependency)) {
            ready = false;
            break;
          }
        }
        if (ready) {
          available = entry.getKey();
          break;
        }
      }
      if (available == null) {
        throw new HbcFormatException("HBX0 module dependencies contain a cycle");
      }
      ordered.add(remaining.remove(available));
    }
    validate(ordered);
    return List.copyOf(ordered);
  }

  private static void validate(List<Module> modules) {
    HashMap<String, Integer> positions = new HashMap<>();
    for (int index = 0; index < modules.size(); index++) {
      Module module = modules.get(index);
      if (module.resource().isEmpty()) throw new HbcFormatException("HBX0 module resource must not be empty");
      if (module.namespaceForm().isEmpty()) throw new HbcFormatException(module.resource() + ": HBX0 namespace form must not be empty");
      if (module.sourceDigest().length != 32) {
        throw new HbcFormatException(module.resource() + ": HBX0 source digest must be 32 bytes");
      }
      if (positions.put(module.resource(), index) != null) throw new HbcFormatException("duplicate HBX0 module: " + module.resource());
      HashSet<String> dependencies = new HashSet<>();
      String previous = null;
      for (String dependency : module.dependencies()) {
        if (!dependencies.add(dependency) || (previous != null && previous.compareTo(dependency) >= 0)) {
          throw new HbcFormatException(module.resource() + ": HBX0 dependencies must be unique and sorted");
        }
        previous = dependency;
      }
      try {
        HbcCodec.decode(module.artifact());
      } catch (RuntimeException error) {
        throw new HbcFormatException(module.resource() + ": invalid HBC0 artifact: " + error.getMessage());
      }
    }
    for (int index = 0; index < modules.size(); index++) {
      Module module = modules.get(index);
      for (String dependency : module.dependencies()) {
        Integer position = positions.get(dependency);
        if (position != null && position >= index) throw new HbcFormatException(module.resource() + ": HBX0 dependency must appear first: " + dependency);
      }
    }
  }

  private static String text(ByteBuffer input) {
    return new String(bytes(input), StandardCharsets.UTF_8);
  }

  private static byte[] bytes(ByteBuffer input) {
    int size = size(input);
    if (input.remaining() < size) throw new HbcFormatException("truncated HBX0 bytecode bundle");
    byte[] value = new byte[size];
    input.get(value);
    return value;
  }

  private static byte[] fixed(ByteBuffer input, int size) {
    if (input.remaining() < size) {
      throw new HbcFormatException("truncated HBX0 bytecode bundle");
    }
    byte[] value = new byte[size];
    input.get(value);
    return value;
  }

  private static int size(ByteBuffer input) {
    if (input.remaining() < 4) throw new HbcFormatException("truncated HBX0 bytecode bundle");
    int value = input.getInt();
    if (value < 0) throw new HbcFormatException("HBX0 bytecode bundle length overflow");
    return value;
  }

  private static void writeSize(ByteArrayOutputStream output, int value) {
    if (value < 0) throw new HbcFormatException("HBX0 bytecode bundle length overflow");
    output.write(value & 0xff);
    output.write((value >>> 8) & 0xff);
    output.write((value >>> 16) & 0xff);
    output.write((value >>> 24) & 0xff);
  }

  private static void writeBytes(ByteArrayOutputStream output, byte[] value) {
    writeSize(output, value.length);
    output.writeBytes(value);
  }

  private static byte[] sha256(byte[] value) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(value);
    } catch (NoSuchAlgorithmException impossible) {
      throw new AssertionError(impossible);
    }
  }
}
