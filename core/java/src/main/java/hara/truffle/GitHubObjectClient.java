package hara.truffle;

import java.util.List;
import java.util.Objects;
import java.util.concurrent.CompletionStage;

/**
 * Authenticated Git-data client used by {@link GitHubFilesystem}.
 *
 * <p>The filesystem owns path, revision, no-follow, and mutation semantics. HTTP, GitHub App token
 * refresh, request identifiers, and REST error decoding stay behind this trusted client boundary.
 */
interface GitHubObjectClient {
  enum FailureKind {
    NOT_FOUND,
    AUTHENTICATION,
    PERMISSION,
    RATE_LIMITED,
    OFFLINE,
    CONFLICT,
    UNSUPPORTED,
    IO
  }

  final class Failure extends RuntimeException {
    private static final long serialVersionUID = 1L;
    private final FailureKind kind;
    private final String providerCode;
    private final boolean retryable;

    Failure(
        FailureKind kind,
        String message,
        String providerCode,
        boolean retryable,
        Throwable cause) {
      super(message, cause);
      this.kind = Objects.requireNonNull(kind, "GitHub failure kind");
      this.providerCode = providerCode;
      this.retryable = retryable;
    }

    Failure(FailureKind kind, String message, String providerCode, boolean retryable) {
      this(kind, message, providerCode, retryable, null);
    }

    FailureKind kind() {
      return kind;
    }

    String providerCode() {
      return providerCode;
    }

    boolean retryable() {
      return retryable;
    }
  }

  record Revision(String commitSha, String treeSha) {
    public Revision {
      commitSha = requireSha(commitSha, "commit SHA");
      treeSha = requireSha(treeSha, "tree SHA");
    }
  }

  record TreeEntry(String path, String mode, String type, String sha, Long size) {
    public TreeEntry {
      path = requireTreePath(path, "Git tree path");
      mode = requireText(mode, "Git tree mode");
      type = requireText(type, "Git tree type");
      sha = requireSha(sha, "Git object SHA");
      if (size != null && size < 0) throw new IllegalArgumentException("negative Git blob size");
    }
  }

  record TreeSnapshot(String treeSha, List<TreeEntry> entries, boolean truncated) {
    public TreeSnapshot {
      treeSha = requireSha(treeSha, "tree SHA");
      entries = List.copyOf(Objects.requireNonNull(entries, "Git tree entries"));
    }
  }

  /** A null SHA deletes the named path from the base tree. */
  record TreeChange(String path, String mode, String type, String sha) {
    public TreeChange {
      path = requireTreePath(path, "Git tree change path");
      if (sha != null) {
        mode = requireText(mode, "Git tree mode");
        type = requireText(type, "Git tree type");
        sha = requireSha(sha, "Git object SHA");
      }
    }

    static TreeChange delete(String path) {
      return new TreeChange(path, null, null, null);
    }
  }

  CompletionStage<Revision> resolveRevision(String repository, String reference);

  CompletionStage<TreeSnapshot> readTree(String repository, String treeSha);

  CompletionStage<byte[]> readBlob(String repository, String blobSha);

  CompletionStage<String> createBlob(String repository, byte[] bytes);

  CompletionStage<String> createTree(
      String repository, String baseTreeSha, List<TreeChange> changes);

  CompletionStage<String> createCommit(
      String repository, String message, String treeSha, String parentCommitSha);

  /**
   * Advances a writable branch only when it still names {@code expectedCommitSha}. Implementations
   * must never force this update and must report a moved reference as {@link FailureKind#CONFLICT}.
   */
  CompletionStage<Void> updateReference(
      String repository,
      String reference,
      String expectedCommitSha,
      String newCommitSha);

  private static String requireTreePath(String value, String label) {
    String path = requireText(value, label);
    if (path.startsWith("/") || path.endsWith("/") || path.indexOf('\0') >= 0
        || path.indexOf('\\') >= 0) {
      throw new IllegalArgumentException(label + " is malformed");
    }
    for (String segment : path.split("/", -1)) {
      if (segment.isEmpty() || ".".equals(segment) || "..".equals(segment)) {
        throw new IllegalArgumentException(label + " is malformed");
      }
    }
    return path;
  }

  private static String requireText(String value, String label) {
    if (value == null || value.isBlank()) throw new IllegalArgumentException(label + " is required");
    return value;
  }

  private static String requireSha(String value, String label) {
    String sha = requireText(value, label);
    if (!sha.matches("[0-9a-fA-F]{7,64}")) {
      throw new IllegalArgumentException(label + " is malformed");
    }
    return sha.toLowerCase(java.util.Locale.ROOT);
  }
}
