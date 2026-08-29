package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.util.List;
import org.junit.Assume;
import org.junit.Test;

public class HaraFileProviderTest {
  @Test
  public void honoursSafeWriteMetadataAndMutationContracts() throws Exception {
    Path root = Files.createTempDirectory("hara-file-provider-");
    try {
      HaraFileProvider provider = new HaraFileProvider(root);
      assertEquals(
          "/work",
          provider.mkdir("/work", new HaraFileProvider.MkdirOptions(true, true)));
      assertEquals(
          "/work/b.bin",
          provider.write(
              "/work/b.bin",
              new byte[] {2},
              new HaraFileProvider.WriteOptions(HaraFileProvider.WriteMode.CREATE, false)));
      assertEquals(
          "/work/a.bin",
          provider.write(
              "/work/a.bin",
              new byte[] {1},
              new HaraFileProvider.WriteOptions(HaraFileProvider.WriteMode.CREATE, false)));

      HaraFileProvider.Failure exists =
          assertThrows(
              HaraFileProvider.Failure.class,
              () ->
                  provider.write(
                      "/work/a.bin",
                      new byte[] {9},
                      new HaraFileProvider.WriteOptions(
                          HaraFileProvider.WriteMode.CREATE, false)));
      assertEquals("already-exists", exists.code());

      provider.write(
          "/work/a.bin",
          new byte[] {3},
          new HaraFileProvider.WriteOptions(HaraFileProvider.WriteMode.APPEND, false));
      assertArrayEquals(new byte[] {1, 3}, provider.read("/work/a.bin"));

      List<HaraFileProvider.Entry> entries = provider.entries("/work");
      assertEquals(List.of("/work/a.bin", "/work/b.bin"), entries.stream().map(HaraFileProvider.Entry::path).toList());
      HaraFileProvider.Entry first = entries.get(0);
      assertEquals("a.bin", first.name());
      assertEquals("file", first.type());
      assertEquals(Long.valueOf(2), first.size());
      assertTrue(first.modifiedAt() > 0);

      assertEquals(
          "/work/copied.bin",
          provider.copy(
              "/work/a.bin",
              "/work/copied.bin",
              new HaraFileProvider.CopyOptions(false, false, true)));
      assertEquals(
          "/work/moved.bin",
          provider.move(
              "/work/copied.bin",
              "/work/moved.bin",
              new HaraFileProvider.MoveOptions(false, false, false)));
      assertFalse(provider.exists("/work/copied.bin"));
      assertTrue(provider.exists("/work/moved.bin"));

      assertEquals(
          "/work/missing-ok",
          provider.delete("/work/missing-ok", new HaraFileProvider.DeleteOptions(true)));
      assertEquals(
          "not-found",
          assertThrows(
                  HaraFileProvider.Failure.class,
                  () ->
                      provider.delete(
                          "/work/missing", new HaraFileProvider.DeleteOptions(false)))
              .code());

      String temporaryFile =
          provider.tempFile("/work", new HaraFileProvider.TempFileOptions("case", ".tmp"));
      String temporaryDirectory =
          provider.tempDirectory("/work", new HaraFileProvider.TempDirectoryOptions("case"));
      assertNotEquals(temporaryFile, temporaryDirectory);
      assertTrue(provider.exists(temporaryFile));
      assertEquals("directory", provider.stat(temporaryDirectory).type());

      provider.mkdir("/work/non-empty", new HaraFileProvider.MkdirOptions(false, false));
      provider.write(
          "/work/non-empty/item",
          new byte[] {1},
          new HaraFileProvider.WriteOptions(HaraFileProvider.WriteMode.CREATE, false));
      assertEquals(
          "directory-not-empty",
          assertThrows(
                  HaraFileProvider.Failure.class,
                  () ->
                      provider.delete(
                          "/work/non-empty", new HaraFileProvider.DeleteOptions(false)))
              .code());
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void copyReplacementNeverFollowsTheTargetSymlink() throws Exception {
    Path root = Files.createTempDirectory("hara-file-symlink-root-");
    Path outside = Files.createTempDirectory("hara-file-symlink-outside-");
    try {
      Path target = root.resolve("target.bin");
      try {
        Files.createSymbolicLink(target, outside.resolve("outside.bin"));
      } catch (UnsupportedOperationException | IOException | SecurityException error) {
        Assume.assumeNoException(error);
      }
      Files.write(root.resolve("source.bin"), new byte[] {7, 8});
      HaraFileProvider provider = new HaraFileProvider(root);
      provider.copy(
          "/source.bin",
          "/target.bin",
          new HaraFileProvider.CopyOptions(true, false, false));
      assertFalse(Files.isSymbolicLink(target));
      assertArrayEquals(new byte[] {7, 8}, Files.readAllBytes(target));
      assertFalse(Files.exists(outside.resolve("outside.bin"), LinkOption.NOFOLLOW_LINKS));
    } finally {
      deleteTree(root);
      deleteTree(outside);
    }
  }

  @Test
  public void rejectsMovingDirectoriesIntoTheirDescendants() throws Exception {
    Path root = Files.createTempDirectory("hara-file-move-");
    try {
      HaraFileProvider provider = new HaraFileProvider(root);
      provider.mkdir("/source/child", new HaraFileProvider.MkdirOptions(true, true));
      assertEquals(
          "invalid-path",
          assertThrows(
                  HaraFileProvider.Failure.class,
                  () ->
                      provider.move(
                          "/source",
                          "/source/child/moved",
                          new HaraFileProvider.MoveOptions(false, false, false)))
              .code());
    } finally {
      deleteTree(root);
    }
  }

  private static void deleteTree(Path root) throws IOException {
    if (root == null || !Files.exists(root, LinkOption.NOFOLLOW_LINKS)) return;
    try (var paths = Files.walk(root)) {
      for (Path path : paths.sorted(java.util.Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }
}
