package hara.truffle;

import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;
import org.junit.Test;

/** Direct Hara calls into the native Work plan, registry, runtime, and host surface. */
public class HaraNativeWorkPlanTest {
  private static Path haraProjectRoot() {
    for (Path directory = Path.of("").toAbsolutePath(); directory != null; directory = directory.getParent()) {
      Path candidate = directory.resolve("hara");
      if (Files.isRegularFile(candidate.resolve("src/work/core.hal"))) return candidate;
    }
    throw new IllegalStateException("cannot locate the hara project from test working directory");
  }

  @Test
  public void planBuildersAreDirectNativeCallsAndRoundTripThroughHta() {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(haraProjectRoot())
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      assertTrue(
          List.of(
                  "(Work/plan? (Work/configured :pure {:work/target \"fixture/value\"}))",
                  "(Work/plan? (Work/pure \"fixture/value\"))",
                  "(Work/plan? (Work/step \"fixture/value\"))",
                  "(Work/plan? (Work/chain [(Work/pure \"fixture/value\")]))",
                  "(Work/plan? (Work/all [(Work/pure \"fixture/value\")]))",
                  "(Work/plan? (Work/each (Work/pure \"fixture/value\")))",
                  "(Work/plan? (Work/filter (Work/pure \"fixture/value\")))",
                  "(Work/plan? (Work/fold 0 (Work/pure \"fixture/value\")))",
                  "(Work/plan? (Work/choose (Work/pure \"fixture/value\") {:value (Work/pure \"fixture/value\")}))",
                  "(Work/plan? (Work/graph {:value (Work/pure \"fixture/value\")} [:value]))",
                  "(Work/plan? (Work/batch (Work/pure \"fixture/value\")))",
                  "(Work/plan? (Work/bind (Work/pure \"fixture/value\") \"fixture/value\"))",
                  "(Work/plan? (Work/ensure (Work/pure \"fixture/value\") (Work/step \"fixture/value\")))",
                  "(Work/plan? (Work/await :wait))",
                  "(Work/plan? (Work/decode-hta (Work/encode-hta (Work/pure \"fixture/value\"))))")
              .stream()
              .allMatch(source -> context.eval(HaraLanguage.ID, source).asBoolean()));
    }
  }

  @Test
  public void registryRuntimeAndHostRemainOpaqueNativeValues() {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(haraProjectRoot())
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(let [registry (Work/new-registry) target (fn [input] input) plan (Work/pure \"fixture/value\") runtime (Work/new-runtime registry) _ (Work/bind-target registry \"fixture/value\" target) evaluated (Work/evaluate runtime plan 7) evaluation (deref evaluated) submitted (Work/submit-plan (Work/default-host) runtime plan 8 {}) submission (deref (IWorkRun/work-result submitted))] [(Base/type registry) (Base/type (Work/target registry \"fixture/value\")) (Base/type (Work/target-names registry)) evaluation (Base/type runtime) (Base/type (Work/runtime-registry runtime)) (Base/type evaluated) submission (Base/type submitted) (Work/unbind-target registry \"fixture/value\") (Base/type (Work/reset-registry registry)) (Work/target-names registry) (Base/type (Work/reset-runtime runtime))])");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArraySize() == 13);
      assertTrue(result.getArrayElement(0).toString().contains("std.native.WorkRegistry"));
      assertTrue(result.getArrayElement(3).fitsInLong());
      assertTrue(result.getArrayElement(3).asLong() == 7L);
      assertTrue(result.getArrayElement(4).toString().contains("std.native.WorkRuntime"));
      assertTrue(result.getArrayElement(6).toString().contains("std.native.Promise"));
      assertTrue(result.getArrayElement(7).fitsInLong());
      assertTrue(result.getArrayElement(7).asLong() == 8L);
      assertTrue(result.getArrayElement(8).toString().contains("std.native.WorkRun"));
      assertTrue(result.getArrayElement(11).hasArrayElements());
      assertTrue(result.getArrayElement(11).getArraySize() == 0);
    }
  }

  @Test
  public void haraWorkCoreBuildsAndResetsDefinitionBundlesThroughNativeWork() throws Exception {
    Path project = haraProjectRoot();
    String source = Files.readString(project.resolve("src/work/core.hal"));
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(project)
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      context.eval(HaraLanguage.ID, source);
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(let [bundle (work.core/bundle (Work/pure \"fixture/id\") {\"fixture/id\" (fn [input] input)}) prepared (work.core/prepare bundle) evaluation (deref (work.core/evaluate prepared bundle 7)) run (work.core/submit bundle 8) submission (deref (IWorkRun/work-result run))] [(Work/plan? (work.core/plan bundle)) (Base/type (:work/registry prepared)) (Base/type (:work/runtime prepared)) evaluation submission (Work/target-names (:work/registry (work.core/reset prepared)))])");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArrayElement(0).asBoolean());
      assertTrue(result.getArrayElement(1).toString().contains("std.native.WorkRegistry"));
      assertTrue(result.getArrayElement(2).toString().contains("std.native.WorkRuntime"));
      assertTrue(result.getArrayElement(3).fitsInLong());
      assertTrue(result.getArrayElement(3).asLong() == 7L);
      assertTrue(result.getArrayElement(4).fitsInLong());
      assertTrue(result.getArrayElement(4).asLong() == 8L);
      assertTrue(result.getArrayElement(5).hasArrayElements());
      assertTrue(result.getArrayElement(5).getArraySize() == 0);
    }
  }

  @Test
  public void haraWorkTaskResultSummarisesPortableTaskRecords() throws Exception {
    Path project = haraProjectRoot();
    String source = Files.readString(project.resolve("src/work/flow/task/result.hal"));
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(project)
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      context.eval(HaraLanguage.ID, source);
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(let [batch {:items [{:item/id :one} {:item/id :two} {:item/id :three}] :results [{:item/value {:status :return :data 7 :elapsed 5 :changed true :inserts 2 :write true}} {:item/value {:status :warn :data :notice :elapsed 2}} {:item/value {:status :error :data :bad :elapsed 1 :deletes 3}}]} summary (:summary (work.flow.task.result/transform-summary batch {}))] [(:items summary) (:results summary) (:warnings summary) (:errors summary) (:cumulative summary) (:changed summary) (:inserts summary) (:deletes summary) (:written summary) (:verified summary)])");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArraySize() == 10);
      assertTrue(result.getArrayElement(0).asLong() == 3L);
      assertTrue(result.getArrayElement(1).asLong() == 1L);
      assertTrue(result.getArrayElement(2).asLong() == 1L);
      assertTrue(result.getArrayElement(3).asLong() == 1L);
      assertTrue(result.getArrayElement(4).asLong() == 8L);
      assertTrue(result.getArrayElement(5).asLong() == 1L);
      assertTrue(result.getArrayElement(6).asLong() == 2L);
      assertTrue(result.getArrayElement(7).asLong() == 3L);
      assertTrue(result.getArrayElement(8).asLong() == 1L);
      assertTrue(result.getArrayElement(9).asLong() == 0L);
    }
  }

  @Test
  public void haraWorkflowRegistryCompilesAndRestoresDefinitionEnvelopes() throws Exception {
    Path project = haraProjectRoot();
    String source = Files.readString(project.resolve("src/work/flow.hal"));
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(project)
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      context.eval(HaraLanguage.ID, source);
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(try (work.flow/reset) (let [flow {:flow/path [:fixture] :flow/defaults {:nested {:left 1}} :flow/normalise (fn [profile name definition] (assoc definition :name name)) :flow/compile (fn [profile definition] (assoc definition :compiled true))} _ (work.flow/register flow) profile (work.flow/configure [:fixture] {:value 2}) installed (work.flow/define-work :sample [:fixture] {:value 3}) state (work.flow/snapshot) _ (work.flow/reset) _ (work.flow/restore state)] [(= (:workflow/options profile) {:nested {:left 1} :value 2}) (= installed {:name :sample :compiled true :value 3}) (= (work.flow/definition :sample) installed) (= (vec (keys (work.flow/registered))) [[:fixture]])]) (finally (work.flow/reset)))");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArraySize() == 4);
      assertTrue(result.getArrayElement(0).asBoolean());
      assertTrue(result.getArrayElement(1).asBoolean());
      assertTrue(result.getArrayElement(2).asBoolean());
      assertTrue(result.getArrayElement(3).asBoolean());
    }
  }

  @Test
  public void haraTaskletCompilesToAndExecutesThroughNativeWork() throws Exception {
    Path project = haraProjectRoot();
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(project)
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/core.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/result.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/command_task.hal")));
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(let [bundle (work.flow.task.command-task/compile {:id :double :run (fn [value context] {:status :return :data (* value 2)})}) record (deref (work.core/evaluate bundle 4))] [(Work/plan? (:work/plan bundle)) (work.flow.task.command-task/task-work? bundle) (:item/id record) (:status record) (:data record)])");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArraySize() == 5);
      assertTrue(result.getArrayElement(0).asBoolean());
      assertTrue(result.getArrayElement(1).asBoolean());
      assertTrue(result.getArrayElement(2).toString().contains("double"));
      assertTrue(result.getArrayElement(3).toString().contains("return"));
      assertTrue(result.getArrayElement(4).asLong() == 8L);
    }
  }

  @Test
  public void haraTaskCommandDelegatesArgvParsingToNativeCommand() throws Exception {
    Path project = haraProjectRoot();
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(project)
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/core.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/command.hal")));
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(let [bundle (work.core/bundle (Work/pure \"fixture/echo\") {\"fixture/echo\" (fn [input] {:status :return :data (:command/options input)})}) app (work.flow.task.command/create {:id 'fixture/command :desc \"Command\"} [{:command/options {:id :echo :path [\"echo\"] :desc \"Echo\" :options [{:id :verbose :type :boolean}]} :command/bundle bundle}])] (try (let [response (work.flow.task.command/run app [\"echo\" \"--verbose\"] {:origin :test})] [(:exit response) (:stderr response)]) (finally (Command/close app))))");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArraySize() == 2);
      assertTrue(result.getArrayElement(0).asLong() == 0L);
      assertTrue(result.getArrayElement(1).asString().isEmpty());
    }
  }

  @Test
  public void haraTaskFlowDefinesAndInvokesNativeWorkBundles() throws Exception {
    Path project = haraProjectRoot();
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(project)
            .allowAllAccess(true)
            .allowIO(org.graalvm.polyglot.io.IOAccess.ALL)
            .build()) {
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/core.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/selector.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/result.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/command_task.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task/command.hal")));
      context.eval(HaraLanguage.ID, Files.readString(project.resolve("src/work/flow/task.hal")));
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(let [bundle (work.flow/define-work :fixture/double [:task] {:run (fn [value context] (* value 2))}) record (deref (work.flow.task/invoke bundle 5))] [(Work/plan? (:work/plan bundle)) (work.flow.task.command-task/task-work? bundle) (:item/id record) (:status record) (:data record)])");
      assertTrue(result.hasArrayElements());
      assertTrue(result.getArraySize() == 5);
      assertTrue(result.getArrayElement(0).asBoolean());
      assertTrue(result.getArrayElement(1).asBoolean());
      assertTrue(result.getArrayElement(2).toString().contains("fixture/double"));
      assertTrue(result.getArrayElement(3).toString().contains("return"));
      assertTrue(result.getArrayElement(4).asLong() == 10L);
    }
  }

}
