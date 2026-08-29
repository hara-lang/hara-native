package hara.truffle;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.util.Comparator;
import java.util.Set;
import java.util.jar.JarEntry;
import java.util.jar.JarOutputStream;
import javax.tools.JavaCompiler;
import javax.tools.ToolProvider;
import org.junit.Test;

public class JvmPackageLoaderTest {
  @Test
  public void loadsAPrebuiltProviderIntoTheKernelAndRemovesItOnClose() throws Exception {
    Path root = Files.createTempDirectory("hara-jvm-package-");
    try {
      Path jar = buildFixture(root, false);
      JvmPackageLoader.Selection selection = selection(jar, digest(jar));
      try (SessionKernel kernel = new SessionKernel(true, false)) {
        kernel.loadJvmProvider(selection);
        SessionModel.SessionMountId mount =
            kernel.createFilesystem("fixture", java.util.Map.of()).toCompletableFuture().join();
        assertTrue(kernel.filesystemInfo(mount).readOnly());
        kernel.closeFilesystem(mount);
        kernel.close();
        assertThrows(IllegalStateException.class, () -> kernel.loadJvmProvider(selection));
      }
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void closeIsIdempotentAndUnregistersFactories() throws Exception {
    Path root = Files.createTempDirectory("hara-jvm-package-");
    try {
      Path jar = buildFixture(root, false);
      FilesystemProviderRegistry registry = new FilesystemProviderRegistry();
      JvmPackageLoader.LoadedProvider loaded =
          JvmPackageLoader.load(selection(jar, digest(jar)), registry);
      assertTrue(registry.contains("fixture"));
      loaded.close();
      loaded.close();
      assertFalse(registry.contains("fixture"));
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void failedRegistrationRollsBackFactoriesAndDigestIsCheckedBeforeLoading() throws Exception {
    Path root = Files.createTempDirectory("hara-jvm-package-");
    try {
      Path failingJar = buildFixture(root.resolve("failing"), true);
      FilesystemProviderRegistry registry = new FilesystemProviderRegistry();
      IllegalArgumentException initialization =
          assertThrows(
              IllegalArgumentException.class,
              () -> JvmPackageLoader.load(selection(failingJar, digest(failingJar)), registry));
      assertTrue(initialization.getMessage().contains("PACKAGE_JVM_INITIALIZATION_FAILED"));
      assertFalse(registry.contains("fixture"));

      Path validJar = buildFixture(root.resolve("valid"), false);
      IllegalArgumentException digestFailure =
          assertThrows(
              IllegalArgumentException.class,
              () ->
                  JvmPackageLoader.load(
                      selection(validJar, "sha256:" + "0".repeat(64)), registry));
      assertTrue(digestFailure.getMessage().contains("PACKAGE_JVM_DIGEST_MISMATCH"));
      assertFalse(registry.contains("fixture"));
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void rejectsMissingEntrypointsAndIncompatibleFlavorBeforeActivation() throws Exception {
    Path root = Files.createTempDirectory("hara-jvm-package-");
    try {
      Path jar = buildFixture(root, false);
      FilesystemProviderRegistry registry = new FilesystemProviderRegistry();

      IllegalArgumentException missingEntrypoint =
          assertThrows(
              IllegalArgumentException.class,
              () ->
                  JvmPackageLoader.load(
                      selection(jar, digest(jar), "fixture.MissingProvider"), registry));
      assertTrue(missingEntrypoint.getMessage().contains("PACKAGE_JVM_INITIALIZATION_FAILED"));
      assertFalse(registry.contains("fixture"));

      IllegalArgumentException targetMismatch =
          assertThrows(
              IllegalArgumentException.class,
              () ->
                  JvmPackageLoader.loadFlavor(
                      new JvmPackageLoader.FlavorSelection(
                          "hara:test-fixture",
                          jar,
                          digest(jar),
                          "java-17",
                          JvmPackageProvider.ABI,
                          "fixture.Provider",
                          java.util.List.of())));
      assertTrue(targetMismatch.getMessage().contains("PACKAGE_JVM_TARGET_MISMATCH"));

      IllegalArgumentException abiMismatch =
          assertThrows(
              IllegalArgumentException.class,
              () ->
                  JvmPackageLoader.loadFlavor(
                      new JvmPackageLoader.FlavorSelection(
                          "hara:test-fixture",
                          jar,
                          digest(jar),
                          "java-21",
                          "hara.provider.jvm.v2",
                          "fixture.Provider",
                          java.util.List.of())));
      assertTrue(abiMismatch.getMessage().contains("PACKAGE_JVM_ABI_MISMATCH"));
    } finally {
      deleteTree(root);
    }
  }

  private static JvmPackageLoader.Selection selection(Path jar, String digest) {
    return selection(jar, digest, "fixture.Provider");
  }

  private static JvmPackageLoader.Selection selection(Path jar, String digest, String entryPoint) {
    return new JvmPackageLoader.Selection(
        "hara:test-fixture", jar, digest, JvmPackageProvider.ABI, entryPoint, Set.of());
  }

  private static Path buildFixture(Path root, boolean failAfterRegistration) throws Exception {
    Files.createDirectories(root);
    Path source = root.resolve("Provider.java");
    Path classes = root.resolve("classes");
    Files.createDirectories(classes);
    Files.writeString(source, fixtureSource(failAfterRegistration));
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    assertNotNull("JDK compiler is required for the prebuilt test fixture", compiler);
    int result =
        compiler.run(
            null,
            null,
            null,
            "-classpath",
            System.getProperty("java.class.path"),
            "-d",
            classes.toString(),
            source.toString());
    assertTrue("fixture provider compilation failed", result == 0);

    Path jar = root.resolve("fixture.jar");
    try (JarOutputStream output =
        new JarOutputStream(
            Files.newOutputStream(
                jar, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING))) {
      Files.walk(classes)
          .filter(path -> path.toString().endsWith(".class"))
          .sorted()
          .forEach(
              path -> {
                String entryName = classes.relativize(path).toString().replace('\\', '/');
                try {
                  output.putNextEntry(new JarEntry(entryName));
                  output.write(Files.readAllBytes(path));
                  output.closeEntry();
                } catch (IOException error) {
                  throw new RuntimeException(error);
                }
              });
    }
    return jar;
  }

  private static String fixtureSource(boolean failAfterRegistration) {
    return """
        package fixture;

        import hara.truffle.IFilesystem;
        import hara.truffle.IFilesystemFactory;
        import hara.truffle.JvmPackageProvider;
        import java.lang.reflect.Proxy;
        import java.util.Map;
        import java.util.Set;
        import java.util.concurrent.CompletableFuture;
        import java.util.concurrent.CompletionStage;

        public final class Provider implements JvmPackageProvider {
          public String identity() { return "hara:test-fixture"; }

          public void register(Registration registration) {
            registration.filesystem(new IFilesystemFactory() {
              public String kind() { return "fixture"; }
              public CompletionStage<IFilesystem> open(
                  OpenContext context, Map<String, ?> configuration) {
                IFilesystem filesystem = (IFilesystem) Proxy.newProxyInstance(
                    IFilesystem.class.getClassLoader(),
                    new Class<?>[] {IFilesystem.class},
                    (proxy, method, arguments) -> {
                      if (method.getName().equals("descriptor")) {
                        return new IFilesystem.Descriptor(
                            "fixture", "prebuilt fixture", true,
                            new IFilesystem.Capabilities(Set.of(IFilesystem.Capability.READ)),
                            null, Map.of());
                      }
                      if (CompletionStage.class.isAssignableFrom(method.getReturnType())) {
                        return CompletableFuture.completedFuture(null);
                      }
                      return null;
                    });
                return CompletableFuture.completedFuture(filesystem);
              }
            });
            %s
          }
        }
        """
        .formatted(failAfterRegistration ? "throw new RuntimeException(\"fixture failure\");" : "");
  }

  private static String digest(Path path) throws Exception {
    return "sha256:"
        + java.util.HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(path)));
  }

  private static void deleteTree(Path root) throws IOException {
    if (!Files.exists(root)) return;
    Files.walk(root)
        .sorted(Comparator.reverseOrder())
        .forEach(
            path -> {
              try {
                Files.deleteIfExists(path);
              } catch (IOException error) {
                throw new RuntimeException(error);
              }
            });
  }
}
