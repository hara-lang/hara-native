package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import org.junit.Test;

public class CodeVmConformanceTest {
  private static final CodeVmConformance.Report REPORT = loadReport();

  @Test
  public void sharedCorpusRunsThroughTheProductionTruffleJournal() {
    assertEquals(CodeVmConformance.CORPUS_SCHEMA, REPORT.corpus().schema());
    assertEquals("code.vm/production", REPORT.corpus().id());
    assertTrue(REPORT.cases().size() >= 13);
    assertTrue(REPORT.passed());

    CodeVmConformance.Observation arithmetic = observation("arith/nested");
    assertEquals("returned", arithmetic.outcome().status());
    assertEquals("7", arithmetic.outcome().display());
    assertTrue(
        arithmetic.journal().events().stream()
            .allMatch(event -> event.sequence() > 0));
  }

  @Test
  public void boundedJournalTruncationDoesNotStopEvaluation() {
    String source =
        "{:corpus/schema \"hal.code-vm-production-corpus/0-alpha\" "
            + ":corpus/id :code.vm/truncation-test "
            + ":corpus/upstream \"test-only\" "
            + ":cases [{:id :trace/tiny :upstream-id :test/tiny "
            + ":source \"(+ 1 2)\" :trace-limit 2 "
            + ":expect {:display \"3\"}}]}";
    CodeVmConformance.Observation bounded =
        CodeVmConformance.runCorpus(source).cases().get(0);
    assertEquals("returned", bounded.outcome().status());
    assertEquals("3", bounded.outcome().display());
    assertTrue(bounded.outcome().truncated());
    assertTrue(bounded.journal().events().size() <= 2);
    assertEquals(
        "journal/truncated",
        bounded.journal().events().get(bounded.journal().events().size() - 1).kind());

    CodeVmConformance.Observation loop = observation("loop/many-iterations");
    assertEquals("1024", loop.outcome().display());
    assertTrue(loop.journal().events().size() <= loop.testCase().traceLimit());
  }

  @Test
  public void compileOnlyCasesStayExplicitlyUnsupportedWithoutFallback() {
    for (String id :
        java.util.List.of(
            "compile/recur-outside-loop",
            "compile/unbound-symbol",
            "compile/fn-multi-arity")) {
      CodeVmConformance.Observation observation = observation(id);
      assertFalse(observation.testCase().interpreterRequired());
      assertEquals("unsupported", observation.outcome().status());
      assertNull(observation.journal());
      assertTrue(observation.passed());
    }
  }

  @Test
  public void reportIsTerminalNeutralBrowserSafeAndDeterministic() {
    String report = CodeVmConformanceDocument.json(REPORT, false, true);
    String browser = CodeVmConformanceDocument.json(REPORT, true, false);
    assertTrue(
        report.contains(
            "\"schema\": \"hal.code-vm-conformance-runtime/0-alpha\""));
    assertTrue(report.contains("\"terminalNeutral\": true"));
    assertTrue(report.contains("\"truffle\""));
    assertTrue(browser.contains("\"id\":\"arith/nested\""));
    assertFalse(browser.contains("\"id\":\"loop/many-iterations\""));
    assertFalse(report.contains("\u001b["));

    CodeVmConformance.Report repeated = loadReport();
    assertEquals(
        observation(REPORT, "arith/nested").teaching(),
        observation(repeated, "arith/nested").teaching());
  }

  @Test
  public void stableCheckCommandReturnsSuccess() {
    ByteArrayOutputStream outputBytes = new ByteArrayOutputStream();
    ByteArrayOutputStream errorBytes = new ByteArrayOutputStream();
    int status =
        CodeVmConformance.run(
            new String[] {"check"},
            new PrintStream(outputBytes, true, StandardCharsets.UTF_8),
            new PrintStream(errorBytes, true, StandardCharsets.UTF_8));
    assertEquals(errorBytes.toString(StandardCharsets.UTF_8), 0, status);
    assertTrue(
        outputBytes.toString(StandardCharsets.UTF_8).contains("conformance passed"));
  }

  private static CodeVmConformance.Report loadReport() {
    try {
      return CodeVmConformance.runEmbedded();
    } catch (Exception error) {
      throw new ExceptionInInitializerError(error);
    }
  }

  private static CodeVmConformance.Observation observation(String id) {
    return observation(REPORT, id);
  }

  private static CodeVmConformance.Observation observation(
      CodeVmConformance.Report report, String id) {
    return report.cases().stream()
        .filter(value -> value.testCase().id().equals(id))
        .findFirst()
        .orElseThrow(
            () -> new AssertionError("missing code.vm corpus case " + id));
  }
}
