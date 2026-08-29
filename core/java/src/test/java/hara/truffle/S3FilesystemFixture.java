package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicBoolean;

final class S3FilesystemFixture implements AutoCloseable {
  static final String BUCKET = "fixture-bucket";
  static final String PREFIX = "tenant/assets/";
  final MemoryClient client = new MemoryClient();

  S3FilesystemFixture() {
    client.object(PREFIX + "README.md", "hello".getBytes(StandardCharsets.UTF_8));
    client.object(PREFIX + "data/a.bin", new byte[] {1});
    client.object(PREFIX + "data/b.bin", new byte[] {2});
  }

  @Override
  public void close() throws Exception {
    client.close();
  }

  static final class MemoryClient implements S3Filesystem.Client {
    private record Value(byte[] bytes, long modifiedAt, String revision) {}

    private final Map<String, Value> values = new HashMap<>();
    private final AtomicBoolean closed = new AtomicBoolean();
    private final Set<IFilesystem.Capability> capabilities =
        new HashSet<>(
            Set.of(
                IFilesystem.Capability.READ,
                IFilesystem.Capability.WRITE,
                IFilesystem.Capability.ENTRIES,
                IFilesystem.Capability.DELETE,
                IFilesystem.Capability.COPY,
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
    public synchronized S3Filesystem.ObjectInfo head(String bucket, String key) throws Exception {
      requireOpen(bucket);
      Value value = values.get(key);
      if (value == null) throw missing();
      return info(key, value);
    }

    @Override
    public synchronized S3Filesystem.ListPage list(
        String bucket, String prefix, String delimiter, String continuationToken, int limit)
        throws Exception {
      requireOpen(bucket);
      ArrayList<S3Filesystem.ObjectInfo> objects = new ArrayList<>();
      HashSet<String> common = new HashSet<>();
      for (Map.Entry<String, Value> entry : values.entrySet()) {
        if (!entry.getKey().startsWith(prefix)) continue;
        String remainder = entry.getKey().substring(prefix.length());
        if (remainder.isEmpty()) continue;
        int slash = remainder.indexOf(delimiter);
        if (slash >= 0) {
          common.add(prefix + remainder.substring(0, slash + delimiter.length()));
        } else {
          objects.add(info(entry.getKey(), entry.getValue()));
        }
      }
      objects.sort(Comparator.comparing(S3Filesystem.ObjectInfo::key));
      ArrayList<String> prefixes = new ArrayList<>(common);
      prefixes.sort(String::compareTo);
      ArrayList<Row> rows = new ArrayList<>();
      for (S3Filesystem.ObjectInfo object : objects) rows.add(new Row(object.key(), object, null));
      for (String commonPrefix : prefixes) rows.add(new Row(commonPrefix, null, commonPrefix));
      rows.sort(Comparator.comparing(Row::sortKey));
      int offset = continuationToken == null ? 0 : Integer.parseInt(continuationToken);
      int end = Math.min(rows.size(), offset + limit);
      ArrayList<S3Filesystem.ObjectInfo> pageObjects = new ArrayList<>();
      ArrayList<String> pagePrefixes = new ArrayList<>();
      for (int index = offset; index < end; index++) {
        Row row = rows.get(index);
        if (row.object() != null) pageObjects.add(row.object());
        else pagePrefixes.add(row.prefix());
      }
      return new S3Filesystem.ListPage(
          pageObjects, pagePrefixes, end < rows.size() ? Integer.toString(end) : null);
    }

    @Override
    public synchronized byte[] read(String bucket, String key, long maxBytes) throws Exception {
      requireOpen(bucket);
      Value value = values.get(key);
      if (value == null) throw missing();
      if (value.bytes().length > maxBytes) {
        throw new S3Filesystem.ClientFailure("quota-exceeded", "fixture-transfer-limit", false);
      }
      return value.bytes().clone();
    }

    @Override
    public synchronized S3Filesystem.ObjectInfo put(
        String bucket,
        String key,
        byte[] bytes,
        boolean createOnly,
        String expectedRevision)
        throws Exception {
      requireOpen(bucket);
      Value existing = values.get(key);
      if (createOnly && existing != null) {
        throw new S3Filesystem.ClientFailure("already-exists", "PreconditionFailed", false);
      }
      checkExpected(existing, expectedRevision);
      Value value = new Value(bytes.clone(), sequence, "r" + sequence++);
      values.put(key, value);
      return info(key, value);
    }

    @Override
    public synchronized void delete(String bucket, String key, String expectedRevision)
        throws Exception {
      requireOpen(bucket);
      Value existing = values.get(key);
      if (existing == null) throw missing();
      checkExpected(existing, expectedRevision);
      values.remove(key);
    }

    @Override
    public synchronized S3Filesystem.ObjectInfo copy(
        String bucket,
        String sourceKey,
        String targetKey,
        boolean replace,
        String expectedSourceRevision,
        String expectedTargetRevision)
        throws Exception {
      requireOpen(bucket);
      Value source = values.get(sourceKey);
      if (source == null) throw missing();
      checkExpected(source, expectedSourceRevision);
      Value target = values.get(targetKey);
      if (!replace && target != null) {
        throw new S3Filesystem.ClientFailure("already-exists", "PreconditionFailed", false);
      }
      checkExpected(target, expectedTargetRevision);
      Value copied = new Value(source.bytes().clone(), sequence, "r" + sequence++);
      values.put(targetKey, copied);
      return info(targetKey, copied);
    }

    @Override
    public void close() {
      closed.set(true);
    }

    synchronized void object(String key, byte[] bytes) {
      values.put(key, new Value(bytes.clone(), sequence, "r" + sequence++));
    }

    synchronized boolean exists(String key) {
      return values.containsKey(key);
    }

    synchronized byte[] bytes(String key) {
      Value value = values.get(key);
      return value == null ? null : value.bytes().clone();
    }

    private S3Filesystem.ObjectInfo info(String key, Value value) {
      return new S3Filesystem.ObjectInfo(
          key,
          value.bytes().length,
          value.modifiedAt(),
          value.revision(),
          Map.of("provider/status", "fixture"));
    }

    private void checkExpected(Value value, String expected) throws Exception {
      if (expected == null) return;
      if (value == null || !expected.equals(value.revision())) {
        throw new S3Filesystem.ClientFailure("conflict", "PreconditionFailed", false);
      }
    }

    private void requireOpen(String bucket) throws Exception {
      if (!BUCKET.equals(bucket)) {
        throw new S3Filesystem.ClientFailure("permission-denied", "NoSuchBucket", false);
      }
      if (closed.get()) {
        throw new S3Filesystem.ClientFailure("provider-closed", "fixture-closed", false);
      }
    }

    private static S3Filesystem.ClientFailure missing() {
      return new S3Filesystem.ClientFailure("not-found", "NoSuchKey", false);
    }

    private record Row(String sortKey, S3Filesystem.ObjectInfo object, String prefix) {}
  }
}
