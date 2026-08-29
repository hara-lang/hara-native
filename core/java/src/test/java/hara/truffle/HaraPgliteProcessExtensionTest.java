package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Assume;
import org.junit.Test;

public class HaraPgliteProcessExtensionTest {
  private static final Path ROOT = extensionRoot("db-postgres-wasm");

  private static Path extensionRoot(String name) {
    String configured = System.getenv("HARA_WORKSPACE_ROOT");
    Path relative = Path.of("extensions/hara-runtime/extensions/" + name + "/target/package");
    if (configured != null && !configured.isBlank()) {
      return Path.of(configured).toAbsolutePath().normalize().resolve(relative);
    }
    Path direct = relative.toAbsolutePath().normalize();
    if (Files.isDirectory(direct)) return direct;
    return Path.of("../../../..").toAbsolutePath().normalize().resolve(relative);
  }

  @Test
  public void pgliteRunsParameterizedPostgresqlThroughTheGenericDbApi() {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/postgres/wasm/hta/project.edn")));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", ROOT.toString());
    try (Context context =
        Context.newBuilder(HaraLanguage.ID).allowCreateProcess(true).build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns app (:require [db.protocol :as db] [db.postgres.wasm :as pglite])) "
              + "(def connection (deref (pglite/open))) "
              + "(deref (pglite/exec connection "
              + "\"create table items (id serial primary key, name text not null)\")) "
              + "(deref (pglite/exec connection "
              + "\"insert into items (name) values ($1)\" [\"wombat\"])) "
              + "(def result (deref (pglite/query connection "
              + "\"select id, name from items where name = $1\" [\"wombat\"])))");
      assertEquals(
          "postgresql",
          context.eval(HaraLanguage.ID, "(name (db/db-engine connection))").asString());
      assertEquals(
          "pglite",
          context.eval(HaraLanguage.ID, "(name (db/db-provider connection))").asString());
      assertEquals(
          "name",
          context.eval(HaraLanguage.ID, "(get (get result :columns) 1)").asString());
      assertEquals(
          "wombat",
          context.eval(HaraLanguage.ID, "(get (get (get result :rows) 0) 1)").asString());
      assertTrue(context.eval(HaraLanguage.ID, "(deref (pglite/close connection))").asBoolean());
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
    }
  }

  @Test
  public void pgliteRunsThroughTheDatabaseKernelRuntime() {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/postgres/wasm/hta/project.edn")));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", ROOT.toString());
    try (Context context =
        Context.newBuilder(HaraLanguage.ID).allowCreateProcess(true).build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns runtime-app (:require [db.protocol :as db] "
              + "[std.substrate :as substrate] "
              + "[db.node.kernel-base :as kernel] "
              + "[db.node.runtime :as runtime] "
              + "[db.postgres.wasm :as postgres-wasm])) "
              + "(def runtime-config {:primary {:type :postgres-wasm :options {}}}) "
              + "(def server (substrate/node-create \"pglite-runtime-server\")) "
              + "(def client-node (substrate/node-create \"pglite-runtime-client\")) "
              + "(kernel/register-driver server :postgres-wasm "
              + "{:id :postgres-wasm :engine :postgresql "
              + ":open postgres-wasm/open :version postgres-wasm/version}) "
              + "(def connected (deref (runtime/local-connect "
              + "client-node server runtime-config {} {}))) "
              + "(def runtime-connection "
              + "(deref (runtime/open-service (get connected :runtime) \"db/primary\"))) "
              + "(deref (db/db-exec runtime-connection "
              + "\"create table items (id serial primary key, name text not null)\" [])) "
              + "(deref (db/db-exec runtime-connection "
              + "\"insert into items (name) values ($1)\" [\"runtime-wombat\"])) "
              + "(def runtime-result (deref (db/db-query runtime-connection "
              + "\"select id, name from items\" []))) "
              + "(def runtime-info (db/db-info runtime-connection))");
      assertTrue(
          context.eval(HaraLanguage.ID, "(get connected :transport-attached)").asBoolean());
      assertEquals(
          "setup",
          context.eval(HaraLanguage.ID, "(name (get (get connected :init) :status))").asString());
      assertEquals(
          "pglite",
          context.eval(HaraLanguage.ID, "(name (get runtime-info :provider))").asString());
      assertTrue(context.eval(HaraLanguage.ID, "(get runtime-info :remote)").asBoolean());
      assertEquals(
          "runtime-wombat",
          context
              .eval(HaraLanguage.ID, "(get (get (get runtime-result :rows) 0) 1)")
              .asString());
      assertTrue(context.eval(HaraLanguage.ID, "(deref (db/db-close runtime-connection))").asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (deref (runtime/close-runtime (get connected :runtime) runtime-config)) true)")
              .asBoolean());
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
    }
  }

  @Test
  public void pgliteProcessProviderRequiresProcessCapability() {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/postgres/wasm/hta/project.edn")));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", ROOT.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Exception error =
          org.junit.Assert.assertThrows(
              Exception.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(ns app (:require [db.postgres.wasm :as pglite]))"));
      assertTrue(error.getMessage().contains("capability-denied"));
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
    }
  }
}
