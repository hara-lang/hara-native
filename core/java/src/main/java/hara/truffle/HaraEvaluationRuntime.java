package hara.truffle;

import java.util.ArrayList;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/** Context-scoped source/form evaluation and HALC schema state. */
final class HaraEvaluationRuntime implements AutoCloseable {
  private final Evaluator evaluator;
  private final Map<String, Object> schemaDefinitions = new ConcurrentHashMap<>();
  private final Map<String, Object> functionSchemas = new ConcurrentHashMap<>();
  private final Map<String, HalcSchema.Type> schemaTypes = new ConcurrentHashMap<>();
  private final Map<String, HalcSchema.Type> functionTypes = new ConcurrentHashMap<>();
  private final Map<String, HalcSchema.Type> inferredFunctionTypes = new ConcurrentHashMap<>();
  private final Map<HaraVar, ArrayList<HaraVar>> pendingSchemaContracts =
      new ConcurrentHashMap<>();

  HaraEvaluationRuntime(Evaluator.SourceExecutor executor) {
    this.evaluator = new Evaluator(executor);
  }

  Object evalSource(String sourceText, String name) {
    return evaluator.evalSource(sourceText, name);
  }

  Object evalForm(Object form, String name) {
    return evaluator.evalForm(form, name);
  }

  void installHalcSchemas(HalcArtifact.SchemaIndex schemas) {
    schemaDefinitions.putAll(schemas.definitions);
    functionSchemas.putAll(schemas.functions);
    schemaTypes.putAll(schemas.definitionTypes);
    functionTypes.putAll(schemas.functionTypes);
    inferredFunctionTypes.putAll(schemas.inferredFunctionTypes);
  }

  Object halcSchema(String qualifiedVar) {
    return schemaDefinitions.get(qualifiedVar);
  }

  Object halcFunctionSchema(String qualifiedVar) {
    return functionSchemas.get(qualifiedVar);
  }

  HalcSchema.Type halcSchemaType(String qualifiedVar) {
    return schemaTypes.get(qualifiedVar);
  }

  HalcSchema.Type halcFunctionType(String qualifiedVar) {
    HalcSchema.Type schema = functionTypes.get(qualifiedVar);
    if (schema instanceof HalcSchema.Reference reference) {
      return schemaTypes.getOrDefault(reference.name(), schema);
    }
    return schema;
  }

  HalcSchema.Type halcInferredFunctionType(String qualifiedVar) {
    return inferredFunctionTypes.get(qualifiedVar);
  }

  HalcSchema.Type halcBestFunctionType(String qualifiedVar) {
    HalcSchema.Type declared = halcFunctionType(qualifiedVar);
    return declared != null ? declared : halcInferredFunctionType(qualifiedVar);
  }

  void installHbcTypes(
      Map<String, HalcSchema.Type> schemaTypes,
      Map<String, HalcSchema.Type> functionTypes,
      Map<String, HalcSchema.Type> inferredFunctionTypes) {
    this.schemaTypes.putAll(schemaTypes);
    this.functionTypes.putAll(functionTypes);
    this.inferredFunctionTypes.putAll(inferredFunctionTypes);
  }

  void deferSchemaContract(HaraVar schemaVariable, HaraVar dependent) {
    pendingSchemaContracts
        .computeIfAbsent(schemaVariable, ignored -> new ArrayList<>())
        .add(dependent);
  }

  ArrayList<HaraVar> takePendingSchemaContracts(HaraVar schemaVariable) {
    return pendingSchemaContracts.remove(schemaVariable);
  }

  @Override
  public void close() {
    schemaDefinitions.clear();
    functionSchemas.clear();
    schemaTypes.clear();
    functionTypes.clear();
    inferredFunctionTypes.clear();
    pendingSchemaContracts.clear();
  }
}
