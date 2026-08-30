use hara_native::Runtime;

#[test]
fn test_check_is_the_case_level_api_and_run_rejects_case_vectors() {
    let mut runtime = Runtime::core();
    let checked = runtime
        .eval_native(
            "(Test/check [{:desc \"advance increments\" :test (fn [] (+ 20 22)) :expected 42}])",
        )
        .unwrap();
    assert!(checked.contains(":desc \"advance increments\""));
    assert!(checked.contains(":success true"));

    let error = runtime
        .eval_native("(Test/run [{:desc \"legacy\" :test (fn [] 1) :expected 1}])")
        .unwrap_err();
    assert!(error.contains("use Test/check for cases"));
}

#[test]
fn test_registry_preserves_code_test_metadata_and_summary_shape() {
    let mut runtime = Runtime::core();
    let summary = runtime
        .eval_native(
            "(ns test.registry) \
             (Test/reset) \
             (Test/register {:desc \"advance increments\" \
                             :test (fn [] (+ 41 1)) \
                             :expected 42 \
                             :meta {:refer (quote test.registry/advance) \
                                    :id (quote advance-increments) \
                                    :added \"3.0\" \
                                    :tags #{:unit :native}}}) \
             (Test/run)",
        )
        .unwrap();
    for expected in [
        ":status :passed",
        ":counts",
        ":check-counts",
        ":desc \"advance increments\"",
        ":name \"advance increments\"",
        ":refer test.registry/advance",
        ":id advance-increments",
        ":added \"3.0\"",
        ":test/namespace \"test.registry\"",
        ":test/order 1",
    ] {
        assert!(
            summary.contains(expected),
            "missing {expected} in {summary}"
        );
    }
}

#[test]
fn test_registry_scopes_facts_and_reset_is_idempotent() {
    let mut runtime = Runtime::core();
    let alpha_facts = runtime
        .eval_native(
            "(ns test.alpha) \
             (Test/reset) \
             (Test/register {:desc \"alpha fact\" :test (fn [] 1) :expected 1}) \
             (ns test.beta) \
             (Test/register {:desc \"beta fact\" :test (fn [] 2) :expected 2}) \
             (Test/facts (quote test.alpha))",
        )
        .unwrap();
    assert!(alpha_facts.contains("alpha fact"));
    assert!(!alpha_facts.contains("beta fact"));

    let empty = runtime
        .eval_native("(Test/reset) (Test/reset) (Test/facts (quote test.alpha))")
        .unwrap();
    assert_eq!(empty, "[]");
}

#[test]
fn test_registry_runs_function_facts_and_guarantees_after_hook_reporting() {
    let mut runtime = Runtime::core();
    let summary = runtime
        .eval_native(
            "(ns test.lifecycle) \
             (Test/reset) \
             (Test/register {:desc \"advance increments\" \
                             :function (fn [_] (Test/check [{:desc \"inner check\" \
                                                              :test (fn [] (+ 41 1)) \
                                                              :expected 42}])) \
                             :after (fn [] (throw (ex :state {:message \"teardown\"}))) \
                             :meta {:refer (quote test.lifecycle/advance) \
                                    :id (quote advance-increments)}}) \
             (Test/run)",
        )
        .unwrap();
    assert!(summary.contains(":status :failed"));
    assert!(summary.contains(":throw 1"));
    assert!(summary.contains(":status :error"));
    assert!(summary.contains("teardown"));
    assert!(summary.contains(":checks 1"));
}

#[test]
fn test_registry_reports_skipped_and_failed_facts_without_losing_metadata() {
    let mut runtime = Runtime::core();
    let summary = runtime
        .eval_native(
            "(ns test.status) \
             (Test/reset) \
             (Test/register {:desc \"skip this\" :test (fn [] 1) :expected 2 \
                             :meta {:skip true :id (quote skip-this)}}) \
             (Test/register {:desc \"fail this\" :test (fn [] 1) :expected 2 \
                             :meta {:id (quote fail-this)}}) \
             (Test/run)",
        )
        .unwrap();
    for expected in [
        ":status :failed",
        ":skipped 1",
        ":failed 1",
        ":id skip-this",
        ":id fail-this",
    ] {
        assert!(
            summary.contains(expected),
            "missing {expected} in {summary}"
        );
    }
}
