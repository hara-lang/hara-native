package hara.truffle;

import static org.junit.Assert.assertTrue;

import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Direct portable behavior for std.native.Command on the JVM host. */
public final class HaraNativeCommandTest {
  @Test
  public void appsInstallParseDispatchAndRestoreRoutes() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      String result =
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [app (std.native.Command/create {:id (quote demo/cli) :desc \"Demo CLI\"})\n"
                      + "      route (std.native.Command/install app\n"
                      + "              {:id :test :path [\"test\"] :aliases [[\"t\"]] :desc \"Run tests\"\n"
                      + "               :options [{:id :watch :short \"w\" :type :boolean}\n"
                      + "                         {:id :namespace :type :string :many? true}]\n"
                      + "               :arguments [{:id :files :required? false :many? true}]\n"
                      + "               :handler (fn [_] {:stdout \"ok\" :stderr \"\" :exit 0})})\n"
                      + "      _ (std.native.Command/install app\n"
                      + "          {:id :delegate :path [\"delegate\"] :desc \"Delegate flags\"\n"
                      + "           :passthrough? true\n"
                      + "           :arguments [{:id :argv :required? false :many? true}]\n"
                      + "           :handler (fn [_] {:stdout \"delegated\" :stderr \"\" :exit 0})})\n"
                      + "      snapshot (std.native.Command/snapshot app)\n"
                      + "      response (std.native.Command/run app\n"
                      + "                 {:argv [\"t\" \"-w\" \"--namespace\" \"demo.core\" \"a_test.hal\"]\n"
                      + "                  :context {:origin :test}})]\n"
                      + "  (std.native.Command/uninstall app route)\n"
                      + "  (std.native.Command/restore app snapshot)\n"
                      + "  [response\n"
                      + "   (std.native.Command/parse app {:argv [\"delegate\" \"--owned-by-handler\" \"value\"]})\n"
                      + "   (std.native.Command/routes app)\n"
                      + "   (std.native.Command/closed? app)])")
              .toString();
      assertTrue(result, result.contains(":stdout \"ok\""));
      assertTrue(result, result.contains(":exit 0"));
      assertTrue(result, result.contains(":aliases [[\"t\"]]"));
      assertTrue(result, result.contains(":passthrough? true"));
      assertTrue(result, result.contains(":argv [\"--owned-by-handler\" \"value\"]"));
      assertTrue(result, result.endsWith("false]"));
    }
  }

  @Test
  public void runMapsUsageAndInvalidHandlerResponsesWithoutExiting() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      String result =
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [app (std.native.Command/create {:id (quote demo/errors) :desc \"Errors\"})\n"
                      + "      _ (std.native.Command/install app\n"
                      + "          {:id :broken :path [\"broken\"] :desc \"Broken route\"\n"
                      + "           :handler (fn [_] {:stdout \"\" :stderr \"\" :exit 999})})\n"
                      + "      _ (std.native.Command/install app\n"
                      + "          {:id :malformed :path [\"malformed\"] :desc \"Malformed route\"\n"
                      + "           :handler (fn [_] {\"not-stdout\" :stdout \"not-stderr\" :stderr 0 :exit})})]\n"
                      + "  [(std.native.Command/run app {:argv [\"missing\"]})\n"
                      + "   (std.native.Command/run app {:argv [\"broken\"]})\n"
                      + "   (std.native.Command/run app {:argv [\"malformed\"]})])")
              .toString();
      assertTrue(result, result.contains(":exit 2"));
      assertTrue(result, result.contains(":command/unknown-route"));
      assertTrue(result, result.contains(":exit 1"));
      assertTrue(result, result.split(":exit 1", -1).length - 1 == 2);
      assertTrue(result, result.contains(":exit must be an integer between 0 and 255"));
    }
  }
}
