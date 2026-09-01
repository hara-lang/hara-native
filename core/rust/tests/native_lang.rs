use hara_native::Runtime;

#[test]
fn direct_native_conforms_to_the_portable_catch_shape() {
    let mut runtime = Runtime::core();
    runtime
        .set_execution_backend("direct-native")
        .expect("native builds must expose the direct-native backend");
    assert_eq!(
        runtime
            .eval_direct_native("(try (throw (ex :test/failure {})) (catch error :caught))")
            .unwrap(),
        ":caught"
    );
    let error = runtime
        .eval_direct_native(
            "(try (throw (ex :test/failure {})) (catch Throwable error :caught))",
        )
        .expect_err("typed host catch syntax must not compile as Hara");
    assert!(
        error.contains(
            "catch expects a binding symbol and one handler form; typed host catch clauses are not part of Hara"
        ),
        "{error}"
    );
}

#[test]
fn direct_native_reads_spanned_forms_without_evaluation() {
    let mut runtime = Runtime::core();
    runtime
        .set_execution_backend("direct-native")
        .expect("native builds must expose the direct-native backend");
    assert_eq!(
        runtime
            .eval_direct_native(
                r#"(let [nodes (std.native.Edn/read-forms-spanned "; comment\n(defn sample [x]\n  (inc x))")
                         node (std.protocol.ilookup.ILookup/lookup nodes 0)
                         call (std.protocol.ilookup.ILookup/lookup (:children node) 3)
                         metadata (std.protocol.ilookup.ILookup/lookup
                                   (std.native.Edn/read-forms-spanned "^{:refer demo.subject/run}\n(def run [] 1)")
                                   0)
                         unevaluated (std.protocol.ilookup.ILookup/lookup
                                      (std.native.Edn/read-forms-spanned "(throw (ex :host {} :ex/message \"not evaluated\"))")
                                      0)]
                     [(= (:form node) (quote (defn sample [x] (inc x))))
                      (= [(:offset (:start node)) (:line (:start node)) (:column (:start node))
                          (:offset (:end node)) (:line (:end node)) (:column (:end node))]
                         [10 2 1 37 3 11])
                      (= (:form call) (quote (inc x)))
                      (= [(:offset (:start call)) (:line (:start call)) (:column (:start call))
                          (:offset (:end call)) (:line (:end call)) (:column (:end call))]
                         [29 3 3 36 3 10])
                      (= (:refer (std.protocol.iobjtype.IObjType/meta (:form metadata)))
                         (quote demo.subject/run))
                      (= (:form unevaluated) (quote (throw (ex :host {} :ex/message "not evaluated"))))])"#,
            )
            .unwrap(),
        "[true true true true true true]"
    );
}

#[test]
fn direct_native_spanned_reader_reports_parse_positions() {
    let mut runtime = Runtime::core();
    runtime
        .set_execution_backend("direct-native")
        .expect("native builds must expose the direct-native backend");
    let error = runtime
        .eval_direct_native("(std.native.Edn/read-forms-spanned \"[1\\n2\")")
        .expect_err("the reader must reject an unterminated form");
    assert!(
        error.contains("read-forms-spanned failed: EOF while reading vector [line 2, column 2]"),
        "{error}"
    );
}

#[test]
fn lang_values_are_qualified_and_library_snapshots_restore_the_baseline() {
    let mut runtime = Runtime::core();
    let display = runtime
        .eval_native(
            "(let [book (std.lang.Book/create {:coordinate (quote demo/book)}) \
                   library (std.lang.Library/create {}) \
                   _ (std.lang.Library/install library book) \
                   snapshot (std.lang.Library/snapshot library) \
                   _ (std.lang.Library/remove library (quote demo/book)) \
                   _ (std.lang.Library/restore library snapshot) \
                   harness (std.lang.Harness/create {}) \
                   _ (std.lang.Harness/close harness) \
                   _ (std.lang.Harness/reset harness)] \
               [(std.native.Base/instance? std.lang.Book book) \
                (std.protocol.ilookup.ILookup/lookup \
                  (std.lang.Library/state library) :book-count) \
                (std.lang.Harness/closed? harness)])",
        )
        .unwrap();

    assert_eq!(display, "[true 1 false]");
}

#[test]
fn lang_runtime_close_and_reset_are_idempotent() {
    let mut runtime = Runtime::core();
    let display = runtime
        .eval_native(
            "(let [runtime (std.lang.Runtime/create {})] \
               (std.lang.Runtime/close runtime) \
               (std.lang.Runtime/close runtime) \
               (std.lang.Runtime/reset runtime) \
               [(std.native.Base/instance? std.lang.Runtime runtime) \
                (std.lang.Runtime/closed? runtime) \
                (std.protocol.ilookup.ILookup/lookup \
                  (std.lang.Runtime/state runtime) :state)])",
        )
        .unwrap();

    assert_eq!(display, "[true false :ready]");
}

#[test]
fn library_rejects_books_without_an_explicit_coordinate() {
    let mut runtime = Runtime::core();
    let display = runtime
        .eval_native(
            "(let [library (std.lang.Library/create {}) \
                   book (std.lang.Book/create {:id (quote fixture/book)})] \
               (try \
                 (std.lang.Library/install library book) \
                 false \
                 (catch _ true)))",
        )
        .unwrap();

    assert_eq!(display, "true");
}
