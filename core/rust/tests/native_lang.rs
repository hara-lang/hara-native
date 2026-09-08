use hara_native::Runtime;

#[test]
fn host_delay_protocols_defer_and_memoize_native_functions() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        assert_eq!(runtime.eval_native(
            "(let [calls (Base/atom 0)
                   lazy (Base/delay (fn []
                          (IReset/reset calls (+ 1 (IDeref/deref calls)))
                          false))
                   before [(IDeref/deref calls) (IRealize/realized? lazy)]]
               [before (IDeref/deref lazy) (IRealize/realize lazy)
                (IDeref/deref calls) (IRealize/realized? lazy)
                (Base/type lazy)])"
        ).unwrap(), "[[0 false] false false 1 true :std.native.Delay]", "{backend}");
        assert_eq!(runtime.eval_native(
            "(let [calls (Base/atom 0)
                   lazy (Base/delay (fn []
                          (IReset/reset calls (+ 1 (IDeref/deref calls)))
                          (throw (Exception/new \"delay boom\" {:reason :delayed}))))
                   first (try (IDeref/deref lazy) (catch e e))
                   second (try (IRealize/realize lazy) (catch e e))]
               [(= first second) (IDeref/deref calls) (IRealize/realized? lazy)])"
        ).unwrap(), "[true 1 true]", "{backend}");
        assert!(runtime.eval_native("(Base/delay 1)").unwrap_err().contains("expects one function"));
    }
}

#[test]
fn host_delay_is_lazy_and_caches_success_including_nil_and_false() {
    use hara_native::lang::data::delay::Delay;
    use std::{cell::Cell, rc::Rc};
    for value in [None, Some(false), Some(true)] {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let delay = Delay::new(move || {
            observed.set(observed.get() + 1);
            Ok(value)
        });
        assert_eq!(calls.get(), 0);
        assert!(!delay.is_realized());
        let alias = delay.clone();
        assert!(delay.same_identity(&alias));
        assert_eq!(delay.identity_address(), alias.identity_address());
        assert_eq!(delay.deref_value(), Ok(value));
        assert_eq!(alias.deref_value(), Ok(value));
        assert!(alias.is_realized());
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn host_delay_caches_failure_and_releases_thunk_captures() {
    use hara_native::lang::data::delay::Delay;
    use std::{cell::Cell, rc::Rc};
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let delay = Delay::<()>::new(move || {
        observed.set(observed.get() + 1);
        Err("original failure".into())
    });
    assert_eq!(Rc::strong_count(&calls), 2);
    assert_eq!(delay.deref_value(), Err("original failure".into()));
    assert!(delay.is_realized());
    assert_eq!(delay.deref_value(), Err("original failure".into()));
    assert_eq!(calls.get(), 1);
    assert_eq!(Rc::strong_count(&calls), 1);
}

#[test]
fn host_delay_drop_releases_unrealized_thunk_without_running_it() {
    use hara_native::lang::data::delay::Delay;
    use std::{cell::Cell, rc::Rc};
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let delay = Delay::new(move || {
        observed.set(observed.get() + 1);
        Ok(42)
    });
    let alias = delay.clone();
    drop(delay);
    assert_eq!(Rc::strong_count(&calls), 2);
    drop(alias);
    assert_eq!(Rc::strong_count(&calls), 1);
    assert_eq!(calls.get(), 0);
}

#[test]
fn host_delay_reentrant_realization_fails_without_borrow_panic() {
    use hara_native::lang::data::delay::Delay;
    use std::{cell::RefCell, rc::Rc};
    let slot: Rc<RefCell<Option<Delay<()>>>> = Rc::new(RefCell::new(None));
    let captured = slot.clone();
    let delay = Delay::new(move || captured.borrow().as_ref().unwrap().deref_value());
    *slot.borrow_mut() = Some(delay.clone());
    assert_eq!(delay.deref_value(), Err("delay realization is recursive".into()));
    assert!(delay.is_realized());
    assert_eq!(delay.deref_value(), Err("delay realization is recursive".into()));
    slot.borrow_mut().take();
    assert_eq!(Rc::strong_count(&slot), 1);
}

#[test]
fn evaluator_accepts_single_clause_functions_but_requires_a_body() {
    let mut runtime = Runtime::core();
    assert_eq!(runtime.eval_native("((fn ([value] value)) 7)").unwrap(), "7");
    for source in ["(fn)", "(fn [])", "(fn ())", "(fn ([]))"] {
        assert!(runtime.eval_native(source).is_err(), "{source}");
    }
}

#[test]
fn direct_native_caller_evaluation_preserves_lexical_function_bindings() {
    let mut runtime = Runtime::core();
    runtime.set_execution_backend("direct-native").unwrap();
    runtime.register_resource(
        "example.context-owner",
        "(ns example.context-owner)
         (defn helper [value] (+ value 100))
         (defn lexical [value] (helper value))
         (defn current [] (Base/current-namespace))
         (defn evaluate [form] (Runtime/eval form))",
    );
    assert_eq!(
        runtime.eval_native(
            "(ns example.context-caller (:require [example.context-owner :as owner]))
             (defn helper [value] (+ value 9))
             [(owner/lexical 3)
              (= (owner/current) (Base/current-namespace))
              (owner/evaluate '(helper 3))]"
        ).unwrap(),
        "[103 true 12]"
    );
    assert_eq!(
        runtime.eval_native(
            "(let [template (example.context-owner/evaluate '(fn [value] (helper value)))]
               (template 4))"
        ).unwrap(),
        "13"
    );
    assert!(runtime.eval_native(
        "(example.context-owner/evaluate '(throw (ex :context/failure {})))"
    ).is_err());
    assert_eq!(runtime.eval_native("(helper 5)").unwrap(), "14");
    assert_eq!(runtime.eval_native(
        "(let [template (example.context-owner/evaluate '(fn ([value] (helper value))))]
           (template 2))"
    ).unwrap(), "11");
    assert!(runtime.eval_native("(Runtime/eval '(fn []))").is_err());
    assert_eq!(runtime.eval_native(
        "(let [template (example.context-owner/evaluate
                          '(fn ([value] (helper value))
                               ([value other] (+ value other))
                               ([value other & remaining] remaining)))]
           [(template 1) (template 2 3) (template 2 3 4 5)])"
    ).unwrap(), "[10 5 (4 5)]");
    assert_eq!(runtime.eval_native(
        "[(example.context-owner/evaluate
            '(Runtime/eval-in 'example.context-owner '[(Runtime/eval '(helper 1))]))
          (example.context-owner/evaluate '(helper 1))]"
    ).unwrap(), "[101 10]");
}

#[test]
fn macro_arguments_preserve_unqualified_division() {
    let registry = hara_native::core::minimal_namespace_registry();
    hara_native::core::install_foundation_intrinsics(&registry);
    hara_native::core::with_namespace_registry(&registry, || {
        let mut runtime = Runtime::core();
        assert_eq!(
            runtime
                .eval_native(
                    "(ns example.division-macro)\n\
                 (defmacro capture-division [form] (Base/list 'quote (Base/list form &form)))\n\
                 (capture-division (/ 6 2))",
                )
                .unwrap(),
            "((/ 6 2) (capture-division (/ 6 2)))"
        );
        assert_eq!(runtime.eval_native("(/ 6 2)").unwrap(), "3");
        assert_eq!(runtime.eval_native("(std.foundation// 6 2)").unwrap(), "3");
        assert_eq!(
            runtime
                .eval_native("(capture-division (std.foundation// 6 2))")
                .unwrap(),
            "((std.foundation// 6 2) (capture-division (std.foundation// 6 2)))"
        );
        runtime.register_resource(
            "example.division-owner",
            "(ns example.division-owner) (defn divide [a b] (/ a b))",
        );
        assert_eq!(
            runtime
                .eval_native(
                    "(ns example.division-client (:require [example.division-owner :as owner]))\n\
                 (owner/divide 6 2)",
                )
                .unwrap(),
            "3"
        );
    });
}

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
        .eval_direct_native("(try (throw (ex :test/failure {})) (catch Throwable error :caught))")
        .expect_err("typed host catch syntax must not compile as Hara");
    assert!(
        error.contains(
            "catch expects a binding symbol and one handler form; typed host catch clauses are not part of Hara"
        ),
        "{error}"
    );
}

#[test]
fn direct_native_namespace_require_compiles_runtime_dependencies() {
    let mut runtime = Runtime::core();
    runtime.register_resource(
        "example.direct-namespace-base",
        "(ns example.direct-namespace-base) (defn increment [value] (+ value 1))",
    );
    runtime.register_resource(
        "example.direct-namespace-dependency",
        "(ns example.direct-namespace-dependency (:require [example.direct-namespace-base :as base])) (defn increment [value] (base/increment value))",
    );
    runtime
        .set_execution_backend("direct-native")
        .expect("native builds must expose the direct-native backend");

    assert_eq!(
        runtime
            .eval_native(
                "(ns example.direct-namespace-client (:require [example.direct-namespace-dependency :as dependency]))\n                 (dependency/increment 41)",
            )
            .unwrap(),
        "42"
    );
}

#[test]
fn direct_native_socket_methods_use_the_declared_native_names() {
    let mut runtime = Runtime::core();
    runtime.install_native_socket_provider();
    runtime
        .set_execution_backend("direct-native")
        .expect("native builds must expose the direct-native backend");
    assert_eq!(
        runtime
            .eval_native(concat!(
                "(let [server (Socket/listen \"127.0.0.1\" 0 {} (fn [_] nil)) ",
                "endpoint (Socket/endpoint server) ",
                "stream (Socket/events server {}) ",
                "_ (Socket/close server)] ",
                "[(= \"127.0.0.1\" (:host endpoint)) (< 0 (:port endpoint)) (< 0 stream)])"
            ),)
            .unwrap(),
        "[true true true]"
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
