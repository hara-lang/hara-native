use hara_native::Runtime;

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
