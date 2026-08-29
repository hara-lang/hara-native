package hara.truffle;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Kernel-ready ownership table for opened provider-neutral filesystem capabilities.
 *
 * <p>The table keeps provider construction, redacted descriptors, attachment ownership, and
 * provider close in one place. A synchronous Graal filesystem is an optional local-only adapter;
 * remote providers never need to implement {@code java.nio.file.Path} or Graal's FileSystem API.
 */
final class FilesystemMountTable implements AutoCloseable {
  @FunctionalInterface
  interface GraalAdapterFactory {
    HaraMountedFileSystem create(Map<String, ?> configuration);
  }

  @FunctionalInterface
  interface AttachmentInstaller {
    void install(OpenedMount mount);
  }

  @FunctionalInterface
  interface AttachmentRemover {
    void remove(SessionModel.SessionMountId mountId);
  }

  record AttachmentKey(String kind, String id) {
    AttachmentKey {
      if (kind == null || !kind.matches("[a-z][a-z0-9-]*")) {
        throw new IllegalArgumentException("invalid filesystem attachment kind");
      }
      if (id == null || id.isBlank()) {
        throw new IllegalArgumentException("invalid filesystem attachment id");
      }
    }

    static AttachmentKey session(SessionModel.SessionId id) {
      return new AttachmentKey("session", Objects.requireNonNull(id, "session id").value());
    }

    static AttachmentKey sandbox(SandboxModel.SandboxId id) {
      return new AttachmentKey(
          "sandbox", Long.toString(Objects.requireNonNull(id, "sandbox id").value()));
    }
  }

  record Info(
      SessionModel.SessionMountId id,
      IFilesystem.Descriptor descriptor,
      int attachments,
      boolean sourceLoadable) {
    Info {
      Objects.requireNonNull(id, "filesystem mount id");
      Objects.requireNonNull(descriptor, "filesystem descriptor");
      if (attachments < 0) throw new IllegalArgumentException("negative filesystem attachments");
    }
  }

  record OpenedMount(
      SessionModel.SessionMountId id,
      IFilesystem filesystem,
      IFilesystem.Descriptor descriptor,
      HaraMountedFileSystem graalFilesystem) {
    OpenedMount {
      Objects.requireNonNull(id, "filesystem mount id");
      Objects.requireNonNull(filesystem, "opened filesystem");
      Objects.requireNonNull(descriptor, "filesystem descriptor");
    }

    boolean sourceLoadable() {
      return graalFilesystem != null;
    }
  }

  private static final class Mount {
    final IFilesystem filesystem;
    final IFilesystem.Descriptor admittedDescriptor;
    final HaraMountedFileSystem graalFilesystem;
    int attachments;

    Mount(
        IFilesystem filesystem,
        IFilesystem.Descriptor admittedDescriptor,
        HaraMountedFileSystem graalFilesystem) {
      this.filesystem = filesystem;
      this.admittedDescriptor = admittedDescriptor;
      this.graalFilesystem = graalFilesystem;
    }
  }

  private final FilesystemProviderRegistry providers;
  private final IFilesystemFactory.OpenContext openContext;
  private final ConcurrentHashMap<Long, Mount> mounts = new ConcurrentHashMap<>();
  private final ConcurrentHashMap<AttachmentKey, SessionModel.SessionMountId> attachments =
      new ConcurrentHashMap<>();
  private final List<JvmPackageLoader.LoadedProvider> loadedProviders = new ArrayList<>();
  private final AtomicLong nextId = new AtomicLong(1);
  private final AtomicBoolean closed = new AtomicBoolean();

  FilesystemMountTable(IFilesystemFactory.OpenContext openContext) {
    this(
        new FilesystemProviderRegistry().register(new NativeFilesystem.Factory()),
        openContext);
  }

  FilesystemMountTable(
      FilesystemProviderRegistry providers,
      IFilesystemFactory.OpenContext openContext) {
    this.providers = Objects.requireNonNull(providers, "filesystem provider registry");
    this.openContext = Objects.requireNonNull(openContext, "filesystem open context");
  }

  synchronized FilesystemMountTable register(IFilesystemFactory factory) {
    requireOpen();
    providers.register(factory);
    return this;
  }

  synchronized void loadJvmProvider(JvmPackageLoader.Selection selection) {
    requireOpen();
    loadedProviders.add(JvmPackageLoader.load(selection, providers));
  }

  boolean supports(String kind) {
    return providers.contains(kind);
  }

  CompletionStage<SessionModel.SessionMountId> open(
      String kind, Map<String, ?> configuration) {
    return open(kind, configuration, null);
  }

  CompletionStage<SessionModel.SessionMountId> openNative(Path root) {
    Objects.requireNonNull(root, "native filesystem root");
    Path normalized = root.toAbsolutePath().normalize();
    if (!Files.isDirectory(normalized)) {
      return CompletableFuture.failedFuture(
          new IllegalArgumentException("FILESYSTEM_NOT_FOUND " + normalized));
    }
    Map<String, ?> configuration = Map.of("root", normalized.toString());
    return open(
        "native",
        configuration,
        ignored -> new HaraMountedFileSystem(normalized));
  }

  CompletionStage<SessionModel.SessionMountId> openSourceLoadable(
      String kind,
      Map<String, ?> configuration,
      GraalAdapterFactory graalAdapterFactory) {
    Objects.requireNonNull(graalAdapterFactory, "Graal filesystem adapter factory");
    return open(kind, configuration, graalAdapterFactory);
  }

  private CompletionStage<SessionModel.SessionMountId> open(
      String kind,
      Map<String, ?> configuration,
      GraalAdapterFactory graalAdapterFactory) {
    try {
      requireOpen();
    } catch (Throwable error) {
      return CompletableFuture.failedFuture(error);
    }
    Map<String, ?> frozen = Map.copyOf(Objects.requireNonNull(configuration, "configuration"));
    return providers
        .open(kind, openContext, frozen)
        .thenCompose(
            filesystem ->
                publish(
                    Objects.requireNonNull(filesystem, "opened filesystem"),
                    kind,
                    frozen,
                    graalAdapterFactory));
  }

  private CompletionStage<SessionModel.SessionMountId> publish(
      IFilesystem filesystem,
      String requestedKind,
      Map<String, ?> configuration,
      GraalAdapterFactory graalAdapterFactory) {
    try {
      IFilesystem.Descriptor descriptor =
          Objects.requireNonNull(filesystem.descriptor(), "filesystem descriptor");
      if (!requestedKind.equals(descriptor.kind())) {
        throw new IllegalStateException(
            "FILESYSTEM_PROVIDER_KIND_MISMATCH "
                + requestedKind
                + " "
                + descriptor.kind());
      }
      HaraMountedFileSystem graalFilesystem =
          graalAdapterFactory == null ? null : graalAdapterFactory.create(configuration);
      synchronized (this) {
        requireOpen();
        long value = nextId.getAndIncrement();
        if (value <= 0) throw new IllegalStateException("FILESYSTEM_IDS_EXHAUSTED");
        SessionModel.SessionMountId id = SessionModel.SessionMountId.of(value);
        mounts.put(value, new Mount(filesystem, descriptor, graalFilesystem));
        return CompletableFuture.completedFuture(id);
      }
    } catch (Throwable error) {
      return rejectAfterClose(filesystem, error);
    }
  }

  IFilesystem filesystem(SessionModel.SessionMountId id) {
    return requireMount(id).filesystem;
  }

  HaraMountedFileSystem graalFilesystem(SessionModel.SessionMountId id) {
    return requireMount(id).graalFilesystem;
  }

  synchronized Info info(SessionModel.SessionMountId id) {
    Mount mount = requireMount(id);
    return new Info(
        id,
        currentDescriptor(mount),
        mount.attachments,
        mount.graalFilesystem != null);
  }

  synchronized SessionModel.SessionMountId attachment(AttachmentKey key) {
    Objects.requireNonNull(key, "filesystem attachment key");
    return attachments.get(key);
  }

  /**
   * Installs one exact opened capability for an owner and commits attachment accounting only after
   * the owner successfully accepts it. Replacing an attachment releases the previous mount exactly
   * once. A failed install leaves the previous attachment unchanged.
   */
  synchronized SessionModel.SessionMountId attach(
      AttachmentKey key,
      SessionModel.SessionMountId id,
      AttachmentInstaller installer) {
    requireOpen();
    Objects.requireNonNull(key, "filesystem attachment key");
    Objects.requireNonNull(installer, "filesystem attachment installer");
    Mount mount = requireMount(id);
    SessionModel.SessionMountId previousId = attachments.get(key);
    if (id.equals(previousId)) return previousId;

    increment(mount, id);
    try {
      installer.install(opened(id, mount));
    } catch (Throwable error) {
      decrement(mount, id);
      throw error;
    }

    if (previousId != null) decrement(requireMount(previousId), previousId);
    attachments.put(key, id);
    return previousId;
  }

  /** Detaches an owner only after its runtime has accepted the removal. */
  synchronized SessionModel.SessionMountId detach(
      AttachmentKey key, AttachmentRemover remover) {
    Objects.requireNonNull(key, "filesystem attachment key");
    Objects.requireNonNull(remover, "filesystem attachment remover");
    SessionModel.SessionMountId id = attachments.get(key);
    if (id == null) return null;
    remover.remove(id);
    attachments.remove(key);
    decrement(requireMount(id), id);
    return id;
  }

  /** Releases accounting after an owner such as a closing Session has already detached itself. */
  synchronized SessionModel.SessionMountId releaseAttachment(AttachmentKey key) {
    Objects.requireNonNull(key, "filesystem attachment key");
    SessionModel.SessionMountId id = attachments.remove(key);
    if (id != null) decrement(requireMount(id), id);
    return id;
  }

  synchronized void retain(SessionModel.SessionMountId id) {
    increment(requireMount(id), id);
  }

  synchronized void release(SessionModel.SessionMountId id) {
    decrement(requireMount(id), id);
  }

  CompletionStage<Void> close(SessionModel.SessionMountId id) {
    final Mount mount;
    synchronized (this) {
      mount = requireMount(id);
      if (mount.attachments != 0) {
        return CompletableFuture.failedFuture(
            new IllegalArgumentException("FILESYSTEM_ATTACHED " + id));
      }
      mounts.remove(id.value());
    }
    return closeProvider(mount.filesystem);
  }

  synchronized int size() {
    return mounts.size();
  }

  synchronized int attachmentCount() {
    return attachments.size();
  }

  CompletionStage<Void> closeAll() {
    List<Mount> owned;
    List<JvmPackageLoader.LoadedProvider> providersToClose;
    synchronized (this) {
      if (!closed.compareAndSet(false, true)) {
        return CompletableFuture.completedFuture(null);
      }
      attachments.clear();
      owned = new ArrayList<>(mounts.values());
      mounts.clear();
      providersToClose = new ArrayList<>(loadedProviders);
      loadedProviders.clear();
    }
    CompletableFuture<?>[] closing =
        owned.stream()
            .map(mount -> closeProvider(mount.filesystem).toCompletableFuture())
            .toArray(CompletableFuture[]::new);
    return CompletableFuture.allOf(closing)
        .handle(
            (ignored, mountFailure) -> {
              RuntimeException failure =
                  mountFailure == null ? null : asRuntimeFailure(unwrap(mountFailure));
              for (int index = providersToClose.size() - 1; index >= 0; index--) {
                try {
                  providersToClose.get(index).close();
                } catch (Exception error) {
                  if (failure == null) failure = asRuntimeFailure(error);
                  else failure.addSuppressed(error);
                }
              }
              if (failure != null) throw failure;
              return null;
            });
  }

  @Override
  public void close() {
    try {
      closeAll().toCompletableFuture().join();
    } catch (CompletionException error) {
      Throwable cause = unwrap(error);
      if (cause instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }

  private static OpenedMount opened(SessionModel.SessionMountId id, Mount mount) {
    return new OpenedMount(
        id,
        mount.filesystem,
        currentDescriptor(mount),
        mount.graalFilesystem);
  }

  private static IFilesystem.Descriptor currentDescriptor(Mount mount) {
    IFilesystem.Descriptor current =
        Objects.requireNonNull(mount.filesystem.descriptor(), "filesystem descriptor");
    IFilesystem.Descriptor admitted = mount.admittedDescriptor;
    if (!admitted.kind().equals(current.kind())
        || admitted.readOnly() != current.readOnly()
        || !admitted.capabilities().equals(current.capabilities())) {
      throw new IllegalStateException(
          "FILESYSTEM_DESCRIPTOR_AUTHORITY_CHANGED " + admitted.kind());
    }
    return current;
  }

  private synchronized Mount requireMount(SessionModel.SessionMountId id) {
    Objects.requireNonNull(id, "filesystem mount id");
    Mount mount = mounts.get(id.value());
    if (mount == null) throw new IllegalArgumentException("NO_FILESYSTEM " + id);
    return mount;
  }

  private static void increment(Mount mount, SessionModel.SessionMountId id) {
    if (mount.attachments == Integer.MAX_VALUE) {
      throw new IllegalStateException("FILESYSTEM_ATTACHMENTS_EXHAUSTED " + id);
    }
    mount.attachments++;
  }

  private static void decrement(Mount mount, SessionModel.SessionMountId id) {
    if (mount.attachments == 0) {
      throw new IllegalStateException("FILESYSTEM_ATTACHMENT_UNDERFLOW " + id);
    }
    mount.attachments--;
  }

  private void requireOpen() {
    if (closed.get()) throw new IllegalStateException("FILESYSTEM_TABLE_CLOSED");
  }

  private static CompletionStage<Void> closeProvider(IFilesystem filesystem) {
    try {
      CompletionStage<Void> closing =
          filesystem.close(IFilesystem.CallContext.create());
      return Objects.requireNonNull(closing, "filesystem close stage");
    } catch (Throwable error) {
      return CompletableFuture.failedFuture(error);
    }
  }

  private static CompletionStage<SessionModel.SessionMountId> rejectAfterClose(
      IFilesystem filesystem, Throwable original) {
    CompletableFuture<SessionModel.SessionMountId> rejected = new CompletableFuture<>();
    closeProvider(filesystem)
        .whenComplete(
            (ignored, closeError) -> {
              if (closeError != null) original.addSuppressed(unwrap(closeError));
              rejected.completeExceptionally(original);
            });
    return rejected;
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

  private static RuntimeException asRuntimeFailure(Throwable error) {
    return error instanceof RuntimeException runtime
        ? runtime
        : new IllegalStateException("filesystem provider close failed", error);
  }
}
