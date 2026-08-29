package hara.truffle;

import java.io.IOException;
import java.net.URI;
import java.nio.channels.SeekableByteChannel;
import java.nio.file.AccessMode;
import java.nio.file.CopyOption;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.NotDirectoryException;
import java.nio.file.OpenOption;
import java.nio.file.Path;
import java.nio.file.attribute.FileAttribute;
import java.util.Map;
import java.util.Set;
import org.graalvm.polyglot.io.FileSystem;

/** Confines a Truffle session's public filesystem to one host directory. */
final class HaraMountedFileSystem implements FileSystem {
  private final Path root;
  private final FileSystem delegate = FileSystem.newDefaultFileSystem();

  HaraMountedFileSystem(Path root) {
    this.root = root.toAbsolutePath().normalize();
  }

  Path root() {
    return root;
  }

  private Path mounted(Path path) throws IOException {
    Path normalized = path.toAbsolutePath().normalize();
    if (normalized.startsWith(root)) return normalized;
    try {
      return HaraLogicalPath.toHost(root, path.toString());
    } catch (HaraLogicalPath.Error error) {
      throw new IOException("file/" + error.code(), error);
    }
  }

  /**
   * Rejects mount escape through an existing ancestor symlink. The final entry is deliberately not
   * followed so metadata and deletion can operate on a symlink itself.
   */
  private Path confined(Path path) throws IOException {
    Path candidate = mounted(path);
    Path rootReal = root.toRealPath();
    Path relative = root.relativize(candidate);
    Path current = root;
    int index = 0;
    int count = relative.getNameCount();
    for (Path component : relative) {
      current = current.resolve(component);
      index++;
      if (index == count) break;
      if (!Files.exists(current, LinkOption.NOFOLLOW_LINKS)) break;
      if (Files.isSymbolicLink(current)) throw new IOException("file/outside-root");
      if (!Files.isDirectory(current, LinkOption.NOFOLLOW_LINKS)) {
        throw new NotDirectoryException(current.toString());
      }
      if (!current.toRealPath().startsWith(rootReal)) throw new IOException("file/outside-root");
    }
    return candidate;
  }

  private static void rejectFinalSymlink(Path path) throws IOException {
    if (Files.isSymbolicLink(path)) throw new IOException("file/unsupported");
  }

  @Override
  public Path parsePath(URI uri) {
    Path path = Path.of(uri);
    try {
      return mounted(path);
    } catch (IOException error) {
      throw new IllegalArgumentException(error.getMessage(), error);
    }
  }

  @Override
  public Path parsePath(String path) {
    try {
      return HaraLogicalPath.toHost(root, path == null ? "" : path);
    } catch (HaraLogicalPath.Error error) {
      throw new IllegalArgumentException("file/" + error.code(), error);
    }
  }

  @Override
  public void checkAccess(Path path, Set<? extends AccessMode> modes, LinkOption... options)
      throws IOException {
    delegate.checkAccess(confined(path), modes, options);
  }

  @Override
  public void createDirectory(Path path, FileAttribute<?>... attributes) throws IOException {
    delegate.createDirectory(confined(path), attributes);
  }

  @Override
  public void delete(Path path) throws IOException {
    delegate.delete(confined(path));
  }

  @Override
  public SeekableByteChannel newByteChannel(
      Path path, Set<? extends OpenOption> options, FileAttribute<?>... attributes)
      throws IOException {
    Path candidate = confined(path);
    rejectFinalSymlink(candidate);
    return delegate.newByteChannel(candidate, options, attributes);
  }

  @Override
  public DirectoryStream<Path> newDirectoryStream(
      Path path, DirectoryStream.Filter<? super Path> filter) throws IOException {
    Path candidate = confined(path);
    rejectFinalSymlink(candidate);
    return delegate.newDirectoryStream(candidate, filter);
  }

  @Override
  public Path toAbsolutePath(Path path) {
    try {
      return mounted(path);
    } catch (IOException error) {
      throw new IllegalArgumentException(error.getMessage(), error);
    }
  }

  @Override
  public Path toRealPath(Path path, LinkOption... options) throws IOException {
    Path real = delegate.toRealPath(mounted(path), options);
    if (!real.startsWith(root.toRealPath())) throw new IOException("file/outside-root");
    return real;
  }

  @Override
  public Map<String, Object> readAttributes(
      Path path, String attributes, LinkOption... options) throws IOException {
    return delegate.readAttributes(confined(path), attributes, options);
  }

  @Override
  public void setAttribute(
      Path path, String attribute, Object value, LinkOption... options) throws IOException {
    Path candidate = confined(path);
    rejectFinalSymlink(candidate);
    delegate.setAttribute(candidate, attribute, value, options);
  }

  @Override
  public void copy(Path source, Path target, CopyOption... options) throws IOException {
    Path sourcePath = confined(source);
    rejectFinalSymlink(sourcePath);
    delegate.copy(sourcePath, confined(target), options);
  }

  @Override
  public void move(Path source, Path target, CopyOption... options) throws IOException {
    Path sourcePath = confined(source);
    rejectFinalSymlink(sourcePath);
    delegate.move(sourcePath, confined(target), options);
  }

  @Override
  public Path getTempDirectory() {
    return root.resolve(".tmp");
  }
}
