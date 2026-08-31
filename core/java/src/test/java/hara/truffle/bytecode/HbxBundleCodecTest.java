package hara.truffle.bytecode;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;

import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Opcode;
import java.util.Arrays;
import java.util.List;
import org.junit.Test;

public class HbxBundleCodecTest {
  @Test
  public void canonicalizesModuleOrderAndRoundTripsEveryDescriptor() {
    byte[] artifact = HbcCodec.encode(program());
    HbxBundleCodec.Module dependent =
        new HbxBundleCodec.Module(
            "demo/b", "(ns demo.b)", digest(2), List.of("demo/a"), false, artifact);
    HbxBundleCodec.Module dependency =
        new HbxBundleCodec.Module("demo/a", "(ns demo.a)", digest(1), List.of(), true, artifact);

    byte[] encoded = HbxBundleCodec.encode(List.of(dependent, dependency));
    List<HbxBundleCodec.Module> decoded = HbxBundleCodec.decode(encoded);

    assertEquals(List.of("demo/a", "demo/b"), decoded.stream().map(HbxBundleCodec.Module::resource).toList());
    assertEquals(List.of("demo/a"), decoded.get(1).dependencies());
    assertArrayEquals(encoded, HbxBundleCodec.encode(decoded));
  }

  private static HbcProgram program() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(Instruction.of(Opcode.NIL), Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null),
            List.of());
    return new HbcProgram(List.of(), List.of(), List.of(entry), 0);
  }

  private static byte[] digest(int value) {
    byte[] digest = new byte[32];
    digest[0] = (byte) value;
    return digest;
  }
}
