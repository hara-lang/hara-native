package hara.truffle;

import hara.lang.data.Keyword;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.AccessDeniedException;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.DirectoryNotEmptyException;
import java.nio.file.FileAlreadyExistsException;
import java.nio.file.FileSystemException;
import java.nio.file.Files;
import java.nio.file.InvalidPathException;
import java.nio.file.LinkOption;
import java.nio.file.NoSuchFileException;
import java.nio.file.NotDirectoryException;
import java.nio.file.OpenOption;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;

/** Synchronous JVM provider hidden behind the promise-returning std.native.File boundary. */
final class HaraFileProvider {
  enum WriteMode {
    CREATE,
    REPLACE,
    APPEND
  }

  record WriteOptions(WriteMode mode, boolean parents) {}

  record MkdirOptions(boolean parents, boolean existsOk) {}

  record DeleteOptions(boolean missingOk) {}

  record CopyOptions(boolean replace, boolean parents, boolean preserveModified) {}

  record MoveOptions(boolean replace, boolean parents, boolean atomic) {}

  record TempFileOptions(String prefix, String suffix) {}

  record TempDirectoryOptions(String prefix) {}

  static final class Failure extends IOException {
    private final String code;

    Failure(String code, String message) {
      super(message);
      this.code = code;
    }

    Failure(String code, String message, Throwable cause) {
      super(message, cause);
      this.code = code;
    }

    String code() {
      return code;
    }
  }

  record Entry(String path, String name, String type, Long size, long modifiedAt) {
    Object toValue() {
      return hara.lang.data.Map.Standard.from(
          null,
          Keyword.create("path"),
          path,
          Keyword.create("name"),
          name,
          Keyword.create("type"),
          Keyword.create(type),
          Keyword.create("size"),
          size,
          Keyword.create("modified-at"),
          modifiedAt,
          Keyword.create("extensions"),
          hara.lang.data.Map.Standard.from(null));
    }
  }

  private static final AtomicLong TEMP_SEQUENCE = new AtomicLong(1);
  private final Path root;

  HaraFileProvider(Path root) {
    Path normalized = root.toAbsolutePath().normalize();
    Path resolved;
    try {
      resolved = normalized.toRealPath();
    } catch (IOException ignored) {
      resolved = normalized;
    }
    this.root = resolved;
  }

  byte[] read(String path) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    Path host = scoped(logical);
    BasicFileAttributes attributes = attributes(host);
    if (attributes.isSymbolicLink()) throw failure("unsupported", "cannot read a symbolic link");
    if (attributes.isDirectory()) throw failure("is-directory", "path is a directory");
    if (!attributes.isRegularFile()) throw failure("unsupported", "path is not a regular file");
    try {
      return Files.readAllBytes(host);
    } catch (Throwable error) {
      throw map(error);
    }
  }

  String write(String path, byte[] bytes, WriteOptions options) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    Path host = scoped(logical);
    ensureParent(host, options.parents());
    if (Files.exists(host, LinkOption.NOFOLLOW_LINKS)) {
      BasicFileAttributes attributes = attributes(host);
      if (attributes.isSymbolicLink()) {
        throw failure("unsupported", "cannot write through a symbolic link");
      }
      if (attributes.isDirectory()) throw failure("is-directory", "path is a directory");
    }
    ArrayList<OpenOption> open = new ArrayList<>();
    open.add(StandardOpenOption.WRITE);
    switch (options.mode()) {
      case CREATE -> open.add(StandardOpenOption.CREATE_NEW);
      case REPLACE -> {
        open.add(StandardOpenOption.CREATE);
        open.add(StandardOpenOption.TRUNCATE_EXISTING);
      }
      case APPEND -> {
        open.add(StandardOpenOption.CREATE);
        open.add(StandardOpenOption.APPEND);
      }
    }
    try (OutputStream output = Files.newOutputStream(host, open.toArray(OpenOption[]::new))) {
      output.write(bytes);
      return logical;
    } catch (Throwable error) {
      throw map(error);
    }
  }

  boolean exists(String path) throws IOException {
    Path host = scoped(HaraLogicalPath.normalise(path));
    try {
      Files.readAttributes(host, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
      return true;
    } catch (NoSuchFileException error) {
      return false;
    } catch (Throwable error) {
      throw map(error);
    }
  }

  Entry stat(String path) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    return entry(logical, scoped(logical));
  }

  List<Entry> entries(String path) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    Path host = scoped(logical);
    BasicFileAttributes attributes = attributes(host);
    if (attributes.isSymbolicLink() || !attributes.isDirectory()) {
      throw failure("not-directory", "path is not a directory");
    }
    ArrayList<Entry> output = new ArrayList<>();
    try (var stream = Files.newDirectoryStream(host)) {
      for (Path child : stream) {
        String name = child.getFileName().toString();
        String childLogical = HaraLogicalPath.join(logical, name);
        output.add(entry(childLogical, child));
      }
    } catch (Throwable error) {
      throw map(error);
    }
    output.sort(Comparator.comparing(Entry::path));
    return output;
  }

  List<String> list(String path) throws IOException {
    return entries(path).stream().map(Entry::path).toList();
  }

  List<String> walk(String path) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    ArrayList<String> output = new ArrayList<>();
    collect(logical, output);
    output.sort(String::compareTo);
    return output;
  }

  String mkdir(String path, MkdirOptions options) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    Path host = scoped(logical);
    if (Files.exists(host, LinkOption.NOFOLLOW_LINKS)) {
      BasicFileAttributes attributes = attributes(host);
      if (attributes.isDirectory() && !attributes.isSymbolicLink() && options.existsOk()) {
        return logical;
      }
      throw failure("already-exists", "path already exists");
    }
    try {
      if (options.parents()) {
        Files.createDirectories(host);
      } else {
        ensureParent(host, false);
        Files.createDirectory(host);
      }
      return logical;
    } catch (Throwable error) {
      throw map(error);
    }
  }

  String delete(String path, DeleteOptions options) throws IOException {
    String logical = HaraLogicalPath.normalise(path);
    if ("/".equals(logical)) throw failure("denied", "cannot delete the mounted root");
    Path host = scoped(logical);
    try {
      Files.delete(host);
      return logical;
    } catch (NoSuchFileException error) {
      if (options.missingOk()) return logical;
      throw map(error);
    } catch (Throwable error) {
      throw map(error);
    }
  }

  String copy(String source, String target, CopyOptions options) throws IOException {
    String sourceLogical = HaraLogicalPath.normalise(source);
    String targetLogical = HaraLogicalPath.normalise(target);
    if (sourceLogical.equals(targetLogical)) {
      throw failure("already-exists", "source and target are the same path");
    }
    Path sourceHost = scoped(sourceLogical);
    Path targetHost = scoped(targetLogical);
    BasicFileAttributes sourceAttributes = attributes(sourceHost);
    if (sourceAttributes.isSymbolicLink()) {
      throw failure("unsupported", "cannot copy a symbolic link");
    }
    if (sourceAttributes.isDirectory()) throw failure("is-directory", "source is a directory");
    if (!sourceAttributes.isRegularFile()) {
      throw failure("unsupported", "source is not a regular file");
    }
    ensureParent(targetHost, options.parents());
    if (Files.exists(targetHost, LinkOption.NOFOLLOW_LINKS)) {
      if (!options.replace()) throw failure("already-exists", "target already exists");
      BasicFileAttributes targetAttributes = attributes(targetHost);
      if (targetAttributes.isDirectory() && !targetAttributes.isSymbolicLink()) {
        throw failure("is-directory", "target is a directory");
      }
      try {
        Files.delete(targetHost);
      } catch (Throwable error) {
        throw map(error);
      }
    }
    try {
      Files.copy(sourceHost, targetHost);
      if (options.preserveModified()) {
        Files.setLastModifiedTime(targetHost, sourceAttributes.lastModifiedTime());
      }
      return targetLogical;
    } catch (Throwable error) {
      throw map(error);
    }
  }

  String move(String source, String target, MoveOptions options) throws IOException {
    String sourceLogical = HaraLogicalPath.normalise(source);
    String targetLogical = HaraLogicalPath.normalise(target);
    if ("/".equals(sourceLogical) || "/".equals(targetLogical)) {
      throw failure("denied", "cannot move the mounted root");
    }
    if (sourceLogical.equals(targetLogical)) {
      stat(sourceLogical);
      return targetLogical;
    }
    if (targetLogical.startsWith(sourceLogical + "/")) {
      throw failure("invalid-path", "cannot move a directory beneath itself");
    }
    Path sourceHost = scoped(sourceLogical);
    Path targetHost = scoped(targetLogical);
    BasicFileAttributes sourceAttributes = attributes(sourceHost);
    if (sourceAttributes.isSymbolicLink()) {
      throw failure("unsupported", "cannot move a symbolic link");
    }
    ensureParent(targetHost, options.parents());
    if (Files.exists(targetHost, LinkOption.NOFOLLOW_LINKS) && !options.replace()) {
      throw failure("already-exists", "target already exists");
    }
    ArrayList<java.nio.file.CopyOption> moveOptions = new ArrayList<>();
    if (options.replace()) moveOptions.add(StandardCopyOption.REPLACE_EXISTING);
    if (options.atomic()) moveOptions.add(StandardCopyOption.ATOMIC_MOVE);
    try {
      Files.move(sourceHost, targetHost, moveOptions.toArray(java.nio.file.CopyOption[]::new));
      return targetLogical;
    } catch (AtomicMoveNotSupportedException error) {
      throw failure("unsupported", "atomic move is unavailable", error);
    } catch (Throwable error) {
      throw map(error);
    }
  }

  String tempFile(String parent, TempFileOptions options) throws IOException {
    HaraLogicalPath.validateFragment(options.prefix(), "temporary file prefix");
    HaraLogicalPath.validateFragment(options.suffix(), "temporary file suffix");
    return temporary(parent, options.prefix(), options.suffix(), false);
  }

  String tempDirectory(String parent, TempDirectoryOptions options) throws IOException {
    HaraLogicalPath.validateFragment(options.prefix(), "temporary directory prefix");
    return temporary(parent, options.prefix(), "", true);
  }

  private String temporary(String parent, String prefix, String suffix, boolean directory)
      throws IOException {
    String parentLogical = HaraLogicalPath.normalise(parent);
    Entry parentEntry = stat(parentLogical);
    if (!"directory".equals(parentEntry.type())) {
      throw failure("not-directory", "temporary parent is not a directory");
    }
    for (int attempt = 0; attempt < 1024; attempt++) {
      String name = prefix + "-" + String.format("%016x", TEMP_SEQUENCE.getAndIncrement()) + suffix;
      String logical = HaraLogicalPath.join(parentLogical, name);
      Path host = scoped(logical);
      try {
        if (directory) Files.createDirectory(host);
        else Files.createFile(host);
        return logical;
      } catch (FileAlreadyExistsException ignored) {
        // Retry with the next sequence value.
      } catch (Throwable error) {
        throw map(error);
      }
    }
    throw failure("io", "unable to allocate a unique temporary entry");
  }

  private void collect(String logical, ArrayList<String> output) throws IOException {
    Entry current = stat(logical);
    if (!"directory".equals(current.type())) {
      output.add(current.path());
      return;
    }
    for (Entry child : entries(logical)) {
      if ("directory".equals(child.type())) collect(child.path(), output);
      else output.add(child.path());
    }
  }

  private Entry entry(String logical, Path host) throws IOException {
    BasicFileAttributes attributes = attributes(host);
    String type =
        attributes.isSymbolicLink()
            ? "symlink"
            : attributes.isRegularFile()
                ? "file"
                : attributes.isDirectory() ? "directory" : "other";
    Long size = attributes.isRegularFile() ? attributes.size() : null;
    return new Entry(
        HaraLogicalPath.normalise(logical),
        HaraLogicalPath.fileName(logical),
        type,
        size,
        attributes.lastModifiedTime().toMillis());
  }

  private BasicFileAttributes attributes(Path host) throws IOException {
    try {
      return Files.readAttributes(host, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
    } catch (Throwable error) {
      throw map(error);
    }
  }

  private Path scoped(String logical) throws IOException {
    final Path host;
    try {
      host = HaraLogicalPath.toHost(root, logical);
    } catch (HaraLogicalPath.Error error) {
      throw failure(error.code(), error.getMessage(), error);
    }
    Path relative = root.relativize(host);
    Path current = root;
    int index = 0;
    int count = relative.getNameCount();
    for (Path component : relative) {
      current = current.resolve(component);
      index++;
      if (index == count) break;
      if (!Files.exists(current, LinkOption.NOFOLLOW_LINKS)) break;
      if (Files.isSymbolicLink(current)) {
        throw failure("outside-root", "path traverses a symbolic link outside the mount");
      }
      if (!Files.isDirectory(current, LinkOption.NOFOLLOW_LINKS)) {
        throw failure("not-directory", "path ancestor is not a directory");
      }
    }
    return host;
  }

  private void ensureParent(Path host, boolean parents) throws IOException {
    Path parent = host.getParent();
    if (parent == null || !parent.startsWith(root)) {
      throw failure("outside-root", "path parent is outside the mount");
    }
    if (parents) {
      try {
        Files.createDirectories(parent);
      } catch (Throwable error) {
        throw map(error);
      }
      return;
    }
    BasicFileAttributes attributes = attributes(parent);
    if (attributes.isSymbolicLink() || !attributes.isDirectory()) {
      throw failure("not-directory", "path parent is not a directory");
    }
  }

  static String code(Throwable error) {
    Throwable current = unwrap(error);
    if (current instanceof Failure failure) return failure.code();
    if (current instanceof HaraLogicalPath.Error pathError) return pathError.code();
    if (current instanceof NoSuchFileException) return "not-found";
    if (current instanceof FileAlreadyExistsException) return "already-exists";
    if (current instanceof InvalidPathException) return "invalid-path";
    if (current instanceof NotDirectoryException) return "not-directory";
    if (current instanceof DirectoryNotEmptyException) return "directory-not-empty";
    if (current instanceof AccessDeniedException || current instanceof SecurityException) {
      return "permission-denied";
    }
    if (current instanceof AtomicMoveNotSupportedException
        || current instanceof UnsupportedOperationException) return "unsupported";
    if (current instanceof FileSystemException filesystem) {
      String reason = filesystem.getReason();
      if (reason != null) {
        String lower = reason.toLowerCase(java.util.Locale.ROOT);
        if (lower.contains("is a directory")) return "is-directory";
        if (lower.contains("not a directory")) return "not-directory";
        if (lower.contains("directory not empty")) return "directory-not-empty";
      }
    }
    return "io";
  }

  static Throwable unwrap(Throwable error) {
    Throwable current = error;
    while ((current instanceof java.util.concurrent.CompletionException
            || current instanceof java.util.concurrent.ExecutionException)
        && current.getCause() != null) {
      current = current.getCause();
    }
    return current;
  }

  private static Failure map(Throwable error) {
    Throwable current = unwrap(error);
    if (current instanceof Failure failure) return failure;
    if (current instanceof HaraLogicalPath.Error pathError) {
      return failure(pathError.code(), pathError.getMessage(), pathError);
    }
    return failure(code(current), message(current), current);
  }

  private static String message(Throwable error) {
    String message = error.getMessage();
    return message == null || message.isBlank() ? error.getClass().getSimpleName() : message;
  }

  private static Failure failure(String code, String message) {
    return new Failure(code, message);
  }

  private static Failure failure(String code, String message, Throwable cause) {
    return new Failure(code, message, cause);
  }
}
