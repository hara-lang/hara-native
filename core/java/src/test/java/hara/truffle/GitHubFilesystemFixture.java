package hara.truffle;

import static org.junit.Assert.assertEquals;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;

/** Deterministic Git object store used by focused filesystem conformance tests. */
final class GitHubFilesystemFixture implements AutoCloseable {
  final FakeClient client = new FakeClient();
  final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();

  IFilesystem open(String mode, String reference) {
    GitHubFilesystem.Factory factory = new GitHubFilesystem.Factory();
    IFilesystemFactory.OpenContext context =
        new IFilesystemFactory.OpenContext(
            Runnable::run,
            scheduler,
            credential -> {
              assertEquals("github:test", credential);
              return client;
            });
    return join(
        factory.open(
            context,
            Map.of(
                "credential-ref", "github:test",
                "repository", "hara-lang/hara",
                "ref", reference,
                "root", "/",
                "mode", mode,
                "display", "hara-lang/hara@test")));
  }

  @Override
  public void close() {
    scheduler.shutdownNow();
  }

  static <T> T join(CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      if (error.getCause() instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }

  static final class FakeClient implements GitHubObjectClient {
    private final Map<String, byte[]> blobs = new HashMap<>();
    private final Map<String, TreeSnapshot> trees = new HashMap<>();
    private final Map<String, Revision> commits = new HashMap<>();
    private final Map<String, String> references = new HashMap<>();
    private final List<String> commitMessages = new ArrayList<>();
    private long sequence = 1;
    private final String readmeBlob;
    private final String initialCommit;
    private boolean moveBeforeNextUpdate;
    private String competingHead;

    FakeClient() {
      readmeBlob = blob("hello".getBytes(StandardCharsets.UTF_8));
      String sourceBlob = blob("(+ 1 2)".getBytes(StandardCharsets.UTF_8));
      String linkBlob = blob("README.md".getBytes(StandardCharsets.UTF_8));
      String sourceTree = sha();
      trees.put(
          sourceTree,
          new TreeSnapshot(
              sourceTree,
              List.of(new TreeEntry("main.hal", "100644", "blob", sourceBlob, 7L)),
              false));
      String rootTree = sha();
      trees.put(
          rootTree,
          new TreeSnapshot(
              rootTree,
              List.of(
                  new TreeEntry("README.md", "100644", "blob", readmeBlob, 5L),
                  new TreeEntry("link", "120000", "blob", linkBlob, 9L),
                  new TreeEntry("src", "040000", "tree", sourceTree, null),
                  new TreeEntry("src/main.hal", "100644", "blob", sourceBlob, 7L),
                  new TreeEntry("vendor", "160000", "commit", sha(), null)),
              false));
      initialCommit = sha();
      commits.put(initialCommit, new Revision(initialCommit, rootTree));
      references.put("heads/main", initialCommit);
    }

    String initialCommit() {
      return initialCommit;
    }

    String readmeBlob() {
      return readmeBlob;
    }

    String head() {
      return references.get("heads/main");
    }

    List<String> commitMessages() {
      return List.copyOf(commitMessages);
    }

    void moveBeforeNextUpdate() {
      moveBeforeNextUpdate = true;
      Revision current = commits.get(head());
      competingHead = sha();
      commits.put(competingHead, new Revision(competingHead, current.treeSha()));
    }

    String competingHead() {
      return competingHead;
    }

    @Override
    public CompletionStage<Revision> resolveRevision(String repository, String reference) {
      String commit = commits.containsKey(reference) ? reference : references.get(reference);
      return commit == null
          ? missing("ref-not-found")
          : CompletableFuture.completedFuture(commits.get(commit));
    }

    @Override
    public CompletionStage<TreeSnapshot> readTree(String repository, String treeSha) {
      TreeSnapshot snapshot = trees.get(treeSha);
      return snapshot == null
          ? missing("tree-not-found")
          : CompletableFuture.completedFuture(snapshot);
    }

    @Override
    public CompletionStage<byte[]> readBlob(String repository, String blobSha) {
      byte[] bytes = blobs.get(blobSha);
      return bytes == null
          ? missing("blob-not-found")
          : CompletableFuture.completedFuture(bytes.clone());
    }

    @Override
    public CompletionStage<String> createBlob(String repository, byte[] bytes) {
      return CompletableFuture.completedFuture(blob(bytes));
    }

    @Override
    public CompletionStage<String> createTree(
        String repository, String baseTreeSha, List<TreeChange> changes) {
      TreeSnapshot base = trees.get(baseTreeSha);
      if (base == null) return missing("base-tree-not-found");
      LinkedHashMap<String, TreeEntry> flat = new LinkedHashMap<>();
      for (TreeEntry entry : base.entries()) {
        if (!"tree".equals(entry.type())) flat.put(entry.path(), entry);
      }
      for (TreeChange change : changes) applyChange(flat, change);
      return CompletableFuture.completedFuture(rebuildTrees(flat));
    }

    @Override
    public CompletionStage<String> createCommit(
        String repository, String message, String treeSha, String parentCommitSha) {
      if (!trees.containsKey(treeSha) || !commits.containsKey(parentCommitSha)) {
        return missing("commit-input-not-found");
      }
      String commit = sha();
      commits.put(commit, new Revision(commit, treeSha));
      commitMessages.add(message);
      return CompletableFuture.completedFuture(commit);
    }

    @Override
    public CompletionStage<Void> updateReference(
        String repository,
        String reference,
        String expectedCommitSha,
        String newCommitSha) {
      if (moveBeforeNextUpdate) {
        moveBeforeNextUpdate = false;
        references.put(reference, competingHead);
      }
      if (!expectedCommitSha.equals(references.get(reference))) {
        return CompletableFuture.failedFuture(
            new Failure(
                FailureKind.CONFLICT,
                "reference moved",
                "reference-update-conflict",
                true));
      }
      references.put(reference, newCommitSha);
      return CompletableFuture.completedFuture(null);
    }

    private void applyChange(Map<String, TreeEntry> flat, TreeChange change) {
      flat.keySet().removeIf(
          path -> path.equals(change.path()) || path.startsWith(change.path() + "/"));
      if (change.sha() == null) return;
      if ("tree".equals(change.type())) {
        TreeSnapshot subtree = trees.get(change.sha());
        if (subtree == null) throw missingFailure("subtree-not-found");
        for (TreeEntry child : subtree.entries()) {
          if ("tree".equals(child.type())) continue;
          String path = change.path() + "/" + child.path();
          flat.put(path, new TreeEntry(path, child.mode(), child.type(), child.sha(), child.size()));
        }
        return;
      }
      Long size = null;
      if ("blob".equals(change.type())) {
        byte[] bytes = blobs.get(change.sha());
        if (bytes == null) throw missingFailure("blob-not-found");
        size = (long) bytes.length;
      }
      flat.put(
          change.path(),
          new TreeEntry(change.path(), change.mode(), change.type(), change.sha(), size));
    }

    private String rebuildTrees(Map<String, TreeEntry> files) {
      Set<String> directories = new java.util.TreeSet<>();
      for (String path : files.keySet()) {
        int separator = path.lastIndexOf('/');
        while (separator > 0) {
          directories.add(path.substring(0, separator));
          separator = path.lastIndexOf('/', separator - 1);
        }
      }
      Map<String, String> directoryShas = new HashMap<>();
      ArrayList<String> deepest = new ArrayList<>(directories);
      deepest.sort(
          Comparator.comparingInt((String value) -> value.split("/").length).reversed());
      for (String directory : deepest) {
        String treeSha = sha();
        directoryShas.put(directory, treeSha);
        String prefix = directory + "/";
        ArrayList<TreeEntry> subtree = new ArrayList<>();
        for (TreeEntry entry : files.values()) {
          if (!entry.path().startsWith(prefix)) continue;
          String relative = entry.path().substring(prefix.length());
          subtree.add(
              new TreeEntry(relative, entry.mode(), entry.type(), entry.sha(), entry.size()));
        }
        trees.put(treeSha, new TreeSnapshot(treeSha, subtree, false));
      }
      ArrayList<TreeEntry> rootEntries = new ArrayList<>(files.values());
      for (Map.Entry<String, String> directory : directoryShas.entrySet()) {
        rootEntries.add(
            new TreeEntry(directory.getKey(), "040000", "tree", directory.getValue(), null));
      }
      rootEntries.sort(Comparator.comparing(TreeEntry::path));
      String rootSha = sha();
      trees.put(rootSha, new TreeSnapshot(rootSha, rootEntries, false));
      return rootSha;
    }

    private String blob(byte[] bytes) {
      String sha = sha();
      blobs.put(sha, bytes.clone());
      return sha;
    }

    private String sha() {
      return String.format("%040x", sequence++);
    }

    private static Failure missingFailure(String providerCode) {
      return new Failure(
          FailureKind.NOT_FOUND,
          "GitHub object not found",
          providerCode,
          false);
    }

    private static <T> CompletionStage<T> missing(String providerCode) {
      return CompletableFuture.failedFuture(missingFailure(providerCode));
    }
  }
}
