package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Test;

public class HaraProjectTest {
  @Test
  public void parsesProjectDescriptorAndResolvesNamespacePaths() throws Exception {
    Path root = Files.createTempDirectory("hara-project");
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project :project/id sample "
            + ":project/source-paths [\"src\"] :project/test-paths [\"test\"] "
            + ":project/extension-paths [\"extensions\" \"target/package\"]}");
    Path source = root.resolve("src/sample/core_name.hal");
    Files.createDirectories(source.getParent());
    Files.writeString(source, "(ns sample.core-name)");

    HaraProject project = HaraProject.discover(source.getParent());
    assertEquals("sample", project.name().display());
    assertEquals(root, project.root());
    assertEquals(source, project.resolve("sample.core-name", false));
    assertEquals(
        java.util.List.of(root.resolve("extensions"), root.resolve("target/package")),
        project.extensionRoots());
  }

  @Test
  public void acceptsStringProjectCoordinatesLikeTheRustRuntime() throws Exception {
    Path root = Files.createTempDirectory("hara-project-coordinate");
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project :project/id \"gh:greenways-ai/hoplite\" "
            + ":project/source-paths [] :project/test-paths [] "
            + ":project/extension-paths []}");

    HaraProject project = HaraProject.discover(root);

    assertEquals("gh:greenways-ai/hoplite", project.name().display());
  }

  @Test
  public void rejectsProjectPathsOutsideTheProjectRoot() throws Exception {
    Path root = Files.createTempDirectory("hara-project-invalid");
    Path descriptor = root.resolve("project.edn");
    Files.writeString(
        descriptor, "{:project/id sample :project/source-paths [\"../outside\"]}");
    HaraException error =
        assertThrows(HaraException.class, () -> HaraProject.read(descriptor));
    assertTrue(error.getMessage().contains("cannot escape"));
  }

  @Test
  public void keepsLegacyProjectHalAsMigrationFallback() throws Exception {
    Path root = Files.createTempDirectory("hara-project-legacy");
    Files.writeString(
        root.resolve("project.hal"),
        "(defproject sample {:source-paths [\"src\"] :test-paths [\"test\"]})");
    HaraProject project = HaraProject.discover(root);
    assertEquals("sample", project.name().display());
  }

  @Test
  public void parsesJvmRuntimeProfileAndMergesEffectivePaths() throws Exception {
    Path root = Files.createTempDirectory("hara-project-jvm");
    Files.writeString(
        root.resolve("project.edn"),
        "{:project/id sample :project/source-paths [\"src\"] :project/test-paths [\"test\"] "
            + ":project/extension-paths [\"extensions\"] :project/capabilities #{:jvm/reflection} "
            + ":project/dependencies {\"hara:hara/base\" {:version \"^1.0.0\"}} "
            + ":project/profiles {:production {:profile/language :hara}} "
            + ":project/runtime-profiles {:jvm {"
            + ":runtime/source-paths [\"src-jvm\"] :runtime/test-paths [\"test-jvm\"] "
            + ":runtime/extension-paths [\"extensions-jvm\"] "
            + ":runtime/native-source-paths [\"java-src\"] "
            + ":runtime/target-path \"build/classes\" "
            + ":runtime/dependencies {:hara {\"hara:hara/jvm\" {:version \"^1.0.0\"}} "
            + ":maven {org.apache.commons/commons-lang3 {:version \"3.12.0\"}}}}}}}");

    HaraProject project = HaraProject.read(root.resolve("project.edn"));

    assertEquals(
        java.util.List.of(root.resolve("src"), root.resolve("src-jvm")),
        project.sourcePaths());
    assertEquals(
        java.util.List.of(root.resolve("test"), root.resolve("test-jvm")),
        project.testPaths());
    assertEquals(
        java.util.List.of(root.resolve("extensions"), root.resolve("extensions-jvm")),
        project.extensionRoots());
    assertEquals(
        java.util.List.of("org.apache.commons:commons-lang3:3.12.0"),
        project.jvmDependencies().stream().map(HaraProject.JvmDependency::coordinate).toList());
    assertEquals(java.util.List.of(root.resolve("java-src")), project.jvmSourcePaths());
    assertEquals(root.resolve("build/classes"), project.jvmTargetPath());
    assertEquals("^1.0.0", project.haraDependencies().get("hara:hara/base"));
    assertEquals("^1.0.0", project.haraDependencies().get("hara:hara/jvm"));
    assertTrue(project.hasCapability("jvm/reflection"));
  }

  @Test
  public void rejectsJvmMavenDependencyRanges() throws Exception {
    Path root = Files.createTempDirectory("hara-project-jvm-invalid");
    Path descriptor = root.resolve("project.edn");
    Files.writeString(
        descriptor,
        "{:project/id sample :project/runtime-profiles {:jvm {:runtime/dependencies "
            + "{:maven {org.example/library {:version \"[1,2)\"}}}}}}}");

    HaraException error =
        assertThrows(HaraException.class, () -> HaraProject.read(descriptor));
    assertTrue(error.getMessage().contains("exact version"));
  }

  @Test
  public void rejectsLegacyJvmKeysWithReplacement() throws Exception {
    Path root = Files.createTempDirectory("hara-project-jvm-legacy");
    Path descriptor = root.resolve("project.edn");
    Files.writeString(
        descriptor,
        "{:project/id sample :jvm/source-paths [\"java-src\"]}");

    HaraException error =
        assertThrows(HaraException.class, () -> HaraProject.read(descriptor));
    assertTrue(
        error
            .getMessage()
            .contains(":project/runtime-profiles :jvm :runtime/native-source-paths"));
  }

  @Test
  public void rejectsConflictingSharedAndJvmHaraRequirements() throws Exception {
    Path root = Files.createTempDirectory("hara-project-jvm-hara-conflict");
    Path descriptor = root.resolve("project.edn");
    Files.writeString(
        descriptor,
        "{:project/id sample "
            + ":project/dependencies {\"hara:hara/crypto\" {:version \"^1.0.0\"}} "
            + ":project/runtime-profiles {:jvm {:runtime/dependencies {:hara "
            + "{\"hara:hara/crypto\" {:version \"^2.0.0\"}}}}}}}");

    HaraException error =
        assertThrows(HaraException.class, () -> HaraProject.read(descriptor));
    assertTrue(error.getMessage().contains("Conflicting Hara dependency requirements"));
  }

  @Test
  public void requiresProjectNamespacesByConvention() throws Exception {
    Path benchmark = Files.createTempDirectory("hara-project-convention");
    Files.writeString(
        benchmark.resolve("project.edn"),
        "{:hara/type :project :project/id fixture :project/source-paths [\"src\"]"
            + " :project/test-paths [\"test\"]}");
    Path source = benchmark.resolve("src/testing/project_fixture.hal");
    Files.createDirectories(source.getParent());
    Files.writeString(source, "(ns testing.project-fixture) (def answer 42)");
    Path testSource = benchmark.resolve("test/testing/project_test_path_test.hal");
    Files.createDirectories(testSource.getParent());
    Files.writeString(
        testSource,
        "(ns testing.project-test-path-test) (def location :test-path)");
    Files.writeString(
        benchmark.resolve("test/testing/project_mismatch_test.hal"),
        "(ns testing.not-project-mismatch-test)");
    try (Context project =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(benchmark)
            .allowIO(IOAccess.ALL)
            .build()) {
      project.eval(HaraLanguage.ID, "(require 'testing.project-fixture)");
      assertEquals(
          42,
          project.eval(HaraLanguage.ID, "testing.project-fixture/answer").asInt());
      project.eval(HaraLanguage.ID, "(require 'testing.project-test-path-test)");
      assertEquals(
          ":test-path",
          project
              .eval(HaraLanguage.ID, "testing.project-test-path-test/location")
              .toString());
      project.eval(
          HaraLanguage.ID,
          "(require 'testing.project-test-path-test {:reload true})");
      assertThrows(
          PolyglotException.class,
          () -> project.eval(HaraLanguage.ID, "(require 'testing.project-mismatch-test)"));
    }
  }

  @Test
  public void projectSourceOverridesPackagedHalResource() throws Exception {
    Path root = Files.createTempDirectory("hara-project-resource-override");
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project :project/id override :project/source-paths [\"src\"]}");
    Path source = root.resolve("src/std/lib/simple.hal");
    Files.createDirectories(source.getParent());
    Files.writeString(source, "(ns std.lib.simple) (defn foo [value] (+ value 40))");

    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(root)
            .allowIO(IOAccess.ALL)
            .build()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(require 'std.lib.simple) (std.lib.simple/foo 2)")
              .asInt());
    }
  }

  @Test
  public void lazyNamespaceStateIsNonForcingAndFailedLoadsRequireExplicitReload()
      throws Exception {
    Path root = Files.createTempDirectory("hara-project-lazy");
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project :project/id sample :project/source-paths [\"src\"]}");
    Path source = root.resolve("src/sample/lazy.hal");
    Files.createDirectories(source.getParent());
    Files.writeString(source, "(ns sample.lazy) (def leaked 1) (throw :broken)");
    String sourcePath = source.toString().replace("\\", "\\\\").replace("\"", "\\\"");

    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(root)
            .allowIO(IOAccess.ALL)
            .build()) {
      assertEquals(
          "nil",
          context
              .eval(HaraLanguage.ID, "(pr-str (resolve 'sample.lazy/leaked))")
              .asString());
      assertEquals(
          ":unknown", context.eval(HaraLanguage.ID, "(ns-state 'sample.lazy)").toString());
      context.eval(
          HaraLanguage.ID,
          "(ns user (:require [sample.lazy :as lazy :lazy true]))");
      assertEquals(
          ":unloaded", context.eval(HaraLanguage.ID, "(ns-state 'sample.lazy)").toString());
      assertEquals(
          ":unloaded",
          context
              .eval(HaraLanguage.ID, "(get (ns-alias-state 'lazy) :state)")
              .toString());

      assertThrows(
          PolyglotException.class, () -> context.eval(HaraLanguage.ID, "lazy/leaked"));
      assertEquals(
          ":failed", context.eval(HaraLanguage.ID, "(ns-state 'sample.lazy)").toString());
      assertEquals(
          ":failed",
          context
              .eval(HaraLanguage.ID, "(get (ns-alias-state 'lazy) :state)")
              .toString());
      assertTrue(
          assertThrows(
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, "lazy/leaked"))
              .getMessage()
              .contains("explicit reload"));

      Files.writeString(
          source,
          "(ns sample.lazy) "
              + "(def observed-state (ns-state 'sample.lazy)) "
              + "(def answer 42)");
      context.eval(HaraLanguage.ID, "(require 'sample.lazy {:reload true})");
      assertEquals(42, context.eval(HaraLanguage.ID, "lazy/answer").asInt());
      assertEquals(
          ":loading", context.eval(HaraLanguage.ID, "lazy/observed-state").toString());
      assertEquals(
          ":loaded", context.eval(HaraLanguage.ID, "(ns-state 'sample.lazy)").toString());
      assertEquals(
          1,
          context
              .eval(HaraLanguage.ID, "(module-revision \"" + sourcePath + "\")")
              .asInt());

      Files.writeString(
          source,
          "(ns sample.lazy) (def answer 99) (def reload-leaked-134 1) (throw :reload-failed)");
      assertThrows(
          PolyglotException.class,
          () ->
              context.eval(
                  HaraLanguage.ID,
                  "(require 'sample.lazy {:reload true})"));
      assertEquals(42, context.eval(HaraLanguage.ID, "lazy/answer").asInt());
      assertEquals(
          1,
          context
              .eval(HaraLanguage.ID, "(module-revision \"" + sourcePath + "\")")
              .asInt());
      assertEquals(
          ":loaded", context.eval(HaraLanguage.ID, "(ns-state 'sample.lazy)").toString());
      assertEquals(
          "nil",
          context
              .eval(HaraLanguage.ID, "(pr-str (resolve 'sample.lazy/reload-leaked-134))")
              .asString());

      context.eval(HaraLanguage.ID, "(ns observer)");
      assertEquals(
          ":loaded",
          context
              .eval(
                  HaraLanguage.ID,
                  "(get (ns-alias-state 'user 'lazy) :state)")
              .toString());
    }

    try (Context isolated =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(root)
            .allowIO(IOAccess.ALL)
            .build()) {
      assertEquals(
          ":unknown", isolated.eval(HaraLanguage.ID, "(ns-state 'sample.lazy)").toString());
    }
  }

  @Test
  public void defprojectIsAnExecutableProjectForm() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "sample",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defproject sample {:source-paths [\"src\"]}) "
                      + "(get project :name)")
              .toString());
    }
  }
}
