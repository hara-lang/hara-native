package hara.truffle.bytecode;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertThrows;

import hara.lang.data.Symbol;
import hara.truffle.HaraContext;
import hara.truffle.HaraLanguage;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Opcode;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import org.graalvm.polyglot.Context;
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

  @Test
  public void installsEagerAndDemandLoadsLazyNamespacesIntoContext() {
    HbxBundleCodec.Module eager =
        new HbxBundleCodec.Module(
            "fixture/hbx/eager",
            "(ns fixture.hbx.eager)",
            digest(3),
            List.of(),
            true,
            HbcCodec.encode(definitionProgram("fixture.hbx.eager", "value", 41L)));
    HbxBundleCodec.Module lazy =
        new HbxBundleCodec.Module(
            "fixture/hbx/lazy",
            "(ns fixture.hbx.lazy)",
            digest(4),
            List.of("fixture/hbx/eager"),
            false,
            HbcCodec.encode(
                referenceProgram(
                    "fixture.hbx.lazy", "fixture.hbx.eager/value", "from-eager")));
    byte[] bundle = HbxBundleCodec.encode(List.of(lazy, eager));

    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        context.installBytecodeBundle(bundle);

        assertEquals(41L, context.resolve(Symbol.create("fixture.hbx.eager/value")).deref());
        assertEquals(41L, context.resolve(Symbol.create("fixture.hbx.lazy/from-eager")).deref());
      } finally {
        polyglot.leave();
      }
    }
  }

  @Test
  public void failedEagerInstallRestoresTheContextAndBundleIndex() {
    HbxBundleCodec.Module good =
        new HbxBundleCodec.Module(
            "fixture.hbx.rollback.good",
            "(ns fixture.hbx.rollback.good)",
            digest(5),
            List.of(),
            true,
            HbcCodec.encode(definitionProgram("fixture.hbx.rollback.good", "marker", 42L)));
    HbxBundleCodec.Module bad =
        new HbxBundleCodec.Module(
            "fixture.hbx.rollback.later",
            "(ns fixture.hbx.rollback.later)",
            digest(6),
            List.of("fixture.hbx.rollback.good"),
            true,
            HbcCodec.encode(
                referenceProgram(
                    "fixture.hbx.rollback.later", "missing.bundle/value", "unreachable")));
    byte[] bundle = HbxBundleCodec.encode(List.of(bad, good));

    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        assertThrows(RuntimeException.class, () -> context.installBytecodeBundle(bundle));
        assertNull(context.resolve(Symbol.create("fixture.hbx.rollback.good/marker")));
      } finally {
        polyglot.leave();
      }
    }
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

  private static HbcProgram definitionProgram(String namespace, String name, long value) {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.DEF_GLOBAL, 1, -1, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    return new HbcProgram(
        namespace,
        List.of(value, name),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static HbcProgram referenceProgram(String namespace, String source, String target) {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(Opcode.GET_GLOBAL, 0, 0, 0),
                new Instruction(Opcode.DEF_GLOBAL, 1, -1, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    return new HbcProgram(
        namespace,
        List.of(source, target),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static byte[] digest(int value) {
    byte[] digest = new byte[32];
    digest[0] = (byte) value;
    return digest;
  }
}
