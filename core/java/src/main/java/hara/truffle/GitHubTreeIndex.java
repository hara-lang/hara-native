package hara.truffle;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Immutable no-follow projection of one complete Git tree beneath a configured mount root. */
final class GitHubTreeIndex {
  record Node(
      String path,
      String repositoryPath,
      String name,
      IFilesystem.EntryType type,
      String mode,
      String sha,
      Long size) {
    boolean directory() {
      return type == IFilesystem.EntryType.DIRECTORY;
    }
  }

  private final String repositoryRoot;
  private final Map<String, Node> nodes;

  GitHubTreeIndex(GitHubObjectClient.TreeSnapshot snapshot, String root) {
    if (snapshot.truncated()) {
      throw failure(
          "unsupported",
          "GitHub returned a truncated tree",
          "tree-truncated",
          true,
          null);
    }
    String logicalRoot = HaraLogicalPath.normalise(root == null ? "/" : root);
    repositoryRoot = "/".equals(logicalRoot) ? "" : logicalRoot.substring(1);

    HashMap<String, GitHubObjectClient.TreeEntry> repositoryEntries = new HashMap<>();
    for (GitHubObjectClient.TreeEntry entry : snapshot.entries()) {
      GitHubObjectClient.TreeEntry previous = repositoryEntries.put(entry.path(), entry);
      if (previous != null) {
        throw failure("io", "GitHub tree contains duplicate paths", "duplicate-tree-path", false, null);
      }
    }

    GitHubObjectClient.TreeEntry rootEntry =
        repositoryRoot.isEmpty() ? null : repositoryEntries.get(repositoryRoot);
    boolean hasDescendant =
        repositoryRoot.isEmpty()
            || repositoryEntries.keySet().stream().anyMatch(path -> beneath(path, repositoryRoot));
    if (!repositoryRoot.isEmpty() && rootEntry == null && !hasDescendant) {
      throw failure("not-found", "GitHub mount root does not exist", "root-not-found", false, null);
    }
    if (rootEntry != null && !"tree".equals(rootEntry.type())) {
      throw failure("not-directory", "GitHub mount root is not a tree", "root-not-tree", false, null);
    }

    LinkedHashMap<String, Node> projected = new LinkedHashMap<>();
    projected.put(
        "/",
        new Node(
            "/",
            repositoryRoot,
            "",
            IFilesystem.EntryType.DIRECTORY,
            "040000",
            rootEntry == null ? snapshot.treeSha() : rootEntry.sha(),
            null));

    ArrayList<GitHubObjectClient.TreeEntry> sorted = new ArrayList<>(snapshot.entries());
    sorted.sort(Comparator.comparing(GitHubObjectClient.TreeEntry::path));
    for (GitHubObjectClient.TreeEntry entry : sorted) {
      String relative = relativePath(entry.path());
      if (relative == null || relative.isEmpty()) continue;
      addParents(projected, repositoryEntries, relative);
      String logical = HaraLogicalPath.normalise("/" + relative);
      Node node = node(logical, entry);
      Node previous = projected.put(logical, node);
      if (previous != null && !sameNode(previous, node)) {
        throw failure("io", "GitHub tree projection collides", "tree-path-collision", false, null);
      }
    }
    nodes = Map.copyOf(projected);
  }

  String repositoryRoot() {
    return repositoryRoot;
  }

  Node find(String path) {
    return nodes.get(HaraLogicalPath.normalise(path));
  }

  List<Node> children(String path) {
    String directory = HaraLogicalPath.normalise(path);
    Node parent = nodes.get(directory);
    if (parent == null) return List.of();
    if (!parent.directory()) {
      throw failure("not-directory", "path is not a directory", "not-tree", false, null);
    }
    ArrayList<Node> output = new ArrayList<>();
    for (Node node : nodes.values()) {
      if (directory.equals(HaraLogicalPath.parent(node.path()))) output.add(node);
    }
    output.sort(Comparator.comparing(Node::path));
    return List.copyOf(output);
  }

  List<Node> descendants(String path) {
    String root = HaraLogicalPath.normalise(path);
    String prefix = "/".equals(root) ? "/" : root + "/";
    ArrayList<Node> output = new ArrayList<>();
    for (Node node : nodes.values()) {
      if (!node.path().equals(root) && node.path().startsWith(prefix)) output.add(node);
    }
    output.sort(Comparator.comparing(Node::path));
    return List.copyOf(output);
  }

  String repositoryPath(String path) {
    String logical = HaraLogicalPath.normalise(path);
    String relative = "/".equals(logical) ? "" : logical.substring(1);
    if (repositoryRoot.isEmpty()) return relative;
    return relative.isEmpty() ? repositoryRoot : repositoryRoot + "/" + relative;
  }

  boolean parentExists(String path) {
    String parent = HaraLogicalPath.parent(HaraLogicalPath.normalise(path));
    Node node = parent == null ? null : nodes.get(parent);
    return node != null && node.directory();
  }

  private String relativePath(String repositoryPath) {
    if (repositoryRoot.isEmpty()) return repositoryPath;
    if (repositoryPath.equals(repositoryRoot)) return "";
    return beneath(repositoryPath, repositoryRoot)
        ? repositoryPath.substring(repositoryRoot.length() + 1)
        : null;
  }

  private void addParents(
      Map<String, Node> projected,
      Map<String, GitHubObjectClient.TreeEntry> repositoryEntries,
      String relative) {
    String[] segments = relative.split("/");
    StringBuilder logical = new StringBuilder();
    StringBuilder repository = new StringBuilder(repositoryRoot);
    for (int index = 0; index < segments.length - 1; index++) {
      logical.append('/').append(segments[index]);
      if (!repository.isEmpty()) repository.append('/');
      repository.append(segments[index]);
      String logicalPath = logical.toString();
      Node current = projected.get(logicalPath);
      if (current != null) {
        if (!current.directory()) {
          throw failure(
              "not-directory",
              "GitHub tree path has a non-directory ancestor",
              "non-tree-ancestor",
              false,
              null);
        }
        continue;
      }
      GitHubObjectClient.TreeEntry explicit = repositoryEntries.get(repository.toString());
      if (explicit != null && !"tree".equals(explicit.type())) {
        throw failure(
            "not-directory",
            "GitHub tree path has a non-directory ancestor",
            "non-tree-ancestor",
            false,
            null);
      }
      projected.put(
          logicalPath,
          new Node(
              logicalPath,
              repository.toString(),
              segments[index],
              IFilesystem.EntryType.DIRECTORY,
              "040000",
              explicit == null ? null : explicit.sha(),
              null));
    }
  }

  private static Node node(String logical, GitHubObjectClient.TreeEntry entry) {
    IFilesystem.EntryType type;
    if ("120000".equals(entry.mode())) {
      type = IFilesystem.EntryType.SYMLINK;
    } else if ("160000".equals(entry.mode()) || "commit".equals(entry.type())) {
      type = IFilesystem.EntryType.OTHER;
    } else if ("tree".equals(entry.type())) {
      type = IFilesystem.EntryType.DIRECTORY;
    } else if ("blob".equals(entry.type())) {
      type = IFilesystem.EntryType.FILE;
    } else {
      type = IFilesystem.EntryType.OTHER;
    }
    return new Node(
        logical,
        entry.path(),
        HaraLogicalPath.fileName(logical),
        type,
        entry.mode(),
        entry.sha(),
        type == IFilesystem.EntryType.FILE ? entry.size() : null);
  }

  private static boolean sameNode(Node left, Node right) {
    return left.type() == right.type()
        && java.util.Objects.equals(left.mode(), right.mode())
        && java.util.Objects.equals(left.sha(), right.sha());
  }

  private static boolean beneath(String path, String root) {
    return path.length() > root.length()
        && path.startsWith(root)
        && path.charAt(root.length()) == '/';
  }

  private static FilesystemException failure(
      String code, String message, String providerCode, boolean retryable, Throwable cause) {
    return new FilesystemException(
        code,
        message,
        "github",
        null,
        null,
        null,
        providerCode,
        retryable,
        cause);
  }
}
