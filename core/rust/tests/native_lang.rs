use hara_native::Runtime;

#[test]
fn evaluated_quotes_retain_live_callbacks_and_records() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        assert_eq!(runtime.eval_native(
            "(def captured (fn [value] (+ value 1)))
             (def held (Runtime/eval (Base/list 'quote {:callback captured})))
             [(= captured (:callback held)) ((:callback held) 41)]"
        ).unwrap(), "[true 42]", "{backend}: quoted callback identity");
        runtime.eval_native(
            "(defmacro carry [] (Base/list 'quote {:callback captured}))"
        ).unwrap();
        runtime.eval_native(
            "(def macro-held (carry))
             (def later (Runtime/eval
               (Base/list 'fn [] (Base/list 'quote {:callback captured}))))
             (def captured nil)"
        ).unwrap();
        assert_eq!(runtime.eval_native(
            "[((:callback (later)) 41) ((:callback macro-held) 41)]"
        ).unwrap(), "[42 42]", "{backend}: closures and expansions own their literals");
        assert_eq!(runtime.eval_native(
            "(def QuotedRecord (Base/struct (Base/current-namespace) 'QuotedRecord (Base/vector 'value) nil))
             (def original (QuotedRecord 7))
             (def quoted (Runtime/eval (Base/list 'quote original)))
             [(Base/type quoted) (Base/identical? original quoted) (:value quoted)]"
        ).unwrap(), "[:user.QuotedRecord true 7]", "{backend}: quoted record identity");
    }
}

#[test]
fn process_local_quoted_literals_are_owned() {
    use hara_native::core::{native_function, Value};
    use hara_native::kernel::Form;
    use hara_native::lang::data::metadata::RuntimeMetadata;
    use std::rc::Rc;

    let payload = Rc::new(42);
    let weak = Rc::downgrade(&payload);
    let callback = native_function("held", 0, move |_| Ok(Value::Number(*payload)));
    let quoted = Form::List(vec![
        Form::Symbol("quote".into()),
        Form::RuntimeLiteral(RuntimeMetadata::new(callback)),
    ]);
    let copy = quoted.clone();
    drop(quoted);
    assert!(weak.upgrade().is_some());
    drop(copy);
    assert!(weak.upgrade().is_none());
}

#[test]
fn intrinsic_values_preserve_foundation_function_identity() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        assert_eq!(runtime.eval_native("[(= * *) (= - -) (= * -)]").unwrap(),
                   "[true true false]", "{backend}: repeated intrinsic values");
        runtime.eval_native(
            "(ns identity.owner)
             (defn classify [tag] [(= tag *) (= tag -)])
             (ns identity.caller)"
        ).unwrap();
        assert_eq!(runtime.eval_native(
            "[(identity.owner/classify *) (identity.owner/classify -)
              (= * (IDeref/deref (var std.foundation/*)))
              (= (fn [] 1) (fn [] 1))]"
        ).unwrap(), "[[true false] [false true] true false]", "{backend}: shared Var identity");
    }
}

#[test]
fn local_values_shadow_foundation_macros_without_falling_back() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native(
            "(ns std.foundation)
             (defmacro shadowed [value] (Base/list 'quote :prelude))
             (ns macro.shadow)
             (defn shadowed [value] (+ value 1))"
        ).unwrap();
        assert_eq!(runtime.eval_native("(shadowed 41)").unwrap(), "42", "{backend}");
        assert_eq!(runtime.eval_native(
            "[(Runtime/macroexpand-1 '(shadowed 41)) (std.foundation/shadowed 41)]"
        ).unwrap(), "[(shadowed 41) :prelude]", "{backend}");
        runtime.eval_native(
            "(ns macro.referral (:require [macro.shadow :refer [shadowed]]))"
        ).unwrap();
        assert_eq!(runtime.eval_native("(shadowed 9)").unwrap(), "10", "{backend}: referral");
        runtime.eval_native(
            "(ns macro.shadow) (def ^{:macro false} shadowed nil)"
        ).unwrap();
        assert_eq!(runtime.eval_native("(Runtime/macroexpand-1 '(shadowed 41))").unwrap(),
                   "(shadowed 41)", "{backend}: nonfunction shadow");
        runtime.eval_native("(ns macro.fallback)").unwrap();
        assert_eq!(runtime.eval_native("(shadowed 41)").unwrap(), ":prelude", "{backend}: absent name");
        runtime.register_resource(
            "macro.compiled",
            "(ns macro.compiled (:config {:override [shadowed]}))
             (defn shadowed [value] (+ value 1))
             (defn use-local [] (shadowed 41))",
        );
        assert_eq!(runtime.eval_native(
            "(ns macro.compiled-client (:require [macro.compiled :as target]))
             (target/use-local)"
        ).unwrap(), "42", "{backend}: compiled namespace owns its function");
        runtime.register_resource(
            "macro.compiled-value",
            "(ns macro.compiled-value (:config {:override [shadowed]}))
             (def shadowed (fn [value] (+ value 2)))
             (defn use-local [] [(shadowed 40) (macro.compiled-value/shadowed 40)
                                (std.foundation/shadowed 40)])",
        );
        assert_eq!(runtime.eval_native(
            "(ns macro.value-client (:require [macro.compiled-value :as target]))
             (target/use-local)"
        ).unwrap(), "[42 42 :prelude]", "{backend}: compiled value and qualified calls");
    }
}

#[test]
fn runtime_macro_vars_follow_live_roots_and_metadata() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native("(ns macro.live) (def ^{:macro false} slot nil)").unwrap();
        assert_eq!(runtime.eval_native(
            "(Runtime/intern 'macro.live
               (IObjType/with-meta 'slot {:macro true :arglists '([value])})
               (fn [form environment value] (Base/list 'quote value)))
             [(Runtime/macroexpand-1 '(macro.live/slot (+ 1 2)))
              (Runtime/eval '(macro.live/slot (+ 1 2)))]"
        ).unwrap(), "[(quote (+ 1 2)) (+ 1 2)]", "{backend}");
        assert_eq!(runtime.eval_native(
            "(Runtime/intern 'macro.live (IObjType/with-meta 'slot {:macro false}) nil)
             [(IDeref/deref (var slot)) (:macro (IObjType/meta (var slot)))
              (Runtime/macroexpand-1 '(macro.live/slot (+ 1 2)))]"
        ).unwrap(), "[nil false (macro.live/slot (+ 1 2))]", "{backend}");
        assert_eq!(runtime.eval_native(
            "(Base/def (Runtime/ns-find 'macro.live) 'slot
               (fn [form environment value] (Base/list 'quote value)) {:macro true})
             (Base/def (Runtime/ns-find 'macro.live) 'slot nil {:macro false})
             (Runtime/macroexpand-1 '(macro.live/slot (+ 1 2)))"
        ).unwrap(), "(macro.live/slot (+ 1 2))", "{backend}");
        assert_eq!(runtime.eval_native(
            "(Runtime/intern 'macro.live (IObjType/with-meta 'slot {:macro true}) 1)
             [(IDeref/deref (var slot)) (:macro (IObjType/meta (var slot)))
              (Runtime/macroexpand-1 '(macro.live/slot (+ 1 2)))]"
        ).unwrap(), "[1 true (macro.live/slot (+ 1 2))]", "{backend}");
    }
}

#[test]
fn runtime_macro_vars_follow_scoped_evaluation_namespace() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native(
            "(ns macro.scoped)
             (Base/def (Runtime/ns-find 'macro.scoped) 'quoted
               (fn [form environment value] (Base/list 'quote value)) {:macro true})
             (ns macro.caller)"
        ).unwrap();
        assert_eq!(runtime.eval_native(
            "(let [before (Runtime/current)
                   expanded (Runtime/eval-in 'macro.scoped
                              (fn [] (Runtime/macroexpand-1 '(quoted (+ 1 2)))))]
               [expanded (= before (Runtime/current))])"
        ).unwrap(), "[(quote (+ 1 2)) true]", "{backend}");
        assert_eq!(runtime.eval_native(
            "(Base/def (Runtime/ns-find 'macro.scoped) 'broken
               (fn [form environment] (throw :macro-failed)) {:macro true})
             (let [before (Runtime/current)
                   result (try
                            (Runtime/eval-in 'macro.scoped
                              (fn [] (Runtime/macroexpand-1 '(broken))))
                            (catch error :caught))]
               [result (= before (Runtime/current))])"
        ).unwrap(), "[:caught true]", "{backend}: failed expansion restores namespace");
        assert_eq!(runtime.eval_native(
            "(ns macro.referred (:require [macro.scoped :refer [quoted]]))
             (Base/def (Runtime/ns-find 'macro.scoped) 'quoted nil {:macro false})
             (Runtime/macroexpand-1 '(macro.referred/quoted (+ 1 2)))"
        ).unwrap(), "(macro.referred/quoted (+ 1 2))", "{backend}");
    }
}

#[test]
fn runtime_intern_binds_live_values_without_copying_a_source_var() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native("(ns intern.contract) (def ^{:doc \"baseline\"} slot nil) (def source 19)").unwrap();
        assert_eq!(runtime.eval_native(
            "(let [offset 40 value {:callback (fn [x] (+ offset x)) :source (var source)}]
               (try
                 (let [published (Runtime/intern 'intern.contract 'slot value)]
                   [(= published (var slot))
                    (Base/identical? value (IDeref/deref published))
                    ((:callback (IDeref/deref published)) 2)
                    (= (:source (IDeref/deref published)) (var source))])
                 (finally (Runtime/intern 'intern.contract 'slot nil))))"
        ).unwrap(), "[true true 42 true]", "{backend}");
        assert_eq!(runtime.eval_native("slot").unwrap(), "nil");
        runtime.eval_native(
            "(ns intern.referrer (:require [intern.contract :refer [source]]))
             (Runtime/intern 'intern.referrer 'source 88)"
        ).unwrap();
        assert_eq!(runtime.eval_native(
            "[(IDeref/deref (var intern.referrer/source))
              (IDeref/deref (var intern.contract/source))]"
        ).unwrap(), "[88 19]", "{backend}: does not mutate a referred source Var");
        runtime.eval_native("(ns intern.contract)").unwrap();
        assert_eq!(runtime.eval_native(
            "(let [before (IObjType/meta (var slot))]
               (try
                 (let [published (Runtime/intern 'intern.contract
                         (IObjType/with-meta 'slot {:doc \"script entry\" :arglists '([x]) :lang :lua}) 42)
                       tagged (IObjType/meta published)]
                   (Runtime/intern 'intern.contract 'slot 43)
                   [(:doc tagged) (:arglists tagged) (:lang tagged)
                    (= tagged (IObjType/meta published))])
                 (finally (Runtime/intern 'intern.contract
                            (IObjType/with-meta 'slot before) nil))))"
        ).unwrap(), "[\"script entry\" ([x]) :lua true]", "{backend}: symbol metadata survives interning");
        assert_eq!(runtime.eval_native("(:doc (IObjType/meta (var slot)))").unwrap(), "\"baseline\"");
        assert_eq!(runtime.eval_native(
            "(try (let [published (Runtime/intern 'intern.contract 'slot (var source))]
                    [(= (IDeref/deref published) (var source))
                     (IDeref/deref (IDeref/deref published))])
                  (finally (Runtime/intern 'intern.contract 'slot nil)))"
        ).unwrap(), "[true 19]", "{backend}: Var is a value, not a copy request");
        assert_eq!(runtime.eval_native(
            "(try (Runtime/intern 'intern.contract 'slot 42)
                  (Runtime/intern 'intern.contract 'slot 43)
                  (IDeref/deref (var slot))
                  (finally (Runtime/intern 'intern.contract 'slot nil)))"
        ).unwrap(), "43", "{backend}: replacement");
        assert!(runtime.eval_native("(Runtime/intern 'intern.contract 'other/slot 1)").unwrap_err().contains("unqualified target symbol"));
        assert!(runtime.eval_native("(Runtime/intern 'intern.contract :slot 1)").unwrap_err().contains("unqualified target symbol"));
        assert!(runtime.eval_native("(Runtime/intern 'intern.contract 'slot)").unwrap_err().contains("expects namespace, symbol, and value"));
        assert!(runtime.eval_native("(Runtime/intern-var 'intern.contract 'slot 1)").unwrap_err().contains("source Var"));
        assert_eq!(runtime.eval_native("slot").unwrap(), "nil");
    }
}

#[test]
fn macro_invocation_preserves_call_metadata() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native("(defmacro capture-call-meta [] (Base/list 'quote (IObjType/meta &form)))").unwrap();
        assert_eq!(runtime.eval_native("^{:- :raw} (capture-call-meta)").unwrap(),
                   "{:- :raw}", "{backend}: direct call");
        assert_eq!(runtime.eval_native("(let [x 1] ^{:- :raw} (capture-call-meta))").unwrap(),
                   "{:- :raw}", "{backend}: nested call");
        runtime.eval_native("(defn captured-call [] ^{:- :raw} (capture-call-meta))").unwrap();
        assert_eq!(runtime.eval_native("(captured-call)").unwrap(),
                   "{:- :raw}", "{backend}: function body");
        assert_eq!(runtime.eval_native("(capture-call-meta)").unwrap(),
                   "nil", "{backend}: metadata does not leak");
        assert_eq!(runtime.eval_native("(Runtime/macroexpand-1 '^{:- :raw} (capture-call-meta))").unwrap(),
                   "(quote {:- :raw})", "{backend}: explicit expansion");
    }
}

#[test]
fn cache_identity_distinguishes_equal_maps_and_records() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native("(def IdentityEntry (Base/struct (Base/current-namespace) 'IdentityEntry (Base/vector 'value) nil))").unwrap();
        assert_eq!(runtime.eval_native(
            "(let [a {:value 1} b {:value 1} r (IdentityEntry 1) s (IdentityEntry 1)
                   saved (Base/atom [a r])]
               [(Base/identical? a a) (Base/identical? a b) (= a b)
                (Base/identical? r r) (Base/identical? r s) (= r s)
                (Base/identical? a (INth/nth (IDeref/deref saved) 0))
                (Base/identical? r (INth/nth (IDeref/deref saved) 1))
                (Base/identical? a (IObjType/with-meta a {:doc \"new\"}))
                (Base/identical? a (IAssoc/assoc a :value 2))
                (Base/identical? a r)])"
        ).unwrap(), "[true false true true false true true true false false false]", "{backend}");
        assert!(runtime.eval_native("(Base/identical? 1 1)").unwrap_err().contains("expects map or record objects"));
        assert!(runtime.eval_native("(Base/identical? {})").unwrap_err().contains("expects two objects"));
    }
}

#[test]
fn iobjtype_metadata_retains_live_records_and_callbacks() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native("(def ParentSnapshot (Base/struct (Base/current-namespace) 'ParentSnapshot (Base/vector 'book) nil))").unwrap();
        assert_eq!(runtime.eval_native(
            "(let [offset 40 callback (fn [x] (+ offset x))
                   parent (ParentSnapshot {:emit callback})
                   original {:value 1}
                   child (IObjType/with-meta original {:parent parent :assign/fn callback})
                   metadata (IObjType/meta child)
                   restored (:parent metadata)]
               [(= parent restored) (= (Base/type parent) (Base/type restored))
                ((:emit (:book restored)) 2) ((:assign/fn metadata) 3)
                (IObjType/meta original) (= child original)
                (IObjType/meta (IObjType/with-meta child nil))
                (IObjType/meta (IObjType/with-meta child {:doc \"replaced\"}))])"
        ).unwrap(), "[true true 42 43 nil true nil {:doc \"replaced\"}]", "{backend}");
        assert_eq!(runtime.eval_native(
            "(let [callback (fn [sym] (Base/list sym :as [1 2 3]))
                   form (IObjType/with-meta '(sym :as [1 2 3]) {:assign/fn callback})]
               [((:assign/fn (IObjType/meta form)) 'a)
                (= form '(sym :as [1 2 3]))])"
        ).unwrap(), "[(a :as [1 2 3]) true]", "{backend}");
    }
}

#[test]
fn process_local_metadata_is_owned_and_not_serialized() {
    use hara_native::lang::data::{Keyword, Metadata, MetadataValue};
    use hara_native::lang::data::metadata::RuntimeMetadata;
    use std::rc::Rc;

    let payload = Rc::new(42);
    let weak = Rc::downgrade(&payload);
    let metadata = Metadata::new(vec![(
        MetadataValue::Keyword(Keyword::from("callback")),
        MetadataValue::Runtime(RuntimeMetadata::new(payload)),
    )]);
    let copy = metadata.clone();
    let mut program = hara_native::vm::compile_source("42").unwrap();
    program.var_metadata = vec![metadata];
    assert_eq!(hara_native::vm::encode_program(&program).unwrap_err(),
               "cannot serialize process-local metadata");
    drop(program);
    assert_eq!(**copy.get_keyword("callback").and_then(|value| match value {
        MetadataValue::Runtime(value) => value.get::<Rc<i32>>(),
        _ => None,
    }).unwrap(), 42);
    assert!(weak.upgrade().is_some());
    drop(copy);
    assert!(weak.upgrade().is_none());

    let portable = Metadata::document("portable metadata");
    let mut program = hara_native::vm::compile_source("42").unwrap();
    program.var_metadata = vec![portable.clone()];
    let encoded = hara_native::vm::encode_program(&program).unwrap();
    let restored = hara_native::vm::decode_program(&encoded).unwrap();
    assert_eq!(restored.var_metadata[0], portable);
    assert_eq!(hara_native::vm::encode_program(&restored).unwrap(), encoded);
}

#[test]
fn scoped_eval_preserves_captures_lexical_resolution_and_dynamic_namespace() {
    for backend in ["interpreter", "direct-native"] {
        let mut runtime = Runtime::core();
        runtime.set_execution_backend(backend).unwrap();
        runtime.eval_native("(ns scope.target) (def value 99) (ns scope.inner) (def value 100) (ns scope.caller) (def value 7) (defn observe [] [(Runtime/current) value (Runtime/eval 'value)])").unwrap();
        assert_eq!(runtime.eval_native(
            "(let [captured (Base/atom 42)]
               [(Runtime/eval-in 'scope.target
                  (fn [] [(IDeref/deref captured) (observe)
                          (Runtime/eval-in 'scope.inner (fn [] (observe)))
                          (observe)]))
                (Runtime/current) value])"
        ).unwrap(), "[[42 [scope.target 7 99] [scope.inner 7 100] [scope.target 7 99]] scope.caller 7]", "{backend}");
        assert_eq!(runtime.eval_native("[(Runtime/eval-in 'scope.target (fn [] nil)) (Runtime/eval-in 'scope.target (fn [] false)) (Runtime/current)]").unwrap(), "[nil false scope.caller]", "{backend}");
        assert!(runtime.eval_native("(Runtime/eval-in 'scope.target (fn [x] x))").is_err());
        assert_eq!(runtime.eval_native("(Runtime/current)").unwrap(), "scope.caller");
        assert_eq!(runtime.eval_native("(Runtime/eval-in 'scope.target (fn [] [(try (Runtime/eval-in 'scope.inner (fn [] (throw :scope-failed))) (catch e :caught)) (Runtime/current)]))").unwrap(), "[:caught scope.target]", "{backend}");
        assert_eq!(runtime.eval_native("(Runtime/current)").unwrap(), "scope.caller");
        assert!(runtime.eval_native("(Runtime/eval-in 'scope.target (fn [] (throw :scope-failed)))").is_err());
        assert_eq!(runtime.eval_native("(Runtime/current)").unwrap(), "scope.caller");
        assert_eq!(runtime.eval_native("(Runtime/eval-in 'scope.target '[value])").unwrap(), "99");
        assert!(runtime.eval_native("(let [isolated 42] (Runtime/eval-in 'scope.target '[isolated]))").is_err());
        assert_eq!(runtime.eval_native("(Runtime/current)").unwrap(), "scope.caller");
    }
}

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
fn direct_native_eval_preserves_dynamic_definition_metadata() {
    let mut runtime = Runtime::core();
    runtime.set_execution_backend("direct-native").unwrap();
    assert_eq!(
        runtime.eval_native(
            "(Runtime/eval '(def ^:dynamic *eval-metadata* 7))
             [(:dynamic (IObjType/meta #'*eval-metadata*))
              (binding [*eval-metadata* 11] *eval-metadata*)
              *eval-metadata*]"
        ).unwrap(),
        "[true 11 7]"
    );
}

#[test]
fn direct_native_eval_in_preserves_dynamic_definition_metadata() {
    let mut runtime = Runtime::core();
    runtime.set_execution_backend("direct-native").unwrap();
    assert_eq!(
        runtime.eval_native(
            "(ns example.eval-in-metadata)
             (Runtime/eval-in 'example.eval-in-metadata
               '[(def ^:dynamic *scoped-metadata* 7)])
             [(:dynamic (IObjType/meta #'*scoped-metadata*))
              (binding [*scoped-metadata* 11] *scoped-metadata*)
              *scoped-metadata*
              (Runtime/eval-in 'example.eval-in-metadata [])]"
        ).unwrap(),
        "[true 11 7 nil]"
    );
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
