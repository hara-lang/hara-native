package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Symbol;
import hara.truffle.bytecode.HbxBundleCodec;
import hara.work.WorkPlan;
import hara.work.WorkRegistry;
import hara.work.WorkRuntime;
import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;
import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Behavioral consumption of the deterministic fixture artifacts emitted by the Rust host. */
public class RustProducedArtifactFixtureTest {
  private static final Path FIXTURES = Path.of("rust/assets/host-fixtures");

  @Test
  public void executesTheRustProducedHbxModule() throws Exception {
    byte[] bundle = fixture("portable-module.hbx");
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        context.installBytecodeBundle(bundle);
        assertEquals(42L, context.resolve(Symbol.create("fixture.hbx/answer")).deref());
      } finally {
        polyglot.leave();
      }
    }
  }

  @Test
  public void executesTheRustProducedHalcModule() throws Exception {
    HalcArtifact.Module module = HalcArtifact.decode(fixture("portable-module.halc"));
    assertEquals("fixture.halc", module.namespace);
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraLanguage.compileHalc(module, "rust-produced-fixture.halc").call();
        assertEquals(
            42L, HaraLanguage.currentContext().resolve(Symbol.create("fixture.halc/answer")).deref());
      } finally {
        polyglot.leave();
      }
    }
  }

  @Test
  public void executesTheRustProducedHtaWorkPlan() throws Exception {
    WorkPlan plan = WorkPlan.decodeHta(fixture("portable-work-plan.hta"));
    WorkRegistry registry = new WorkRegistry();
    registry.bind(
        "fixture/answer",
        (input, context) -> CompletableFuture.completedFuture(((Long) input) + 1L));

    assertEquals(WorkPlan.Operation.PURE, plan.operation());
    assertEquals(42L, new WorkRuntime(registry).evaluate(plan, 41L).toCompletableFuture().join());
  }

  @Test
  public void verifiesTheRustProducedHarpPackage() throws Exception {
    ByteArrayOutputStream output = new ByteArrayOutputStream();
    ByteArrayOutputStream error = new ByteArrayOutputStream();
    PrintStream stdout = new PrintStream(output, true, StandardCharsets.UTF_8);
    PrintStream stderr = new PrintStream(error, true, StandardCharsets.UTF_8);

    assertEquals(
        0,
        HaraPackageTool.run(
            new String[] {"verify", FIXTURES.resolve("portable-package.harp").toString()},
            stdout,
            stderr));
    assertTrue(output.toString(StandardCharsets.UTF_8).contains("fixture/rust-host-fixtures 1.0.0"));
    assertEquals("", error.toString(StandardCharsets.UTF_8));
  }

  private static byte[] fixture(String name) throws Exception {
    Path path = FIXTURES.resolve(name);
    assertTrue("missing Rust-produced fixture: " + path, Files.isRegularFile(path));
    return Files.readAllBytes(path);
  }
}
