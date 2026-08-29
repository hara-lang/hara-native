package hara.truffle.bytecode;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

/** Deterministic graph helpers for exact admitted schema catalogs. */
final class HbcSchemaCatalogGraph {
  private HbcSchemaCatalogGraph() {}

  static List<List<HbcSchemaLinks.SchemaCoordinate>> stronglyConnectedComponents(
      Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> graph,
      Comparator<HbcSchemaLinks.SchemaCoordinate> coordinateOrder) {
    Set<HbcSchemaLinks.SchemaCoordinate> seen = new HashSet<>();
    List<HbcSchemaLinks.SchemaCoordinate> order = new ArrayList<>();
    for (HbcSchemaLinks.SchemaCoordinate coordinate : graph.keySet()) {
      visitOrder(coordinate, graph, seen, order);
    }

    Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> reverse =
        new TreeMap<>(coordinateOrder);
    for (HbcSchemaLinks.SchemaCoordinate coordinate : graph.keySet()) {
      reverse.put(coordinate, new TreeSet<>(coordinateOrder));
    }
    for (Map.Entry<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> entry :
        graph.entrySet()) {
      for (HbcSchemaLinks.SchemaCoordinate dependency : entry.getValue()) {
        reverse.get(dependency).add(entry.getKey());
      }
    }

    seen.clear();
    List<List<HbcSchemaLinks.SchemaCoordinate>> output = new ArrayList<>();
    for (int index = order.size() - 1; index >= 0; index--) {
      HbcSchemaLinks.SchemaCoordinate coordinate = order.get(index);
      if (seen.contains(coordinate)) continue;
      List<HbcSchemaLinks.SchemaCoordinate> members = new ArrayList<>();
      visitComponent(coordinate, reverse, seen, members);
      members.sort(coordinateOrder);
      output.add(List.copyOf(members));
    }
    output.sort(
        (left, right) -> compareCoordinateLists(left, right, coordinateOrder));
    return output;
  }

  static List<String> dependencyFirstOrder(Map<String, Set<String>> rawGraph) {
    Map<String, Set<String>> graph = new TreeMap<>();
    for (Map.Entry<String, Set<String>> entry : rawGraph.entrySet()) {
      graph.put(entry.getKey(), new TreeSet<>(entry.getValue()));
    }
    List<String> output = new ArrayList<>();
    while (!graph.isEmpty()) {
      List<String> ready =
          graph.entrySet().stream()
              .filter(entry -> entry.getValue().isEmpty())
              .map(Map.Entry::getKey)
              .toList();
      if (ready.isEmpty()) {
        throw new HbcFormatException(
            "schema catalog component graph contains a cycle");
      }
      Set<String> readySet = Set.copyOf(ready);
      ready.forEach(graph::remove);
      graph.values().forEach(
          dependencies -> dependencies.removeAll(readySet));
      output.addAll(ready);
    }
    return List.copyOf(output);
  }

  static int compareCoordinateLists(
      List<HbcSchemaLinks.SchemaCoordinate> left,
      List<HbcSchemaLinks.SchemaCoordinate> right,
      Comparator<HbcSchemaLinks.SchemaCoordinate> coordinateOrder) {
    int common = Math.min(left.size(), right.size());
    for (int index = 0; index < common; index++) {
      int compared = coordinateOrder.compare(left.get(index), right.get(index));
      if (compared != 0) return compared;
    }
    return Integer.compare(left.size(), right.size());
  }

  private static void visitOrder(
      HbcSchemaLinks.SchemaCoordinate coordinate,
      Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> graph,
      Set<HbcSchemaLinks.SchemaCoordinate> seen,
      List<HbcSchemaLinks.SchemaCoordinate> order) {
    if (!seen.add(coordinate)) return;
    for (HbcSchemaLinks.SchemaCoordinate dependency : graph.get(coordinate)) {
      visitOrder(dependency, graph, seen, order);
    }
    order.add(coordinate);
  }

  private static void visitComponent(
      HbcSchemaLinks.SchemaCoordinate coordinate,
      Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> reverse,
      Set<HbcSchemaLinks.SchemaCoordinate> seen,
      List<HbcSchemaLinks.SchemaCoordinate> members) {
    if (!seen.add(coordinate)) return;
    members.add(coordinate);
    for (HbcSchemaLinks.SchemaCoordinate dependent : reverse.get(coordinate)) {
      visitComponent(dependent, reverse, seen, members);
    }
  }
}
