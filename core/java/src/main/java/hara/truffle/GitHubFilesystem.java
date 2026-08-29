package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Supplier;

/** Provider-neutral GitHub repository mount backed directly by Git objects and refs. */
final class GitHubFilesystem implements IFilesystem {
  enum MountMode {
    READ_ONLY,
    COMMIT
  }

  static final class Factory implements IFilesystemFactory {
    private static final Set<String> ALLOWED =
        Set.of(
            "credential-ref",
            "repository",
            "ref",
            "root",
            "mode",
            "display",
            "commit-message-prefix");

    @Override
    public String kind() {
      return "github";
    }

    @Override
    public void validate(Map<String, ?> configuration) {
      IFilesystemFactory.super.validate(configuration);
      for (String key : configuration.keySet()) {
        if (!ALLOWED.contains(key)) {
          throw new IllegalArgumentException("unknown GitHub filesystem option " + key);
        }
      }
      requireText(configuration, "credential-ref");
      String repository = requireText(configuration, "repository");
      if (!repository.matches("[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")) {
        throw new IllegalArgumentException("GitHub repository must be owner/name");
      }
      String reference = requireText(configuration, "ref");
      MountMode mode = mountMode(configuration.get("mode"));
      if (mode == MountMode.COMMIT && !reference.startsWith("heads/")) {
        throw new IllegalArgumentException("writable GitHub mounts require a heads/* ref");
      }
      HaraLogicalPath.normalise(
          configuration.get("root") instanceof String root ? root : "/");
      Object display = configuration.get("display");
      if (display != null && (!(display instanceof String value) || value.isBlank())) {
        throw new IllegalArgumentException("GitHub filesystem display must be a string");
      }
      Object prefix = configuration.get("commit-message-prefix");
      if (prefix != null && (!(prefix instanceof String value) || value.isBlank())) {
        throw new IllegalArgumentException("GitHub commit message prefix must be a string");
      }
    }

    @Override
    public CompletionStage<IFilesystem> open(
        OpenContext context, Map<String, ?> configuration) {
      Objects.requireNonNull(context, "filesystem open context");
      validate(configuration);
      String credentialReference = (String) configuration.get("credential-ref");
      Object resolved = context.credentials().resolve(credentialReference);
      if (!(resolved instanceof GitHubObjectClient client)) {
        return CompletableFuture.failedFuture(
            new IllegalArgumentException(
                "GitHub credential reference did not resolve to an authenticated object client"));
      }
      String repository = (String) configuration.get("repository");
      String reference = (String) configuration.get("ref");
      String root =
          HaraLogicalPath.normalise(
              configuration.get("root") instanceof String value ? value : "/");
      MountMode mode = mountMode(configuration.get("mode"));
      String display =
          configuration.get("display") instanceof String value
              ? value
              : repository + "@" + safeReference(reference);
      String prefix =
          configuration.get("commit-message-prefix") instanceof String value
              ? value
              : "hara filesystem";

      CompletableFuture<IFilesystem> result = new CompletableFuture<>();
      client
          .resolveRevision(repository, reference)
          .thenCompose(
              revision ->
                  client
                      .readTree(repository, revision.treeSha())
                      .thenApply(tree -> new Snapshot(revision, new GitHubTreeIndex(tree, root))))
          .whenComplete(
              (snapshot, error) -> {
                if (error != null) {
                  result.completeExceptionally(
                      mapFailure(error, "open", null, null));
                } else {
                  result.complete(
                      new GitHubFilesystem(
                          client,
                          repository,
                          reference,
                          root,
                          mode,
                          display,
                          prefix,
                          context.scheduler(),
                          snapshot));
                }
              });
      return result;
    }

    private static String requireText(Map<String, ?> values, String key) {
      Object value = values.get(key);
      if (!(value instanceof String text) || text.isBlank()) {
        throw new IllegalArgumentException("GitHub filesystem " + key + " is required");
      }
      return text;
    }

    private static MountMode mountMode(Object value) {
      String mode = value == null ? "read-only" : String.valueOf(value);
      return switch (mode) {
        case "read-only" -> MountMode.READ_ONLY;
        case "commit" -> MountMode.COMMIT;
        default -> throw new IllegalArgumentException(
            "GitHub filesystem mode must be read-only or commit");
      };
    }

    private static String safeReference(String reference) {
      return reference.startsWith("heads/") ? reference.substring("heads/".length()) : reference;
    }
  }

  @FunctionalInterface
  private interface AsyncOperation<T> {
    CompletionStage<T> call();
  }

  private record Snapshot(
      GitHubObjectClient.Revision revision, GitHubTreeIndex index) {}

  private record PendingOperation(
      CompletableFuture<?> result, String operation, String path, String target) {}

  private record PageCursor(String revision, int offset) {}

  private final GitHubObjectClient client;
  private final String repository;
  private final String reference;
  private final String root;
  private final MountMode mode;
  private final String display;
  private final String commitMessagePrefix;
  private final ScheduledExecutorService scheduler;
  private final AtomicBoolean closed = new AtomicBoolean();
  private final Object lifecycle = new Object();
  private final Set<PendingOperation> pending = ConcurrentHashMap.newKeySet();
  private final Snapshot immutableSnapshot;
  private volatile String currentRevision;

  GitHubFilesystem(
      GitHubObjectClient client,
      String repository,
      String reference,
      String root,
      MountMode mode,
      String display,
      String commitMessagePrefix,
      ScheduledExecutorService scheduler,
      Snapshot initialSnapshot) {
    this.client = Objects.requireNonNull(client, "GitHub object client");
    this.repository = Objects.requireNonNull(repository, "GitHub repository");
    this.reference = Objects.requireNonNull(reference, "GitHub reference");
    this.root = HaraLogicalPath.normalise(root);
    this.mode = Objects.requireNonNull(mode, "GitHub mount mode");
    this.display = Objects.requireNonNull(display, "GitHub filesystem display");
    this.commitMessagePrefix = sanitizePrefix(commitMessagePrefix);
    this.scheduler = Objects.requireNonNull(scheduler, "filesystem scheduler");
    this.immutableSnapshot = mode == MountMode.READ_ONLY ? initialSnapshot : null;
    this.currentRevision = initialSnapshot.revision().commitSha();
  }

  @Override
  public Descriptor descriptor() {
    return new Descriptor(
        "github",
        display,
        mode == MountMode.READ_ONLY,
        mode == MountMode.READ_ONLY
            ? Capabilities.of(Capability.READ, Capability.ENTRIES, Capability.REVISION_CHECK)
            : Capabilities.of(
                Capability.READ,
                Capability.WRITE,
                Capability.ENTRIES,
                Capability.DELETE,
                Capability.COPY,
                Capability.MOVE,
                Capability.REVISION_CHECK),
        currentRevision,
        Map.of(
            "provider/repository", repository,
            "provider/ref", reference,
            "provider/root", root));
  }

  @Override
  public CompletionStage<Entry> stat(CallContext context, String path) {
    String logical = normalise(path);
    return submit(
        context,
        "stat",
        logical,
        null,
        () ->
            snapshot(context, "stat", logical, null)
                .thenApply(value -> entry(requireNode(value.index(), logical, "stat", null))));
  }

  @Override
  public CompletionStage<byte[]> read(CallContext context, String path) {
    String logical = normalise(path);
    return submit(
        context,
        "read",
        logical,
        null,
        () ->
            snapshot(context, "read", logical, null)
                .thenCompose(
                    value -> {
                      GitHubTreeIndex.Node node =
                          requireNode(value.index(), logical, "read", null);
                      if (node.type() == EntryType.DIRECTORY) {
                        return failed(
                            failure(
                                "is-directory",
                                "path is a directory",
                                "read",
                                logical,
                                null,
                                "tree",
                                false,
                                null));
                      }
                      if (node.type() != EntryType.FILE) {
                        return failed(
                            failure(
                                "unsupported",
                                "GitHub links and gitlinks are not followed",
                                "read",
                                logical,
                                null,
                                "no-follow",
                                false,
                                null));
                      }
                      return client.readBlob(repository, node.sha()).thenApply(byte[]::clone);
                    }));
  }

  @Override
  public CompletionStage<Mutation> write(
      CallContext context,
      String path,
      byte[] bytes,
      WriteOptions options,
      MutationContext mutation) {
    String logical = mutablePath(path, "write");
    Objects.requireNonNull(bytes, "filesystem bytes");
    Objects.requireNonNull(options, "write options");
    byte[] frozen = bytes.clone();
    return submit(
        context,
        "write",
        logical,
        null,
        () -> {
          requireWritable("write", logical, null);
          if (options.mode() == WriteMode.APPEND) {
            return failed(
                unsupported("write", logical, null, "append-unavailable"));
          }
          return snapshot(context, "write", logical, null)
              .thenCompose(
                  base -> {
                    GitHubTreeIndex.Node existing = base.index().find(logical);
                    checkExpected(
                        existing,
                        mutation == null ? null : mutation.expectedRevision(),
                        "write",
                        logical,
                        null);
                    if (options.mode() == WriteMode.CREATE && existing != null) {
                      return failed(
                          failure(
                              "already-exists",
                              "path already exists",
                              "write",
                              logical,
                              null,
                              "create-target-exists",
                              false,
                              null));
                    }
                    if (existing != null && existing.type() == EntryType.DIRECTORY) {
                      return failed(
                          failure(
                              "is-directory",
                              "path is a directory",
                              "write",
                              logical,
                              null,
                              "target-tree",
                              false,
                              null));
                    }
                    if (existing != null && existing.type() != EntryType.FILE) {
                      return failed(
                          unsupported("write", logical, null, "no-follow"));
                    }
                    try {
                      requireParent(base.index(), logical, options.parents(), "write", null);
                    } catch (Throwable error) {
                      return failed(error);
                    }
                    return client
                        .createBlob(repository, frozen)
                        .thenCompose(
                            blobSha -> {
                              String fileMode =
                                  existing != null && "100755".equals(existing.mode())
                                      ? "100755"
                                      : "100644";
                              GitHubObjectClient.TreeChange change =
                                  new GitHubObjectClient.TreeChange(
                                      base.index().repositoryPath(logical),
                                      fileMode,
                                      "blob",
                                      blobSha);
                              return commit(
                                  context,
                                  "write",
                                  logical,
                                  null,
                                  base,
                                  List.of(change),
                                  blobSha,
                                  logical);
                            });
                  });
        });
  }

  @Override
  public CompletionStage<EntryPage> entriesPage(
      CallContext context, String path, PageRequest request) {
    String logical = normalise(path);
    Objects.requireNonNull(request, "filesystem page request");
    return submit(
        context,
        "entries",
        logical,
        null,
        () ->
            snapshot(context, "entries", logical, null)
                .thenApply(
                    value -> {
                      GitHubTreeIndex.Node directory =
                          requireNode(value.index(), logical, "entries", null);
                      if (!directory.directory()) {
                        throw failure(
                            "not-directory",
                            "path is not a directory",
                            "entries",
                            logical,
                            null,
                            "not-tree",
                            false,
                            null);
                      }
                      List<GitHubTreeIndex.Node> children = value.index().children(logical);
                      PageCursor cursor = decodeCursor(request.token(), value.revision().commitSha());
                      int end =
                          Math.min(
                              children.size(),
                              Math.addExact(cursor.offset(), request.limit()));
                      ArrayList<Entry> entries = new ArrayList<>(end - cursor.offset());
                      for (GitHubTreeIndex.Node child :
                          children.subList(cursor.offset(), end)) {
                        entries.add(entry(child));
                      }
                      String next =
                          end < children.size()
                              ? encodeCursor(value.revision().commitSha(), end)
                              : null;
                      return new EntryPage(entries, next);
                    }));
  }

  @Override
  public CompletionStage<Mutation> mkdir(
      CallContext context, String path, MkdirOptions options, MutationContext mutation) {
    String logical = normalise(path);
    Objects.requireNonNull(options, "mkdir options");
    return submit(
        context,
        "mkdir",
        logical,
        null,
        () -> {
          requireWritable("mkdir", logical, null);
          return snapshot(context, "mkdir", logical, null)
              .thenCompose(
                  base -> {
                    GitHubTreeIndex.Node existing = base.index().find(logical);
                    checkExpected(
                        existing,
                        mutation == null ? null : mutation.expectedRevision(),
                        "mkdir",
                        logical,
                        null);
                    if (existing != null) {
                      if (existing.directory() && options.existsOk()) {
                        return CompletableFuture.completedFuture(
                            mutation(logical, existing.sha(), base.revision().commitSha()));
                      }
                      return failed(
                          failure(
                              "already-exists",
                              "path already exists",
                              "mkdir",
                              logical,
                              null,
                              "target-exists",
                              false,
                              null));
                    }
                    return failed(
                        unsupported("mkdir", logical, null, "empty-directory-unavailable"));
                  });
        });
  }

  @Override
  public CompletionStage<Mutation> delete(
      CallContext context, String path, DeleteOptions options, MutationContext mutation) {
    String logical = mutablePath(path, "delete");
    Objects.requireNonNull(options, "delete options");
    return submit(
        context,
        "delete",
        logical,
        null,
        () -> {
          requireWritable("delete", logical, null);
          return snapshot(context, "delete", logical, null)
              .thenCompose(
                  base -> {
                    GitHubTreeIndex.Node existing = base.index().find(logical);
                    if (existing == null) {
                      if (options.missingOk()) {
                        return CompletableFuture.completedFuture(
                            mutation(logical, null, base.revision().commitSha()));
                      }
                      return failed(notFound("delete", logical, null));
                    }
                    checkExpected(
                        existing,
                        mutation == null ? null : mutation.expectedRevision(),
                        "delete",
                        logical,
                        null);
                    if (existing.directory()
                        && !base.index().descendants(logical).isEmpty()) {
                      return failed(
                          failure(
                              "directory-not-empty",
                              "directory is not empty",
                              "delete",
                              logical,
                              null,
                              "tree-not-empty",
                              false,
                              null));
                    }
                    GitHubObjectClient.TreeChange deletion =
                        GitHubObjectClient.TreeChange.delete(
                            base.index().repositoryPath(logical));
                    return commit(
                        context,
                        "delete",
                        logical,
                        null,
                        base,
                        List.of(deletion),
                        null,
                        logical);
                  });
        });
  }

  @Override
  public CompletionStage<Mutation> copy(
      CallContext context,
      String source,
      String target,
      CopyOptions options,
      MutationContext mutation) {
    String logicalSource = normalise(source);
    String logicalTarget = mutablePath(target, "copy");
    Objects.requireNonNull(options, "copy options");
    return submit(
        context,
        "copy",
        logicalSource,
        logicalTarget,
        () -> {
          requireWritable("copy", logicalSource, logicalTarget);
          if (options.preserveModified()) {
            return failed(
                unsupported(
                    "copy",
                    logicalSource,
                    logicalTarget,
                    "preserve-modified-unavailable"));
          }
          if (logicalSource.equals(logicalTarget)) {
            return failed(
                failure(
                    "already-exists",
                    "source and target are the same path",
                    "copy",
                    logicalSource,
                    logicalTarget,
                    "same-path",
                    false,
                    null));
          }
          return snapshot(context, "copy", logicalSource, logicalTarget)
              .thenCompose(
                  base -> {
                    GitHubTreeIndex.Node sourceNode =
                        requireNode(base.index(), logicalSource, "copy", logicalTarget);
                    if (sourceNode.type() == EntryType.SYMLINK
                        || sourceNode.type() == EntryType.OTHER) {
                      return failed(
                          unsupported(
                              "copy", logicalSource, logicalTarget, "no-follow"));
                    }
                    checkExpected(
                        sourceNode,
                        mutation == null ? null : mutation.expectedRevision(),
                        "copy",
                        logicalSource,
                        logicalTarget);
                    GitHubTreeIndex.Node targetNode = base.index().find(logicalTarget);
                    checkExpected(
                        targetNode,
                        mutation == null ? null : mutation.expectedTargetRevision(),
                        "copy",
                        logicalSource,
                        logicalTarget);
                    if (targetNode != null && !options.replace()) {
                      return failed(
                          failure(
                              "already-exists",
                              "target already exists",
                              "copy",
                              logicalSource,
                              logicalTarget,
                              "target-exists",
                              false,
                              null));
                    }
                    if (sourceNode.directory()
                        && logicalTarget.startsWith(logicalSource + "/")) {
                      return failed(
                          failure(
                              "invalid-path",
                              "cannot copy a directory beneath itself",
                              "copy",
                              logicalSource,
                              logicalTarget,
                              "recursive-target",
                              false,
                              null));
                    }
                    try {
                      requireParent(
                          base.index(), logicalTarget, options.parents(), "copy", logicalSource);
                      List<GitHubObjectClient.TreeChange> changes =
                          transferChanges(
                              base.index(), sourceNode, logicalSource, logicalTarget, false);
                      return commit(
                          context,
                          "copy",
                          logicalSource,
                          logicalTarget,
                          base,
                          changes,
                          sourceNode.sha(),
                          logicalTarget);
                    } catch (Throwable error) {
                      return failed(error);
                    }
                  });
        });
  }

  @Override
  public CompletionStage<Mutation> move(
      CallContext context,
      String source,
      String target,
      MoveOptions options,
      MutationContext mutation) {
    String logicalSource = mutablePath(source, "move");
    String logicalTarget = mutablePath(target, "move");
    Objects.requireNonNull(options, "move options");
    return submit(
        context,
        "move",
        logicalSource,
        logicalTarget,
        () -> {
          requireWritable("move", logicalSource, logicalTarget);
          if (options.atomic()) {
            return failed(
                unsupported(
                    "move", logicalSource, logicalTarget, "atomic-move-unavailable"));
          }
          return snapshot(context, "move", logicalSource, logicalTarget)
              .thenCompose(
                  base -> {
                    GitHubTreeIndex.Node sourceNode =
                        requireNode(base.index(), logicalSource, "move", logicalTarget);
                    if (sourceNode.type() == EntryType.SYMLINK
                        || sourceNode.type() == EntryType.OTHER) {
                      return failed(
                          unsupported(
                              "move", logicalSource, logicalTarget, "no-follow"));
                    }
                    checkExpected(
                        sourceNode,
                        mutation == null ? null : mutation.expectedRevision(),
                        "move",
                        logicalSource,
                        logicalTarget);
                    if (logicalSource.equals(logicalTarget)) {
                      return CompletableFuture.completedFuture(
                          mutation(
                              logicalTarget,
                              sourceNode.sha(),
                              base.revision().commitSha()));
                    }
                    if (sourceNode.directory()
                        && logicalTarget.startsWith(logicalSource + "/")) {
                      return failed(
                          failure(
                              "invalid-path",
                              "cannot move a directory beneath itself",
                              "move",
                              logicalSource,
                              logicalTarget,
                              "recursive-target",
                              false,
                              null));
                    }
                    GitHubTreeIndex.Node targetNode = base.index().find(logicalTarget);
                    checkExpected(
                        targetNode,
                        mutation == null ? null : mutation.expectedTargetRevision(),
                        "move",
                        logicalSource,
                        logicalTarget);
                    if (targetNode != null && !options.replace()) {
                      return failed(
                          failure(
                              "already-exists",
                              "target already exists",
                              "move",
                              logicalSource,
                              logicalTarget,
                              "target-exists",
                              false,
                              null));
                    }
                    try {
                      requireParent(
                          base.index(), logicalTarget, options.parents(), "move", logicalSource);
                      List<GitHubObjectClient.TreeChange> changes =
                          transferChanges(
                              base.index(), sourceNode, logicalSource, logicalTarget, true);
                      return commit(
                          context,
                          "move",
                          logicalSource,
                          logicalTarget,
                          base,
                          changes,
                          sourceNode.sha(),
                          logicalTarget);
                    } catch (Throwable error) {
                      return failed(error);
                    }
                  });
        });
  }

  @Override
  public CompletionStage<Void> close(CallContext context) {
    Objects.requireNonNull(context, "filesystem call context");
    CompletableFuture<Void> result = new CompletableFuture<>();
    try {
      context.check("github", "close", null, null);
      List<PendingOperation> operations;
      synchronized (lifecycle) {
        if (!closed.compareAndSet(false, true)) {
          result.complete(null);
          return result;
        }
        operations = List.copyOf(pending);
        pending.clear();
      }
      for (PendingOperation operation : operations) {
        operation
            .result()
            .completeExceptionally(
                FilesystemException.providerClosed(
                    "github",
                    operation.operation(),
                    operation.path(),
                    operation.target()));
      }
      result.complete(null);
    } catch (Throwable error) {
      result.completeExceptionally(error);
    }
    return result;
  }

  private CompletionStage<Snapshot> snapshot(
      CallContext context, String operation, String path, String target) {
    if (immutableSnapshot != null) {
      return CompletableFuture.completedFuture(immutableSnapshot);
    }
    context.check("github", operation, path, target);
    return client
        .resolveRevision(repository, reference)
        .thenCompose(
            revision ->
                client
                    .readTree(repository, revision.treeSha())
                    .thenApply(
                        tree -> {
                          context.check("github", operation, path, target);
                          Snapshot value =
                              new Snapshot(revision, new GitHubTreeIndex(tree, root));
                          currentRevision = revision.commitSha();
                          return value;
                        }));
  }

  private CompletionStage<Mutation> commit(
      CallContext context,
      String operation,
      String path,
      String target,
      Snapshot base,
      List<GitHubObjectClient.TreeChange> changes,
      String entryRevision,
      String resultPath) {
    context.check("github", operation, path, target);
    return client
        .createTree(repository, base.revision().treeSha(), List.copyOf(changes))
        .thenCompose(
            treeSha ->
                client.createCommit(
                    repository,
                    commitMessage(operation, path, target),
                    treeSha,
                    base.revision().commitSha()))
        .thenCompose(
            commitSha ->
                client
                    .updateReference(
                        repository,
                        reference,
                        base.revision().commitSha(),
                        commitSha)
                    .thenApply(
                        ignored -> {
                          currentRevision = commitSha;
                          return mutation(resultPath, entryRevision, commitSha);
                        }));
  }

  private <T> CompletionStage<T> submit(
      CallContext context,
      String operation,
      String path,
      String target,
      AsyncOperation<T> action) {
    Objects.requireNonNull(context, "filesystem call context");
    Objects.requireNonNull(action, "filesystem operation");
    CompletableFuture<T> result = new CompletableFuture<>();
    try {
      context.check("github", operation, path, target);
    } catch (Throwable error) {
      result.completeExceptionally(mapFailure(error, operation, path, target));
      return result;
    }

    PendingOperation pendingOperation =
        new PendingOperation(result, operation, path, target);
    synchronized (lifecycle) {
      if (closed.get()) {
        result.completeExceptionally(
            FilesystemException.providerClosed("github", operation, path, target));
        return result;
      }
      pending.add(pendingOperation);
    }
    AutoCloseable cancellation =
        context.onCancel(
            () ->
                result.completeExceptionally(
                    FilesystemException.cancelled("github", operation, path, target)));
    ScheduledFuture<?> deadline =
        scheduleDeadline(context, result, operation, path, target);
    result.whenComplete(
        (value, error) -> {
          pending.remove(pendingOperation);
          if (deadline != null) deadline.cancel(false);
          try {
            cancellation.close();
          } catch (Exception ignored) {
            // Cancellation registration is an in-memory removal only.
          }
        });

    try {
      action
          .call()
          .whenComplete(
              (value, error) -> {
                if (result.isDone()) return;
                if (error != null) {
                  result.completeExceptionally(
                      mapFailure(error, operation, path, target));
                  return;
                }
                try {
                  context.check("github", operation, path, target);
                  if (closed.get()) {
                    throw FilesystemException.providerClosed(
                        "github", operation, path, target);
                  }
                  result.complete(value);
                } catch (Throwable failure) {
                  result.completeExceptionally(
                      mapFailure(failure, operation, path, target));
                }
              });
    } catch (Throwable error) {
      result.completeExceptionally(mapFailure(error, operation, path, target));
    }
    return result;
  }

  private ScheduledFuture<?> scheduleDeadline(
      CallContext context,
      CompletableFuture<?> result,
      String operation,
      String path,
      String target) {
    if (!context.hasDeadline()) return null;
    try {
      return scheduler.schedule(
          () ->
              result.completeExceptionally(
                  FilesystemException.timeout("github", operation, path, target)),
          context.remainingNanos(),
          TimeUnit.NANOSECONDS);
    } catch (Throwable error) {
      result.completeExceptionally(mapFailure(error, operation, path, target));
      return null;
    }
  }

  private List<GitHubObjectClient.TreeChange> transferChanges(
      GitHubTreeIndex index,
      GitHubTreeIndex.Node source,
      String sourcePath,
      String targetPath,
      boolean removeSource) {
    ArrayList<GitHubObjectClient.TreeChange> changes = new ArrayList<>();
    if (!source.directory()) {
      changes.add(
          new GitHubObjectClient.TreeChange(
              index.repositoryPath(targetPath),
              source.mode(),
              gitType(source),
              source.sha()));
    } else if (source.sha() != null) {
      changes.add(
          new GitHubObjectClient.TreeChange(
              index.repositoryPath(targetPath), "040000", "tree", source.sha()));
    } else {
      List<GitHubTreeIndex.Node> descendants = index.descendants(sourcePath);
      boolean copied = false;
      for (GitHubTreeIndex.Node node : descendants) {
        if (node.directory() || node.sha() == null) continue;
        String suffix = node.path().substring(sourcePath.length());
        changes.add(
            new GitHubObjectClient.TreeChange(
                index.repositoryPath(targetPath + suffix),
                node.mode(),
                gitType(node),
                node.sha()));
        copied = true;
      }
      if (!copied) {
        throw unsupported(
            removeSource ? "move" : "copy",
            sourcePath,
            targetPath,
            "empty-directory-unavailable");
      }
    }
    if (removeSource) {
      changes.add(GitHubObjectClient.TreeChange.delete(index.repositoryPath(sourcePath)));
    }
    return List.copyOf(changes);
  }

  private void requireParent(
      GitHubTreeIndex index,
      String path,
      boolean parents,
      String operation,
      String source) {
    String parent = HaraLogicalPath.parent(path);
    while (parent != null) {
      GitHubTreeIndex.Node node = index.find(parent);
      if (node != null) {
        if (!node.directory()) {
          throw failure(
              "not-directory",
              "path ancestor is not a directory",
              operation,
              source == null ? path : source,
              source == null ? null : path,
              "non-tree-ancestor",
              false,
              null);
        }
        return;
      }
      if (!parents) {
        throw notFound(
            operation,
            source == null ? path : source,
            source == null ? null : path);
      }
      parent = HaraLogicalPath.parent(parent);
    }
  }

  private void requireWritable(String operation, String path, String target) {
    if (mode == MountMode.READ_ONLY) {
      throw failure(
          "permission-denied",
          "GitHub revision mount is read-only",
          operation,
          path,
          target,
          "read-only-mount",
          false,
          null);
    }
  }

  private static void checkExpected(
      GitHubTreeIndex.Node node,
      String expected,
      String operation,
      String path,
      String target) {
    if (expected == null) return;
    String actual = node == null ? null : node.sha();
    if (!Objects.equals(expected, actual)) {
      throw failure(
          "conflict",
          "GitHub entry revision does not match",
          operation,
          path,
          target,
          actual == null ? "revision-missing" : "revision-mismatch",
          true,
          null);
    }
  }

  private static GitHubTreeIndex.Node requireNode(
      GitHubTreeIndex index, String path, String operation, String target) {
    GitHubTreeIndex.Node node = index.find(path);
    if (node == null) throw notFound(operation, path, target);
    return node;
  }

  private static Entry entry(GitHubTreeIndex.Node node) {
    LinkedHashMap<String, Object> extensions = new LinkedHashMap<>();
    extensions.put("provider/mode", node.mode());
    return new Entry(
        node.path(),
        node.name(),
        node.type(),
        node.size(),
        null,
        node.sha(),
        node.sha(),
        null,
        extensions);
  }

  private String commitMessage(String operation, String path, String target) {
    String suffix = target == null ? path : path + " -> " + target;
    return commitMessagePrefix + ": " + operation + " " + suffix;
  }

  private static String sanitizePrefix(String prefix) {
    String value = Objects.requireNonNull(prefix, "GitHub commit message prefix").trim();
    if (value.isEmpty() || value.length() > 120 || value.indexOf('\n') >= 0 || value.indexOf('\r') >= 0) {
      throw new IllegalArgumentException("invalid GitHub commit message prefix");
    }
    return value;
  }

  private static String mutablePath(String path, String operation) {
    String logical = normalise(path);
    if ("/".equals(logical)) {
      throw failure(
          "denied",
          "cannot mutate the mounted root",
          operation,
          logical,
          null,
          "root-mutation",
          false,
          null);
    }
    return logical;
  }

  private static String normalise(String path) {
    return HaraLogicalPath.normalise(Objects.requireNonNull(path, "filesystem path"));
  }

  private static Mutation mutation(String path, String revision, String mountRevision) {
    return new Mutation(path, revision, mountRevision, Map.of());
  }

  private static String gitType(GitHubTreeIndex.Node node) {
    return node.type() == EntryType.OTHER ? "commit" : node.directory() ? "tree" : "blob";
  }

  private static String encodeCursor(String revision, int offset) {
    String value = revision + ":" + offset;
    return Base64.getUrlEncoder()
        .withoutPadding()
        .encodeToString(value.getBytes(StandardCharsets.UTF_8));
  }

  private static PageCursor decodeCursor(String token, String revision) {
    if (token == null) return new PageCursor(revision, 0);
    try {
      String decoded =
          new String(Base64.getUrlDecoder().decode(token), StandardCharsets.UTF_8);
      int separator = decoded.lastIndexOf(':');
      if (separator <= 0) throw new IllegalArgumentException();
      String tokenRevision = decoded.substring(0, separator);
      int offset = Integer.parseInt(decoded.substring(separator + 1));
      if (offset < 0) throw new IllegalArgumentException();
      if (!revision.equals(tokenRevision)) {
        throw failure(
            "conflict",
            "GitHub page token belongs to another revision",
            "entries",
            null,
            null,
            "stale-page-token",
            true,
            null);
      }
      return new PageCursor(revision, offset);
    } catch (FilesystemException error) {
      throw error;
    } catch (RuntimeException error) {
      throw failure(
          "invalid-path",
          "invalid GitHub filesystem page token",
          "entries",
          null,
          null,
          "invalid-page-token",
          false,
          error);
    }
  }

  private static FilesystemException unsupported(
      String operation, String path, String target, String providerCode) {
    return failure(
        "unsupported",
        "GitHub filesystem cannot honor the requested semantics",
        operation,
        path,
        target,
        providerCode,
        false,
        null);
  }

  private static FilesystemException notFound(
      String operation, String path, String target) {
    return failure(
        "not-found",
        "GitHub path does not exist",
        operation,
        path,
        target,
        "path-not-found",
        false,
        null);
  }

  private static FilesystemException failure(
      String code,
      String message,
      String operation,
      String path,
      String target,
      String providerCode,
      boolean retryable,
      Throwable cause) {
    return new FilesystemException(
        code,
        message,
        "github",
        operation,
        path,
        target,
        providerCode,
        retryable,
        cause);
  }

  private static FilesystemException mapFailure(
      Throwable error, String operation, String path, String target) {
    Throwable cause = unwrap(error);
    if (cause instanceof FilesystemException filesystem) {
      if (filesystem.operation() != null) return filesystem;
      return failure(
          filesystem.code(),
          filesystem.getMessage(),
          operation,
          path,
          target,
          filesystem.providerCode(),
          filesystem.retryable(),
          filesystem.getCause());
    }
    if (cause instanceof GitHubObjectClient.Failure github) {
      String code =
          switch (github.kind()) {
            case NOT_FOUND -> "not-found";
            case AUTHENTICATION -> "authentication-failed";
            case PERMISSION -> "permission-denied";
            case RATE_LIMITED -> "rate-limited";
            case OFFLINE -> "offline";
            case CONFLICT -> "conflict";
            case UNSUPPORTED -> "unsupported";
            case IO -> "io";
          };
      return failure(
          code,
          github.getMessage() == null ? "GitHub filesystem operation failed" : github.getMessage(),
          operation,
          path,
          target,
          github.providerCode(),
          github.retryable(),
          github);
    }
    if (cause instanceof HaraLogicalPath.Error logical) {
      return failure(
          logical.code(),
          logical.getMessage(),
          operation,
          path,
          target,
          logical.code(),
          false,
          logical);
    }
    return failure(
        "io",
        cause.getMessage() == null ? "GitHub filesystem operation failed" : cause.getMessage(),
        operation,
        path,
        target,
        cause.getClass().getSimpleName(),
        false,
        cause);
  }

  private static Throwable unwrap(Throwable error) {
    Throwable current = error;
    while ((current instanceof CompletionException
            || current instanceof java.util.concurrent.ExecutionException)
        && current.getCause() != null) {
      current = current.getCause();
    }
    return current;
  }

  private static <T> CompletionStage<T> failed(Throwable error) {
    return CompletableFuture.failedFuture(error);
  }
}
