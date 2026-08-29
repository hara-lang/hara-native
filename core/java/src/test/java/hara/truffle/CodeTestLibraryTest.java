package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public class CodeTestLibraryTest {
  @Test
  public void classpathDiscoveryRunsFactsWithStructuredLifecycleEvents() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[:passed 1 1 1 [:test/run-started :test/fact-started :test/fact-completed :test/run-completed]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns code-test-truffle-probe (:use code.test) (:require [work.base :as work])) "
                      + "(fact \"portable\" (promise/from 42) => 42) "
                      + "(let [observer (work/recording-observer) "
                      + "      runtime (work/local-runtime {:observer observer}) "
                      + "      summary (run runtime {:namespace \"code-test-truffle-probe\"}) "
                      + "      positional (run '[code-test])]"
                      + "  (pr-str [(get summary :status) "
                      + "           (get summary :facts) "
                      + "           (get summary :checks) "
                      + "           (get positional :facts) "
                      + "           (vec (filter (fn [event] "
                      + "                          (has? #{:test/run-started "
                      + "                                       :test/fact-started "
                      + "                                       :test/fact-completed "
                      + "                                       :test/run-completed} event)) "
                      + "                        (map (fn [event] (get event :event)) "
                      + "                             (work/observer-events observer))))]))")
              .asString());
    }
  }

  @Test
  public void foundationCompatibilityNamespacesLoadAndCompose() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[true true true 42 {:namespace [std code]}]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns code-test-compat-truffle-probe "
                      + "(:require [code.test :as test] "
                      + "[code.test.checker.common :as common] "
                      + "[code.test.checker.collection :as collection] "
                      + "[code.test.compile.types :as types])) "
                      + "(pr-str (let [fact (types/Fact :core 'id 'probe nil nil "
                      + "\"portable\" 1 1 nil nil (fn [] 42) {})] "
                      + "[(common/succeeded? "
                      + "  (common/verify (common/exactly 1) 1)) "
                      + " (test/comparison-passed? (test/check "
                      + "         (fn [] {:a 1 :b 2}) "
                      + "         (collection/contains-map {:a 1}))) "
                      + " (types/fact? fact) "
                      + " (fact) "
                      + " (test/process-test-args "
                      + "  [\":only\" \"std\" \"code\"])]))")
              .asString());
    }
  }

  @Test
  public void functionsRetainPortableMetadataWithoutLosingExecutability() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[:handler 42 true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [handler (with-meta (fn [value] value) "
                      + "{:handler/id :handler})] "
                      + "  (pr-str [(get (meta handler) :handler/id) "
                      + "           (handler 42) "
                      + "           (fn? handler)]))")
              .asString());
    }
  }

  @Test
  public void dynamicBindingsResolveInTheirDefiningNamespace() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[2 3]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns binding-source) "
                      + "(def ^:dynamic *value* 1) "
                      + "(defn locally [] (binding [*value* 2] *value*)) "
                      + "(ns binding-caller) "
                      + "[(binding-source/locally) "
                      + " (binding [binding-source/*value* 3] binding-source/*value*)]")
              .toString());
    }
  }

}
