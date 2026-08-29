package hara.truffle;

import hara.truffle.bytecode.HbxBundleCodec;
import hara.truffle.bytecode.HbcProgram;
import java.util.List;
import java.util.Map;

/**
 * Namespace index for a verified package-provided HBX0 artifact.
 *
 * <p>The native distribution deliberately supplies no default bundle. Canonical
 * library bytecode belongs to a signed HARP package and must be mounted by the
 * consuming application before a Truffle context requests it.
 */
final class HbxBundleLibrary {
  record Module(String namespace, HbxBundleCodec.Module descriptor, HbcProgram program) {}

  private final Map<String, Module> modules;

  HbxBundleLibrary(ClassLoader loader) {
    this.modules = Map.of();
  }

  boolean available() {
    return !modules.isEmpty();
  }

  boolean provides(String namespace) {
    return modules.containsKey(namespace);
  }

  Module module(String namespace) {
    return modules.get(namespace);
  }

  Iterable<String> namespaces() {
    return modules.keySet();
  }

  List<Module> eagerModules() {
    return List.of();
  }
}
