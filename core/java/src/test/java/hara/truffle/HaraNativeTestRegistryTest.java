package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.Test;

/** Exercises the native Test registry through the JVM's isolated project-test host. */
public final class HaraNativeTestRegistryTest {
  @Test
  public void runsNativeRegistryFactsAndPreservesCompatibilityMetadata() throws Exception {
    Path root = Files.createTempDirectory("hara-native-test-registry-");
    Path test = root.resolve("test/fixture/advance_test.hal");
    try {
      Files.createDirectories(test.getParent());
      Files.writeString(
          test,
          "(ns fixture.advance-test)\n"
              + "(Test/reset)\n"
              + "(Test/register {:desc \"advance increments\"\n"
              + "                :test (fn [] (+ 41 1))\n"
              + "                :expected 42\n"
              + "                :meta {:refer (quote fixture.advance/advance)\n"
              + "                       :id (quote advance-increments)}})\n"
              + "(Test/run)\n");

      HaraNativeTestRunner.Result result = HaraNativeTestRunner.runFile(root, test);

      assertTrue(result.failureMessage(), result.passed());
      assertEquals(1, result.facts());
      assertEquals(1, result.checks());
      assertEquals(1, result.passedChecks());
      assertEquals(0, result.failedChecks());
      assertTrue(result.rawSummary().contains(":desc \"advance increments\""));
      assertTrue(result.rawSummary().contains(":refer fixture.advance/advance"));
      assertTrue(result.rawSummary().contains(":id advance-increments"));
    } finally {
      Files.deleteIfExists(test);
      Files.deleteIfExists(test.getParent());
      Files.deleteIfExists(test.getParent().getParent());
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void keepsDirectChecksCompatibleWhileRunOwnsTheRegistry() throws Exception {
    Path root = Files.createTempDirectory("hara-native-test-check-");
    Path check = root.resolve("test/fixture/direct_check_test.hal");
    Path invalidRun = root.resolve("test/fixture/invalid_run_test.hal");
    try {
      Files.createDirectories(check.getParent());
      Files.writeString(
          check,
          "(ns fixture.direct-check)\n"
              + "(Test/check [{:name \"legacy direct check\"\n"
              + "              :test (fn [] (+ 40 2))\n"
              + "              :expected 42}])\n");
      HaraNativeTestRunner.Result result = HaraNativeTestRunner.runFile(root, check);
      assertTrue(result.failureMessage(), result.passed());
      assertEquals(1, result.checks());

      Files.writeString(
          invalidRun,
          "(ns fixture.invalid-run)\n"
              + "(Test/run [{:desc \"must use check\"\n"
              + "           :test (fn [] 42)\n"
              + "           :expected 42}])\n");
      HaraException error = assertThrows(
          HaraException.class, () -> HaraNativeTestRunner.runFile(root, invalidRun));
      assertTrue(error.getMessage().contains("use Test/check"));
    } finally {
      Files.deleteIfExists(check);
      Files.deleteIfExists(invalidRun);
      Files.deleteIfExists(check.getParent());
      Files.deleteIfExists(check.getParent().getParent());
      Files.deleteIfExists(root);
    }
  }
}
