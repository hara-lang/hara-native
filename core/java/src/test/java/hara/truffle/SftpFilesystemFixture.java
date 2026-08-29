package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicBoolean;

final class SftpFilesystemFixture implements AutoCloseable {
  final MemoryClient client = new MemoryClient();

  SftpFilesystemFixture() {
    client.directory("/srv");
    client.directory("/srv/application");
    client.file("/srv/application/README.md", "hello".getBytes(StandardCharsets.UTF_8));
    client.directory("/srv/application/data");
    client.file("/srv/application/data/b.bin", new byte[] {2});
    client.file("/srv/application/data/a.bin", new byte[] {1});
  }

  @Override
  public void close() throws Exception {
    client.close();
  }

  static final class MemoryClient implements SftpFilesystem.Client {
    private record Node(
        IFilesystem.EntryType type,
        byte[] bytes,
        long modifiedAt,
        String id,
        String revision) {}

    private final Map<String, Node> nodes = new HashMap<>();
    private final AtomicBoolean closed = new AtomicBoolean();
    private final Set<IFilesystem.Capability> capabilities =
        new HashSet<>(
            Set.of(
                IFilesystem.Capability.READ,
                IFilesystem.Capability.WRITE,
                IFilesystem.Capability.ENTRIES,
                IFilesystem.Capability.MKDIR,
                IFilesystem.Capability.DELETE,
                IFilesystem.Capability.MOVE,
                IFilesystem.Capability.APPEND,
                IFilesystem.Capability.REVISION_CHECK));
    boolean authenticated = true;
    boolean hostKeyVerified = true;
    int sequence = 1;

    @Override
    public boolean authenticated() {
      return authenticated;
    }

    @Override
    public boolean hostKeyVerified() {
      return hostKeyVerified;
    }

    @Override
    public Set<IFilesystem.Capability> capabilities() {
      return Set.copyOf(capabilities);
    }

    @Override
    public synchronized SftpFilesystem.RemoteEntry lstat(String path) throws Exception {
      requireOpen();
      Node node = nodes.get(normal(path));
      if (node == null) throw missing();
      return remote(path, node);
    }

    @Override
    public synchronized byte[] read(String path, long maxBytes) throws Exception {
      requireOpen();
      Node node = nodes.get(normal(path));
      if (node == null) throw missing();
      if (node.type() != IFilesystem.EntryType.FILE) {
        throw new SftpFilesystem.ClientFailure("unsupported", "fixture-not-file", false);
      }
      if (node.bytes().length > maxBytes) {
        throw new SftpFilesystem.ClientFailure("quota-exceeded", "fixture-transfer-limit", false);
      }
      return node.bytes().clone();
    }

    @Override
    public synchronized void write(
        String path,
        byte[] bytes,
        IFilesystem.WriteMode mode,
        IFilesystem.MutationContext mutation)
        throws Exception {
      requireOpen();
      String key = normal(path);
      Node existing = nodes.get(key);
      checkExpected(existing, mutation.expectedRevision());
      if (mode == IFilesystem.WriteMode.CREATE && existing != null) {
        throw new SftpFilesystem.ClientFailure("already-exists", "SSH_FX_FILE_ALREADY_EXISTS", false);
      }
      if (existing != null && existing.type() != IFilesystem.EntryType.FILE) {
        throw new SftpFilesystem.ClientFailure("is-directory", "fixture-not-file", false);
      }
      byte[] value = bytes.clone();
      if (mode == IFilesystem.WriteMode.APPEND && existing != null) {
        byte[] joined = Arrays.copyOf(existing.bytes(), existing.bytes().length + bytes.length);
        System.arraycopy(bytes, 0, joined, existing.bytes().length, bytes.length);
        value = joined;
      }
      nodes.put(
          key,
          new Node(
              IFilesystem.EntryType.FILE,
              value,
              sequence,
              "id:" + key,
              "r" + sequence++));
    }

    @Override
    public synchronized List<SftpFilesystem.RemoteEntry> entries(String path) throws Exception {
      requireOpen();
      String parent = normal(path);
      Node directory = nodes.get(parent);
      if (directory == null) throw missing();
      if (directory.type() != IFilesystem.EntryType.DIRECTORY) {
        throw new SftpFilesystem.ClientFailure("not-directory", "SSH_FX_NOT_A_DIRECTORY", false);
      }
      String prefix = "/".equals(parent) ? "/" : parent + "/";
      ArrayList<SftpFilesystem.RemoteEntry> values = new ArrayList<>();
      for (Map.Entry<String, Node> value : nodes.entrySet()) {
        if (!value.getKey().startsWith(prefix)) continue;
        String remainder = value.getKey().substring(prefix.length());
        if (remainder.isEmpty() || remainder.contains("/")) continue;
        values.add(remote(value.getKey(), value.getValue()));
      }
      return values;
    }

    @Override
    public synchronized void mkdir(String path, IFilesystem.MutationContext mutation)
        throws Exception {
      requireOpen();
      String key = normal(path);
      if (nodes.containsKey(key)) {
        throw new SftpFilesystem.ClientFailure("already-exists", "SSH_FX_FILE_ALREADY_EXISTS", false);
      }
      nodes.put(
          key,
          new Node(
              IFilesystem.EntryType.DIRECTORY,
              null,
              sequence,
              "id:" + key,
              "r" + sequence++));
    }

    @Override
    public synchronized void delete(
        String path, boolean directory, IFilesystem.MutationContext mutation) throws Exception {
      requireOpen();
      String key = normal(path);
      Node existing = nodes.get(key);
      if (existing == null) throw missing();
      checkExpected(existing, mutation.expectedRevision());
      if (directory) {
        String prefix = key + "/";
        if (nodes.keySet().stream().anyMatch(value -> value.startsWith(prefix))) {
          throw new SftpFilesystem.ClientFailure(
              "directory-not-empty", "SSH_FX_DIR_NOT_EMPTY", false);
        }
      }
      nodes.remove(key);
    }

    @Override
    public synchronized void move(
        String source,
        String target,
        boolean replace,
        boolean atomic,
        IFilesystem.MutationContext mutation)
        throws Exception {
      requireOpen();
      String sourceKey = normal(source);
      String targetKey = normal(target);
      Node existing = nodes.get(sourceKey);
      if (existing == null) throw missing();
      checkExpected(existing, mutation.expectedRevision());
      Node targetNode = nodes.get(targetKey);
      checkExpected(targetNode, mutation.expectedTargetRevision());
      if (targetNode != null && !replace) {
        throw new SftpFilesystem.ClientFailure("already-exists", "SSH_FX_FILE_ALREADY_EXISTS", false);
      }
      if (atomic && !capabilities.contains(IFilesystem.Capability.ATOMIC_MOVE)) {
        throw new SftpFilesystem.ClientFailure("unsupported", "posix-rename-unavailable", false);
      }
      nodes.remove(sourceKey);
      nodes.put(
          targetKey,
          new Node(
              existing.type(),
              existing.bytes() == null ? null : existing.bytes().clone(),
              sequence,
              "id:" + targetKey,
              "r" + sequence++));
      if (existing.type() == IFilesystem.EntryType.DIRECTORY) {
        String prefix = sourceKey + "/";
        List<String> children =
            nodes.keySet().stream().filter(value -> value.startsWith(prefix)).sorted().toList();
        for (String child : children) {
          Node node = nodes.remove(child);
          nodes.put(targetKey + child.substring(sourceKey.length()), node);
        }
      }
    }

    @Override
    public void close() {
      closed.set(true);
    }

    synchronized void directory(String path) {
      String key = normal(path);
      nodes.put(
          key,
          new Node(
              IFilesystem.EntryType.DIRECTORY,
              null,
              sequence,
              "id:" + key,
              "r" + sequence++));
    }

    synchronized void file(String path, byte[] bytes) {
      String key = normal(path);
      nodes.put(
          key,
          new Node(
              IFilesystem.EntryType.FILE,
              bytes.clone(),
              sequence,
              "id:" + key,
              "r" + sequence++));
    }

    synchronized void symlink(String path) {
      String key = normal(path);
      nodes.put(
          key,
          new Node(
              IFilesystem.EntryType.SYMLINK,
              null,
              sequence,
              "id:" + key,
              "r" + sequence++));
    }

    synchronized byte[] bytes(String path) throws Exception {
      return read(path, Long.MAX_VALUE);
    }

    synchronized boolean exists(String path) {
      return nodes.containsKey(normal(path));
    }

    private SftpFilesystem.RemoteEntry remote(String path, Node node) {
      String key = normal(path);
      int separator = key.lastIndexOf('/');
      String name = separator < 0 ? key : key.substring(separator + 1);
      if (name.isEmpty()) name = "root";
      return new SftpFilesystem.RemoteEntry(
          name,
          node.type(),
          node.type() == IFilesystem.EntryType.FILE ? (long) node.bytes().length : null,
          node.modifiedAt(),
          node.id(),
          node.revision(),
          new IFilesystem.Capabilities(capabilities),
          Map.of("provider/status", "fixture"));
    }

    private void checkExpected(Node node, String expected) throws Exception {
      if (expected == null) return;
      if (node == null || !expected.equals(node.revision())) {
        throw new SftpFilesystem.ClientFailure("conflict", "revision-mismatch", false);
      }
    }

    private void requireOpen() throws Exception {
      if (closed.get()) {
        throw new SftpFilesystem.ClientFailure("provider-closed", "fixture-closed", false);
      }
    }

    private static SftpFilesystem.ClientFailure missing() {
      return new SftpFilesystem.ClientFailure("not-found", "SSH_FX_NO_SUCH_FILE", false);
    }

    private static String normal(String value) {
      if (value.length() > 1 && value.endsWith("/")) return value.substring(0, value.length() - 1);
      return value;
    }
  }
}
