package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicBoolean;

final class GoogleDriveFilesystemFixture implements AutoCloseable {
  final MemoryClient client = new MemoryClient();
  final String rootId;

  GoogleDriveFilesystemFixture() {
    rootId = client.folder(null, "fixture-root");
    client.file(rootId, "README.md", "hello".getBytes(StandardCharsets.UTF_8));
    String data = client.folder(rootId, "data");
    client.file(data, "b.bin", new byte[] {2});
    client.file(data, "a.bin", new byte[] {1});
    client.workspace(rootId, "notes.gdoc");
    client.shortcut(rootId, "shortcut");
  }

  @Override
  public void close() throws Exception {
    client.close();
  }

  static final class MemoryClient implements GoogleDriveFilesystem.Client {
    private final Map<String, GoogleDriveFilesystem.Item> items = new HashMap<>();
    private final Map<String, byte[]> content = new HashMap<>();
    private final AtomicBoolean closed = new AtomicBoolean();
    private final Set<IFilesystem.Capability> capabilities =
        new HashSet<>(
            Set.of(
                IFilesystem.Capability.READ,
                IFilesystem.Capability.WRITE,
                IFilesystem.Capability.ENTRIES,
                IFilesystem.Capability.MKDIR,
                IFilesystem.Capability.DELETE,
                IFilesystem.Capability.COPY,
                IFilesystem.Capability.MOVE,
                IFilesystem.Capability.REVISION_CHECK));
    boolean authenticated = true;
    int sequence = 1;

    @Override
    public boolean authenticated() {
      return authenticated;
    }

    @Override
    public Set<IFilesystem.Capability> capabilities() {
      return Set.copyOf(capabilities);
    }

    @Override
    public synchronized GoogleDriveFilesystem.Item get(String id) throws Exception {
      requireOpen();
      GoogleDriveFilesystem.Item item = items.get(id);
      if (item == null) throw missing();
      return item;
    }

    @Override
    public synchronized GoogleDriveFilesystem.ItemPage listChildren(
        String parentId, String pageToken, int pageSize) throws Exception {
      requireOpen();
      ArrayList<GoogleDriveFilesystem.Item> children = new ArrayList<>();
      for (GoogleDriveFilesystem.Item item : items.values()) {
        if (parentId.equals(item.parentId())) children.add(item);
      }
      children.sort(
          Comparator.comparing(GoogleDriveFilesystem.Item::name)
              .thenComparing(GoogleDriveFilesystem.Item::id));
      int offset = pageToken == null ? 0 : Integer.parseInt(pageToken);
      int end = Math.min(children.size(), offset + pageSize);
      String next = end < children.size() ? Integer.toString(end) : null;
      return new GoogleDriveFilesystem.ItemPage(children.subList(offset, end), next);
    }

    @Override
    public synchronized byte[] readMedia(String id, long maxBytes) throws Exception {
      requireOpen();
      GoogleDriveFilesystem.Item item = get(id);
      if (item.type() != GoogleDriveFilesystem.ItemType.FILE) {
        throw new GoogleDriveFilesystem.ClientFailure("unsupported", "not-downloadable", false);
      }
      byte[] bytes = content.get(id);
      if (bytes.length > maxBytes) {
        throw new GoogleDriveFilesystem.ClientFailure("quota-exceeded", "transfer-limit", false);
      }
      return bytes.clone();
    }

    @Override
    public synchronized GoogleDriveFilesystem.Item createFile(
        String parentId, String name, byte[] bytes) throws Exception {
      requireOpen();
      get(parentId);
      String id = "file-" + sequence++;
      GoogleDriveFilesystem.Item item =
          item(
              id,
              parentId,
              name,
              GoogleDriveFilesystem.ItemType.FILE,
              (long) bytes.length,
              "application/octet-stream");
      items.put(id, item);
      content.put(id, bytes.clone());
      return item;
    }

    @Override
    public synchronized GoogleDriveFilesystem.Item updateFile(
        String id, byte[] bytes, String expectedRevision) throws Exception {
      requireOpen();
      GoogleDriveFilesystem.Item current = get(id);
      checkExpected(current, expectedRevision);
      GoogleDriveFilesystem.Item updated =
          revise(current, (long) bytes.length, current.parentId(), current.name());
      items.put(id, updated);
      content.put(id, bytes.clone());
      return updated;
    }

    @Override
    public synchronized GoogleDriveFilesystem.Item createFolder(String parentId, String name)
        throws Exception {
      requireOpen();
      get(parentId);
      String id = "folder-" + sequence++;
      GoogleDriveFilesystem.Item item =
          item(id, parentId, name, GoogleDriveFilesystem.ItemType.FOLDER, null, "application/vnd.google-apps.folder");
      items.put(id, item);
      return item;
    }

    @Override
    public synchronized void trash(String id, String expectedRevision) throws Exception {
      requireOpen();
      GoogleDriveFilesystem.Item current = get(id);
      checkExpected(current, expectedRevision);
      removeTree(id);
    }

    @Override
    public synchronized GoogleDriveFilesystem.Item copyFile(
        String sourceId, String parentId, String name, String expectedRevision) throws Exception {
      requireOpen();
      GoogleDriveFilesystem.Item source = get(sourceId);
      checkExpected(source, expectedRevision);
      if (source.type() != GoogleDriveFilesystem.ItemType.FILE) {
        throw new GoogleDriveFilesystem.ClientFailure("unsupported", "copy-type", false);
      }
      return createFile(parentId, name, content.get(sourceId));
    }

    @Override
    public synchronized GoogleDriveFilesystem.Item move(
        String id,
        String oldParentId,
        String newParentId,
        String newName,
        String expectedRevision)
        throws Exception {
      requireOpen();
      GoogleDriveFilesystem.Item current = get(id);
      checkExpected(current, expectedRevision);
      if (!java.util.Objects.equals(oldParentId, current.parentId())) {
        throw new GoogleDriveFilesystem.ClientFailure("conflict", "parent-mismatch", false);
      }
      get(newParentId);
      GoogleDriveFilesystem.Item updated =
          revise(current, current.size(), newParentId, newName);
      items.put(id, updated);
      return updated;
    }

    @Override
    public void close() {
      closed.set(true);
    }

    synchronized String folder(String parentId, String name) {
      String id = "folder-" + sequence++;
      items.put(
          id,
          item(id, parentId, name, GoogleDriveFilesystem.ItemType.FOLDER, null, "application/vnd.google-apps.folder"));
      return id;
    }

    synchronized String file(String parentId, String name, byte[] bytes) {
      String id = "file-" + sequence++;
      items.put(
          id,
          item(
              id,
              parentId,
              name,
              GoogleDriveFilesystem.ItemType.FILE,
              (long) bytes.length,
              "application/octet-stream"));
      content.put(id, bytes.clone());
      return id;
    }

    synchronized String workspace(String parentId, String name) {
      String id = "workspace-" + sequence++;
      items.put(
          id,
          item(
              id,
              parentId,
              name,
              GoogleDriveFilesystem.ItemType.WORKSPACE,
              null,
              "application/vnd.google-apps.document"));
      return id;
    }

    synchronized String shortcut(String parentId, String name) {
      String id = "shortcut-" + sequence++;
      items.put(
          id,
          item(
              id,
              parentId,
              name,
              GoogleDriveFilesystem.ItemType.SHORTCUT,
              null,
              "application/vnd.google-apps.shortcut"));
      return id;
    }

    synchronized boolean exists(String id) {
      return items.containsKey(id);
    }

    synchronized byte[] bytes(String id) {
      byte[] value = content.get(id);
      return value == null ? null : value.clone();
    }

    synchronized GoogleDriveFilesystem.Item byName(String parentId, String name) {
      return items.values().stream()
          .filter(value -> parentId.equals(value.parentId()) && name.equals(value.name()))
          .findFirst()
          .orElse(null);
    }

    private GoogleDriveFilesystem.Item item(
        String id,
        String parentId,
        String name,
        GoogleDriveFilesystem.ItemType type,
        Long size,
        String mime) {
      return new GoogleDriveFilesystem.Item(
          id,
          parentId,
          name,
          type,
          size,
          (long) sequence,
          "r" + sequence++,
          mime,
          new GoogleDriveFilesystem.ItemCapabilities(true, true, true, true, true, true, true),
          Map.of("provider/status", "fixture"));
    }

    private GoogleDriveFilesystem.Item revise(
        GoogleDriveFilesystem.Item current, Long size, String parentId, String name) {
      return new GoogleDriveFilesystem.Item(
          current.id(),
          parentId,
          name,
          current.type(),
          size,
          (long) sequence,
          "r" + sequence++,
          current.mimeType(),
          current.capabilities(),
          current.extensions());
    }

    private void checkExpected(GoogleDriveFilesystem.Item item, String expected) throws Exception {
      if (expected == null) return;
      if (!expected.equals(item.revision())) {
        throw new GoogleDriveFilesystem.ClientFailure("conflict", "revision-mismatch", false);
      }
    }

    private void removeTree(String id) {
      List<String> children =
          items.values().stream()
              .filter(value -> id.equals(value.parentId()))
              .map(GoogleDriveFilesystem.Item::id)
              .toList();
      for (String child : children) removeTree(child);
      items.remove(id);
      content.remove(id);
    }

    private void requireOpen() throws Exception {
      if (closed.get()) {
        throw new GoogleDriveFilesystem.ClientFailure("provider-closed", "fixture-closed", false);
      }
    }

    private static GoogleDriveFilesystem.ClientFailure missing() {
      return new GoogleDriveFilesystem.ClientFailure("not-found", "notFound", false);
    }
  }
}
