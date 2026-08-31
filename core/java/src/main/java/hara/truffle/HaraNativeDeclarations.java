package hara.truffle;

import hara.lang.declaration.HaraNativeBinding;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Runtime view of the annotated native type surface. */
final class HaraNativeDeclarations {
  private static final String NATIVE_NAMESPACE = "std.native";
  private static final Map<String, HaraNativeBinding> BINDINGS = bindingsByQualifiedName();
  static final Map<String, List<String>> METHODS = methodInventory();

  private HaraNativeDeclarations() {}

  static List<HaraNativeBinding> bindings() {
    return List.copyOf(BINDINGS.values());
  }

  /** Compatibility lookup for the closed std.native surface. */
  static HaraNativeBinding binding(String name) {
    return binding(NATIVE_NAMESPACE, name);
  }

  static HaraNativeBinding binding(String namespace, String name) {
    HaraNativeBinding binding = BINDINGS.get(qualifiedName(namespace, name));
    if (binding == null) {
      throw new HaraException("Missing annotated native type: " + qualifiedName(namespace, name));
    }
    return binding;
  }

  /** Compatibility lookup for the closed std.native surface. */
  static String namespace(String name) {
    return qualifiedName(binding(name));
  }

  static String qualifiedName(HaraNativeBinding binding) {
    return qualifiedName(binding.namespace(), binding.name());
  }

  static String qualifiedName(String namespace, String name) {
    return namespace + "." + name;
  }

  /** Compatibility lookup for the closed std.native surface. */
  static List<String> methods(String name) {
    return List.of(binding(name).methods());
  }

  static List<String> methods(HaraNativeBinding binding) {
    return List.of(binding.methods());
  }

  private static Map<String, List<String>> methodInventory() {
    Map<String, List<String>> methods = new LinkedHashMap<>();
    for (HaraNativeBinding binding : BINDINGS.values()) {
      if (NATIVE_NAMESPACE.equals(binding.namespace())) {
        methods.put(binding.name(), List.of(binding.methods()));
      }
    }
    return Map.copyOf(methods);
  }

  private static Map<String, HaraNativeBinding> bindingsByQualifiedName() {
    Map<String, HaraNativeBinding> bindings = new LinkedHashMap<>();
    for (HaraNativeBinding binding :
        HaraBuiltinCatalog.class.getAnnotationsByType(HaraNativeBinding.class)) {
      if (binding.namespace().isBlank() || binding.name().isBlank()) {
        throw new HaraException("Native binding namespace and name are required");
      }
      String qualifiedName = qualifiedName(binding);
      if (bindings.put(qualifiedName, binding) != null) {
        throw new HaraException("Duplicate annotated native type: " + qualifiedName);
      }
      if (binding.methods().length == 0) {
        throw new HaraException("Native binding has no methods: " + binding.name());
      }
      if (binding.availability() == hara.lang.declaration.HaraAvailability.CAPABILITY_GATED
          && binding.capability().isBlank()) {
        throw new HaraException("Capability-gated native binding has no capability: " + binding.name());
      }
      if (binding.availability() != hara.lang.declaration.HaraAvailability.CAPABILITY_GATED
          && !binding.capability().isBlank()) {
        throw new HaraException("Portable native binding declares a capability: " + binding.name());
      }
      Set<String> methods = new java.util.HashSet<>();
      for (String method : binding.methods()) {
        if (!methods.add(method)) {
          throw new HaraException("Duplicate annotated native method: " + binding.name() + "/" + method);
        }
      }
    }
    return Map.copyOf(bindings);
  }
}
