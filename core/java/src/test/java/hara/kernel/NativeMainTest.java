package hara.kernel;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.Test;

public class NativeMainTest {
  @Test
  public void selectedCasesShareOneRuntimeAndExpectedEvaluatorFailuresPass() throws Exception {
    Path suite = Files.createTempFile("hara-native-jvm-suite-", ".json");
    try {
      Files.writeString(
          suite,
          "{\"format\":\"hara-native/test-suite/1\",\"groups\":{"
              + "\"serial\":[{\"id\":\"define\",\"source\":\"(do (def native-suite-value 42) native-suite-value)\",\"expect\":\"42\"},"
              + "{\"id\":\"read\",\"source\":\"native-suite-value\",\"expect\":\"42\"}],"
              + "\"failure\":[{\"id\":\"missing\",\"source\":\"native-suite-missing\",\"error\":\"native-suite-missing\"}]}}\n");
      ByteArrayOutputStream output = new ByteArrayOutputStream();
      ByteArrayOutputStream error = new ByteArrayOutputStream();
      assertEquals(
          0,
          NativeMain.runTestSuite(
              suite,
              new String[] {"serial", "failure"},
              new PrintStream(output, true, StandardCharsets.UTF_8),
              new PrintStream(error, true, StandardCharsets.UTF_8)));
      assertTrue(output.toString(StandardCharsets.UTF_8).contains("SUMMARY selected=3 passed=3 failed=0"));
      assertEquals("", error.toString(StandardCharsets.UTF_8));
      assertEquals(
          2,
          NativeMain.runTestSuite(
              suite,
              new String[] {"missing"},
              new PrintStream(output, true, StandardCharsets.UTF_8),
              new PrintStream(error, true, StandardCharsets.UTF_8)));
      assertTrue(error.toString(StandardCharsets.UTF_8).contains("native test group is unknown: missing"));
    } finally {
      Files.deleteIfExists(suite);
    }
  }

  @Test
  public void failedCasesRetainTheirActualValueInTheCompleteSummary() throws Exception {
    Path suite = Files.createTempFile("hara-native-jvm-failure-", ".json");
    try {
      Files.writeString(
          suite,
          "{\"format\":\"hara-native/test-suite/1\",\"groups\":{\"core\":["
              + "{\"id\":\"wrong\",\"source\":\"(+ 20 22)\",\"expect\":\"99\"},"
              + "{\"id\":\"also-wrong\",\"source\":\"7\",\"expect\":\"8\"}]}}\n");
      ByteArrayOutputStream output = new ByteArrayOutputStream();
      assertEquals(
          1,
          NativeMain.runTestSuite(
              suite,
              new String[0],
              new PrintStream(output, true, StandardCharsets.UTF_8),
              new PrintStream(new ByteArrayOutputStream(), true, StandardCharsets.UTF_8)));
      String report = output.toString(StandardCharsets.UTF_8);
      assertTrue(report.contains("FAIL  core/wrong"));
      assertTrue(report.contains("actual:   value 42"));
      assertTrue(report.contains("FAIL  core/also-wrong"));
      assertTrue(report.contains("SUMMARY selected=2 passed=0 failed=2"));
    } finally {
      Files.deleteIfExists(suite);
    }
  }
}
