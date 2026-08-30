use hara_native::Runtime;

#[test]
fn command_apps_parse_routes_dispatch_handlers_and_restore_snapshots() {
    let mut runtime = Runtime::core();
    let display = runtime
        .eval_native(
            "(let [app (std.native.Command/create {:id (quote demo/cli) :desc \"Demo CLI\"}) \
                   route (std.native.Command/install app \
                           {:id :test :path [\"test\"] :aliases [[\"t\"]] :desc \"Run tests\" \
                            :options [{:id :watch :short \"w\" :type :boolean} \
                                      {:id :namespace :type :string :many? true}] \
                            :arguments [{:id :files :required? false :many? true}] \
                            :handler (fn [_] {:stdout \"ok\" :stderr \"\" :exit 0})}) \
                   _ (std.native.Command/install app \
                       {:id :delegate :path [\"delegate\"] :desc \"Delegate flags\" \
                        :passthrough? true \
                        :arguments [{:id :argv :required? false :many? true}] \
                        :handler (fn [_] {:stdout \"delegated\" :stderr \"\" :exit 0})}) \
                   snapshot (std.native.Command/snapshot app) \
                   response (std.native.Command/run app \
                              {:argv [\"t\" \"-w\" \"--namespace\" \"demo.core\" \"a_test.hal\"] \
                               :context {:origin :test}})] \
               (std.native.Command/uninstall app route) \
               (std.native.Command/restore app snapshot) \
               [response \
                (std.native.Command/parse app {:argv [\"delegate\" \"--owned-by-handler\" \"value\"]}) \
                (std.native.Command/routes app) \
                (std.native.Command/closed? app)])",
        )
        .unwrap();
    assert!(display.contains(":stdout \"ok\""));
    assert!(display.contains(":exit 0"));
    assert!(display.contains(":aliases [[\"t\"]]"));
    assert!(display.contains(":passthrough? true"));
    assert!(display.contains(":argv [\"--owned-by-handler\" \"value\"]"));
    assert!(display.ends_with("false]"));
}

#[test]
fn command_run_distinguishes_usage_and_handler_failures_without_exiting() {
    let mut runtime = Runtime::core();
    let display = runtime
        .eval_native(
            "(let [app (std.native.Command/create {:id (quote demo/errors) :desc \"Errors\"}) \
                   _ (std.native.Command/install app \
                       {:id :broken :path [\"broken\"] :desc \"Broken route\" \
                        :handler (fn [_] {:stdout \"\" :stderr \"\" :exit 999})})] \
               [(std.native.Command/run app {:argv [\"missing\"]}) \
                (std.native.Command/run app {:argv [\"broken\"]})])",
        )
        .unwrap();
    assert!(display.contains(":exit 2"));
    assert!(display.contains(":command/unknown-route"));
    assert!(display.contains(":exit 1"));
    assert!(display.contains(":command/invalid-response"));
}
