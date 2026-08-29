package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;
import java.util.stream.Stream;
import org.junit.Test;

public class HaraPackageToolTest {
  @Test
  public void localBuildAndInspectAreDeterministic() throws Exception {
    Path root = Files.createTempDirectory("hara-package-tool-");
    try {
      Files.createDirectories(root.resolve("src/demo"));
      Files.writeString(
          root.resolve("project.edn"),
          "{:hara/type :project :hara/version \"1.0.0\" "
              + ":project/id demo/app :project/version \"1.2.3\" "
              + ":project/source-paths [\"src\"] :project/test-paths [] "
              + ":project/extension-paths [\"extensions\"] "
              + ":project/artifact-paths [\"artifacts\"] "
              + ":project/extensions {demo.native {:provider :wasm :abi :core.v1 "
              + ":module \"artifacts/demo.wasm\" :exports {} :capabilities []}} "
              + ":project/capabilities #{}}\n");
      Files.createDirectories(root.resolve("artifacts"));
      Files.write(root.resolve("artifacts/demo.wasm"), new byte[] {0, 97, 115, 109});
      Files.writeString(root.resolve("src/demo/main.hal"), "(ns demo.main)\n(def answer 42)\n");
      Path first = root.resolve("first.harp");
      Path second = root.resolve("second.harp");
      ByteArrayOutputStream output = new ByteArrayOutputStream();
      ByteArrayOutputStream error = new ByteArrayOutputStream();
      PrintStream stdout = new PrintStream(output, true, StandardCharsets.UTF_8);
      PrintStream stderr = new PrintStream(error, true, StandardCharsets.UTF_8);
      assertEquals(
          0,
          HaraPackageTool.run(
              new String[] {"build", root.toString(), "--output", first.toString()},
              stdout,
              stderr));
      assertEquals(
          0,
          HaraPackageTool.run(
              new String[] {"build", root.toString(), "--output", second.toString()},
              stdout,
              stderr));
      assertArrayEquals(Files.readAllBytes(first), Files.readAllBytes(second));
      output.reset();
      assertEquals(
          0,
          HaraPackageTool.run(
              new String[] {"inspect", first.toString()}, stdout, stderr));
      String manifest = output.toString(StandardCharsets.UTF_8);
      assertTrue(manifest.contains(":identity \"demo/app\""));
      assertTrue(manifest.contains("\"demo.main\" \"src/demo/main.hal\""));
      assertTrue(manifest.contains(":extensions {demo.native"));
      assertEquals("", error.toString(StandardCharsets.UTF_8));
    } finally {
      Files.walk(root)
          .sorted(Comparator.reverseOrder())
          .forEach(
              path -> {
                try {
                  Files.deleteIfExists(path);
                } catch (Exception ignored) {
                }
              });
    }
  }

  @Test
  public void buildsAndInstallsTheDeclaredJvmFlavorAsPrebuiltBytes() throws Exception {
    Path root = Files.createTempDirectory("hara-package-jvm-");
    Path dist = Files.createTempDirectory("hara-package-dist-");
    String previousDist = System.getProperty("hara.dist.home");
    try {
      Files.createDirectories(root.resolve("java-src/fixture"));
      Files.writeString(
          root.resolve("project.edn"),
          "{:hara/type :project :hara/version \"1.0.0\" "
              + ":project/id demo/jvm :project/version \"1.2.3\" "
              + ":project/source-paths [] :project/test-paths [] "
              + ":project/extension-paths [] :project/capabilities #{} "
              + ":project/package {:entry-points [fixture.Provider]} "
              + ":project/runtime-profiles {:jvm {:runtime/native-source-paths [\"java-src\"] "
              + ":runtime/target-path \"target/jvm/classes\"}}}");
      Files.writeString(
          root.resolve("java-src/fixture/Provider.java"),
          "package fixture; "
              + "public final class Provider implements hara.truffle.JvmPackageProvider { "
              + "public String identity() { return \"demo/jvm\"; } "
              + "public void register(Registration registration) {} }");
      Path archive = root.resolve("demo-jvm.harp");
      ByteArrayOutputStream output = new ByteArrayOutputStream();
      ByteArrayOutputStream error = new ByteArrayOutputStream();
      PrintStream stdout = new PrintStream(output, true, StandardCharsets.UTF_8);
      PrintStream stderr = new PrintStream(error, true, StandardCharsets.UTF_8);

      assertEquals(
          0,
          HaraPackageTool.run(
              new String[] {"build", root.toString(), "--output", archive.toString()},
              stdout,
              stderr));
      try (ZipFile zip = new ZipFile(archive.toFile(), StandardCharsets.UTF_8)) {
        assertTrue(zip.getEntry("project.lock.edn") != null);
        assertTrue(zip.getEntry("artifacts/jvm/provider.jar") != null);
        String manifest =
            new String(zip.getInputStream(zip.getEntry("package.edn")).readAllBytes(), StandardCharsets.UTF_8);
        HaraPackageManifest parsed = HaraPackageManifest.parse(manifest, "test archive");
        assertEquals("fixture.Provider", parsed.jvmFlavor().entryPoint());
        assertEquals("artifacts/jvm/provider.jar", parsed.jvmFlavor().path());
      }

      System.setProperty("hara.dist.home", dist.toString());
      assertEquals(0, HaraPackageTool.run(new String[] {"install", archive.toString()}, stdout, stderr));
      assertTrue(Files.isRegularFile(dist.resolve("roots/sha256").resolve(archiveDigest(archive)).resolve("package.edn")));
      assertEquals("", error.toString(StandardCharsets.UTF_8));
    } finally {
      if (previousDist == null) System.clearProperty("hara.dist.home");
      else System.setProperty("hara.dist.home", previousDist);
      deleteTree(dist);
      deleteTree(root);
    }
  }

  @Test
  public void rejectsModifiedAndUndeclaredArchivesBeforeCachingThem() throws Exception {
    Path root = Files.createTempDirectory("hara-native-package-integrity-");
    Path dist = Files.createTempDirectory("hara-native-package-integrity-dist-");
    String previousDist = System.getProperty("hara.dist.home");
    try {
      Files.createDirectories(root.resolve("src/demo"));
      Files.writeString(
          root.resolve("project.edn"),
          "{:hara/type :project :hara/version \"1.0.0\" "
              + ":project/id demo/integrity :project/version \"1.0.0\" "
              + ":project/source-paths [\"src\"] :project/test-paths [] "
              + ":project/extension-paths [] :project/capabilities #{}}\n");
      Files.writeString(root.resolve("src/demo/main.hal"), "(ns demo.main)\n(def answer 42)\n");
      Path archive = root.resolve("valid.harp");
      ByteArrayOutputStream output = new ByteArrayOutputStream();
      ByteArrayOutputStream error = new ByteArrayOutputStream();
      PrintStream stdout = new PrintStream(output, true, StandardCharsets.UTF_8);
      PrintStream stderr = new PrintStream(error, true, StandardCharsets.UTF_8);
      assertEquals(
          0,
          HaraPackageTool.run(
              new String[] {"build", root.toString(), "--output", archive.toString()}, stdout, stderr));

      Path modified = root.resolve("modified.harp");
      rewriteArchive(archive, modified, "src/demo/main.hal", "(ns demo.main)\n(def answer 99)\n".getBytes(StandardCharsets.UTF_8), null);
      Path undeclared = root.resolve("undeclared.harp");
      rewriteArchive(archive, undeclared, null, null, "hidden/payload.bin");

      System.setProperty("hara.dist.home", dist.toString());
      assertEquals(1, HaraPackageTool.run(new String[] {"install", modified.toString()}, stdout, stderr));
      assertEquals(1, HaraPackageTool.run(new String[] {"install", undeclared.toString()}, stdout, stderr));
      assertTrue(error.toString(StandardCharsets.UTF_8).contains("package/"));
      assertTrue(isEmptyDirectory(dist.resolve("archives/sha256")));
      assertTrue(isEmptyDirectory(dist.resolve("roots/sha256")));
    } finally {
      if (previousDist == null) System.clearProperty("hara.dist.home");
      else System.setProperty("hara.dist.home", previousDist);
      deleteTree(dist);
      deleteTree(root);
    }
  }

  private static void rewriteArchive(
      Path source, Path destination, String replacementEntry, byte[] replacement, String extraEntry)
      throws Exception {
    try (ZipFile input = new ZipFile(source.toFile(), StandardCharsets.UTF_8);
        ZipOutputStream output = new ZipOutputStream(Files.newOutputStream(destination), StandardCharsets.UTF_8)) {
      Enumeration<? extends ZipEntry> entries = input.entries();
      while (entries.hasMoreElements()) {
        ZipEntry entry = entries.nextElement();
        ZipEntry copied = new ZipEntry(entry.getName());
        copied.setTime(0L);
        output.putNextEntry(copied);
        byte[] bytes = input.getInputStream(entry).readAllBytes();
        output.write(entry.getName().equals(replacementEntry) ? replacement : bytes);
        output.closeEntry();
      }
      if (extraEntry != null) {
        ZipEntry extra = new ZipEntry(extraEntry);
        extra.setTime(0L);
        output.putNextEntry(extra);
        output.write(new byte[] {1, 2, 3});
        output.closeEntry();
      }
    }
  }

  private static String archiveDigest(Path archive) throws Exception {
    return java.util.HexFormat.of()
        .formatHex(
            java.security.MessageDigest.getInstance("SHA-256")
                .digest(Files.readAllBytes(archive)));
  }

  private static boolean isEmptyDirectory(Path directory) throws Exception {
    if (!Files.exists(directory)) return true;
    try (Stream<Path> entries = Files.list(directory)) {
      return entries.findAny().isEmpty();
    }
  }

  private static void deleteTree(Path root) throws Exception {
    if (!Files.exists(root)) return;
    Files.walk(root)
        .sorted(Comparator.reverseOrder())
        .forEach(
            path -> {
              try {
                Files.deleteIfExists(path);
              } catch (Exception error) {
                throw new RuntimeException(error);
              }
            });
  }
}
