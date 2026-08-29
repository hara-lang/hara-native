package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.nio.file.Path;
import org.junit.Test;

public final class HaraPostgresCompatibilityTest {
  private static final Path ROOT = Path.of(".").toAbsolutePath().normalize();

  private static void assertSuite(String path, int facts, int checks) throws Exception {
    HaraNativeTestRunner.Result result =
        HaraNativeTestRunner.runFile(ROOT, ROOT.resolve(path));

    assertTrue(result.failureMessage(), result.passed());
    assertEquals(facts, result.facts());
    assertEquals(checks, result.checks());
    assertEquals(checks, result.passedChecks());
    assertEquals(0, result.failedChecks());
    assertEquals(0, result.errors());
    assertEquals(0, result.timeouts());
  }

  @Test
  public void runsMergedPostgresConnectionProviderSuite() throws Exception {
    assertSuite("lib/test/db/postgres/connection_test.hal", 11, 11);
  }

  @Test
  public void runsManagedDbPostgresLifecycleFacade() throws Exception {
    assertSuite("lib/test/db/postgres_test.hal", 8, 8);
  }

  @Test
  public void runsDirectDatabaseProtocolSuite() throws Exception {
    assertSuite("lib/test/db/protocol_test.hal", 3, 3);
  }
}
