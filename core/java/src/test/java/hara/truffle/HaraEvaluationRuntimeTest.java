package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import hara.truffle.HalcSchema.Primitive;
import java.util.Map;
import org.junit.Test;

/** Verifies the evaluation/schema owner has an explicit teardown boundary. */
public class HaraEvaluationRuntimeTest {
  @Test
  public void schemaStateIsInstalledResolvedAndClearedOnClose() {
    HaraEvaluationRuntime runtime = new HaraEvaluationRuntime(source -> null);
    Primitive schema = new Primitive("int");
    runtime.installHalcSchemas(
        new HalcArtifact.SchemaIndex(
            Map.of("demo/Value", "raw"),
            Map.of("demo/f", "function"),
            Map.of("demo/Value", schema),
            Map.of("demo/f", new HalcSchema.Reference("demo/Value")),
            Map.of("demo/f", schema)));

    assertEquals("raw", runtime.halcSchema("demo/Value"));
    assertEquals("function", runtime.halcFunctionSchema("demo/f"));
    assertEquals(schema, runtime.halcSchemaType("demo/Value"));
    assertEquals(schema, runtime.halcFunctionType("demo/f"));
    assertEquals(schema, runtime.halcBestFunctionType("demo/f"));

    runtime.close();
    assertNull(runtime.halcSchema("demo/Value"));
    assertNull(runtime.halcFunctionSchema("demo/f"));
    assertNull(runtime.halcSchemaType("demo/Value"));
    assertNull(runtime.halcFunctionType("demo/f"));
    assertNull(runtime.halcBestFunctionType("demo/f"));
  }
}
