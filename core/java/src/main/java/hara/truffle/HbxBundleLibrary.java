package hara.truffle;

import hara.lang.data.Symbol;
import hara.truffle.bytecode.HbcCodec;
import hara.truffle.bytecode.HbcFormatException;
import hara.truffle.bytecode.HbxBundleCodec;
import hara.truffle.bytecode.HbcProgram;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
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

  record Installation(
      Map<String, Module> namespaces,
      Map<String, Module> resources,
      List<Module> eagerModules) {
    Installation {
      namespaces = immutableIndex(namespaces);
      resources = immutableIndex(resources);
      eagerModules = List.copyOf(eagerModules);
    }
  }

  record Snapshot(
      Map<String, Module> namespaces,
      Map<String, Module> resources,
      List<Module> eagerModules) {
    Snapshot {
      namespaces = immutableIndex(namespaces);
      resources = immutableIndex(resources);
      eagerModules = List.copyOf(eagerModules);
    }
  }

  private volatile Snapshot state;

  HbxBundleLibrary(ClassLoader loader) {
    this.state = new Snapshot(Map.of(), Map.of(), List.of());
  }

  /**
   * Validates an externally supplied bundle without mutating the host index.
   *
   * <p>The caller owns authorization and the installation transaction. Keeping
   * preparation separate prevents an invalid or conflicting package from
   * changing namespace visibility.
   */
  Installation prepare(byte[] bundle) {
    List<HbxBundleCodec.Module> descriptors = HbxBundleCodec.decode(bundle);
    LinkedHashMap<String, Module> byNamespace = new LinkedHashMap<>();
    LinkedHashMap<String, Module> byResource = new LinkedHashMap<>();
    ArrayList<Module> eager = new ArrayList<>();
    for (HbxBundleCodec.Module descriptor : descriptors) {
      String namespace = namespace(descriptor);
      HbcProgram program = HbcCodec.decode(descriptor.artifact());
      if (program.namespace() != null && !namespace.equals(program.namespace())) {
        throw new HbcFormatException(
            descriptor.resource()
                + ": HBC0 namespace "
                + program.namespace()
                + " does not match HBX0 namespace "
                + namespace);
      }
      Module module = new Module(namespace, descriptor, program);
      if (byNamespace.put(namespace, module) != null) {
        throw new HbcFormatException("duplicate HBX0 namespace: " + namespace);
      }
      if (byResource.put(descriptor.resource(), module) != null) {
        throw new HbcFormatException("duplicate HBX0 module: " + descriptor.resource());
      }
      if (descriptor.eager()) eager.add(module);
    }
    return new Installation(byNamespace, byResource, eager);
  }

  synchronized Snapshot snapshot() {
    return state;
  }

  synchronized void restore(Snapshot snapshot) {
    state = snapshot;
  }

  synchronized void install(Installation installation) {
    Snapshot current = state;
    LinkedHashMap<String, Module> updatedNamespaces = new LinkedHashMap<>(current.namespaces());
    for (Map.Entry<String, Module> entry : installation.namespaces().entrySet()) {
      if (updatedNamespaces.putIfAbsent(entry.getKey(), entry.getValue()) != null) {
        throw new HbcFormatException("HBX0 namespace is already installed: " + entry.getKey());
      }
    }
    LinkedHashMap<String, Module> updatedResources = new LinkedHashMap<>(current.resources());
    for (Map.Entry<String, Module> entry : installation.resources().entrySet()) {
      if (updatedResources.putIfAbsent(entry.getKey(), entry.getValue()) != null) {
        throw new HbcFormatException("HBX0 module is already installed: " + entry.getKey());
      }
    }
    ArrayList<Module> updatedEager = new ArrayList<>(current.eagerModules());
    updatedEager.addAll(installation.eagerModules());
    state = new Snapshot(updatedNamespaces, updatedResources, updatedEager);
  }

  boolean available() {
    return !state.namespaces().isEmpty();
  }

  boolean provides(String namespace) {
    return state.namespaces().containsKey(namespace);
  }

  Module module(String namespace) {
    return state.namespaces().get(namespace);
  }

  Iterable<String> namespaces() {
    return state.namespaces().keySet();
  }

  List<Module> eagerModules() {
    return state.eagerModules();
  }

  List<String> dependencyNamespaces(Module module) {
    Snapshot current = state;
    ArrayList<String> dependencies = new ArrayList<>();
    for (String resource : module.descriptor().dependencies()) {
      Module dependency = current.resources().get(resource);
      dependencies.add(dependency == null ? resource : dependency.namespace());
    }
    return List.copyOf(dependencies);
  }

  private static String namespace(HbxBundleCodec.Module descriptor) {
    Object[] forms;
    try {
      forms = HaraLanguage.readAll(descriptor.namespaceForm(), descriptor.resource() + "#ns");
    } catch (RuntimeException error) {
      throw new HbcFormatException(
          descriptor.resource() + ": invalid HBX0 namespace form: " + error.getMessage());
    }
    if (forms.length != 1
        || !(forms[0] instanceof hara.lang.data.List<?> form)
        || form.count() < 2
        || !(form.nth(0) instanceof Symbol operator)
        || operator.getNamespace() != null
        || !"ns".equals(operator.getName())
        || !(form.nth(1) instanceof Symbol namespace)
        || namespace.getNamespace() != null) {
      throw new HbcFormatException(descriptor.resource() + ": invalid HBX0 namespace form");
    }
    return namespace.getName();
  }

  private static <K, V> Map<K, V> immutableIndex(Map<K, V> index) {
    return Collections.unmodifiableMap(new LinkedHashMap<>(index));
  }
}
