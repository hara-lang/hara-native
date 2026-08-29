package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Assume;
import org.junit.Test;

public class HaraSqliteProcessExtensionTest {
  private static final Path ROOT = extensionRoot("db-sqlite-wasm");

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
  public void filesystemDatabasePersistsAcrossProviderReplacement() throws Exception {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/sqlite/wasm/hta/project.edn")));
    Path database = Files.createTempFile("hara-work-store", ".db");
    Files.delete(database);
    String path = database.toString().replace("\\", "\\\\").replace("\"", "\\\"");
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", ROOT.toString());
    try {
      try (Context first =
          Context.newBuilder(HaraLanguage.ID)
              .allowCreateProcess(true)
              .allowIO(IOAccess.ALL)
              .build()) {
        first.eval(
            HaraLanguage.ID,
            "(ns sqlite-restart-first (:require [db.sqlite.wasm :as sqlite])) "
                + "(def connection (deref (sqlite/open "
                + "{:storage :filesystem :path \"" + path + "\"}))) "
                + "(deref (sqlite/exec connection "
                + "\"create table items (id integer primary key, name text not null)\")) "
                + "(deref (sqlite/exec connection "
                + "\"insert into items (name) values (?)\" [\"persistent-wombat\"])) "
                + "(def first-closed (deref (sqlite/close connection)))");
        assertTrue(first.eval(HaraLanguage.ID, "first-closed").asBoolean());
      }

      try (Context second =
          Context.newBuilder(HaraLanguage.ID)
              .allowCreateProcess(true)
              .allowIO(IOAccess.ALL)
              .build()) {
        second.eval(
            HaraLanguage.ID,
            "(ns sqlite-restart-second (:require [db.sqlite.wasm :as sqlite])) "
                + "(def connection (deref (sqlite/open "
                + "{:storage :filesystem :path \"" + path + "\"}))) "
                + "(def result (deref (sqlite/query connection "
                + "\"select name from items\")))");
        assertEquals(
            "persistent-wombat",
            second.eval(HaraLanguage.ID, "(get (get (get result :rows) 0) 0)").asString());
        assertTrue(
            second
                .eval(HaraLanguage.ID, "(deref (sqlite/close connection))")
                .asBoolean());
      }
    } finally {
      Files.deleteIfExists(database);
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
    }
  }

  @Test
  public void sqliteWasmRunsParameterizedSqlThroughTheGenericDbApi() {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/sqlite/wasm/hta/project.edn")));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", ROOT.toString());
    try (Context context =
        Context.newBuilder(HaraLanguage.ID).allowCreateProcess(true).build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns app (:require [db.protocol :as db] "
              + "[db.sqlite.wasm :as sqlite])) "
              + "(def connection (deref (sqlite/open))) "
              + "(deref (sqlite/exec connection "
              + "\"create table items (id integer primary key, name text not null)\")) "
              + "(deref (sqlite/exec connection "
              + "\"insert into items (name) values (?)\" [\"wombat\"])) "
              + "(def result (deref (sqlite/query connection "
              + "\"select id, name from items where name = ?\" [\"wombat\"])))");
      assertEquals("sqlite", context.eval(HaraLanguage.ID, "(name (db/db-engine connection))").asString());
      assertEquals("sqlite-wasm", context.eval(HaraLanguage.ID, "(name (db/db-provider connection))").asString());
      assertEquals(
          "name",
          context.eval(HaraLanguage.ID, "(get (get result :columns) 1)").asString());
      assertEquals(
          "wombat",
          context.eval(HaraLanguage.ID, "(get (get (get result :rows) 0) 1)").asString());
      assertTrue(context.eval(HaraLanguage.ID, "(deref (sqlite/close connection))").asBoolean());
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
    }
  }

  @Test
  public void sqliteWasmRunsThroughTheDatabaseKernelRuntime() {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/sqlite/wasm/hta/project.edn")));
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
              + "[db.sqlite.wasm :as sqlite-wasm])) "
              + "(def runtime-config {:primary {:type :sqlite-wasm :options {}}}) "
              + "(def server (substrate/node-create \"sqlite-runtime-server\")) "
              + "(def client-node (substrate/node-create \"sqlite-runtime-client\")) "
              + "(kernel/register-driver server :sqlite-wasm "
              + "{:id :sqlite-wasm :engine :sqlite "
              + ":open sqlite-wasm/open :version sqlite-wasm/version}) "
              + "(def connected (deref (runtime/local-connect "
              + "client-node server runtime-config {} {}))) "
              + "(def runtime-connection "
              + "(deref (runtime/open-service (get connected :runtime) \"db/primary\"))) "
              + "(deref (db/db-exec runtime-connection "
              + "\"create table items (id integer primary key, name text not null)\" [])) "
              + "(deref (db/db-exec runtime-connection "
              + "\"insert into items (name) values (?)\" [\"runtime-wombat\"])) "
              + "(def runtime-result (deref (db/db-query runtime-connection "
              + "\"select id, name from items\" []))) "
              + "(def runtime-info (db/db-info runtime-connection))");
      assertTrue(
          context.eval(HaraLanguage.ID, "(get connected :transport-attached)").asBoolean());
      assertEquals(
          "setup",
          context.eval(HaraLanguage.ID, "(name (get (get connected :init) :status))").asString());
      assertEquals(
          "sqlite-wasm",
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
  public void sqliteProcessProviderRequiresProcessCapability() {
    Assume.assumeTrue(
        Files.isRegularFile(ROOT.resolve("db/sqlite/wasm/hta/project.edn")));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", ROOT.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Exception error =
          org.junit.Assert.assertThrows(
              Exception.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(ns app (:require [db.sqlite.wasm :as sqlite]))"));
      assertTrue(error.getMessage().contains("capability-denied"));
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
    }
  }
}
