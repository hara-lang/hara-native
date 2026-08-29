use hara_wasm::Runtime;

#[test]
fn portable_schema_accepts_canonical_and_native_forms() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "(ns typed-schema-rust-probe \
                   (:require [std.typed.schema :as typed])) \
                 (let [primitive (schema :int) \
                       user (schema [:map [:name :str]])] \
                   [(= (typed/normalize :int) (typed/normalize [:int])) \
                    (= (typed/normalize :int) (typed/normalize primitive)) \
                    (typed/valid? [:int] 42) \
                    (typed/valid? [:int] \"42\") \
                    (typed/valid? user {:name \"Ada\"}) \
                    (typed/valid? user {:name 42}) \
                    (typed/compatible? primitive :int)])"
            )
            .unwrap(),
        "[true true true false true false true]"
    );
}

#[test]
fn native_schema_ast_is_the_portable_normal_form() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime
            .eval_native(
                "(ns typed-schema-ast-rust-probe \
                   (:require [std.typed.schema :as typed])) \
                 (defn canonical-ast? [surface] \
                   (let [compiled (schema surface) \
                         normalized (typed/normalize surface) \
                         ast (Schema/ast compiled)] \
                     (and (= normalized ast) \
                          (= ast (typed/normalize ast)) \
                          (= ast (Schema/ast (schema ast)))))) \
                 (let [surfaces \
                       [:int \
                        :vendor/type \
                        (quote [:int {:title \"Age\" :owner :accounts}]) \
                        (quote [:map {:title \"User record\" :version 2 :owner :accounts} [:name {:required true :description \"Display name\" :default \"Anonymous\"} :str]]) \
                        (quote [:or :int :str :int]) \
                        (quote [:vector [:maybe :int]]) \
                        (quote [:str {:min-count 1 :max-count 8 :pattern \"^a\"}]) \
                        (quote [:keyword {:qualified true}]) \
                        (quote [:vector {:min-count 1 :max-count 3 :distinct true} :int]) \
                        (quote [:set {:min-count 1 :max-count 3} :keyword]) \
                        (quote [:tuple :keyword :int :str]) \
                        (quote [:map [:name :str] [:tags [:vector :keyword]]]) \
                        (quote [:map {:closed true} [:id :int] [:nickname {:optional true} :str]]) \
                        (quote [:fn [:str & :any] :str]) \
                        (quote [:function [:fn [:int] :int] \
                                          [:fn [:str & :any] :str]]) \
                        (quote [:enum :must :may]) \
                        (quote [:test/tagged 42]) \
                        (quote [:vendor/vector :int]) \
                        (quote (var demo/Customer))]] \
                   [(every? canonical-ast? surfaces) \
                    (= (typed/normalize \
                        (quote [:map [:name :str] \
                                     [:tags [:vector :keyword]]])) \
                       {:kind :map \
                        :fields \
                        [{:name :name \
                          :type {:kind :primitive :name :str}} \
                         {:name :tags \
                          :type {:kind :vector \
                                 :item {:kind :primitive \
                                        :name :keyword}}}]}) \
                    (= (typed/normalize \
                        (quote [:map {:closed true} \
                                     [:id :int] \
                                     [:nickname {:optional true} :str]])) \
                       {:kind :map \
                        :properties {:closed true} \
                        :fields \
                        [{:name :id \
                          :type {:kind :primitive :name :int}} \
                         {:name :nickname \
                          :properties {:optional true} \
                          :type {:kind :primitive :name :str}}]}) \
                    [(Schema/kind (schema (quote [:or :int :str]))) \
                     (Schema/kind (schema (quote [:fn [:int] :int]))) \
                     (Schema/kind (schema (quote [:set :int]))) \
                     (Schema/kind (schema (quote [:str {:min-count 1}]))) \
                     (Schema/kind \
                      (schema \
                       (quote [:function [:fn [:int] :int] \
                                         [:fn [:str] :str]])))]])"
            )
            .unwrap(),
        "[true true true [:union :fn :set :primitive :function]]"
    );
}
fn registry_runtime() -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .eval_native(
            "(ns typed-registry-rust-probe \
               (:require [std.typed.registry :as registry] \
                         [std.typed.schema :as typed])) \
             (def nodes \
               (registry/local \
                (quote demo) \
                {(quote Node) \
                 (quote [:map \
                         [:value :int] \
                         [:next [:maybe Node]]])})) \
             (def cycle \
               (registry/local \
                (quote cycle) \
                {(quote A) (quote B) \
                 (quote B) (quote A)}))",
        )
        .unwrap();
    runtime
}

#[test]
fn portable_schema_registry_qualifies_names() {
    let mut runtime = registry_runtime();
    assert_eq!(
        runtime
            .eval_native("(registry/qualify nodes (quote Node))")
            .unwrap(),
        "demo/Node"
    );
}

#[test]
fn portable_schema_registry_validates_recursive_success() {
    let mut runtime = registry_runtime();
    assert_eq!(
        runtime
            .eval_native(
                "(typed/valid? \
                   (quote Node) \
                   {:value 1 :next {:value 2 :next nil}} \
                   nodes)"
            )
            .unwrap(),
        "true"
    );
}

#[test]
fn portable_schema_registry_validates_recursive_failure() {
    let mut runtime = registry_runtime();
    assert_eq!(
        runtime
            .eval_native(
                "(typed/valid? \
                   (quote Node) \
                   {:value 1 :next {:value \"two\" :next nil}} \
                   nodes)"
            )
            .unwrap(),
        "false"
    );
}

#[test]
fn portable_schema_registry_reports_unresolved_references() {
    let mut runtime = registry_runtime();
    assert_eq!(
        runtime
            .eval_native("(typed/unresolved-references (quote Node) nodes)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn portable_schema_registry_reports_alias_cycles() {
    let mut runtime = registry_runtime();
    assert_eq!(
        runtime
            .eval_native(
                "(:finding/type \
                   (first (typed/validate (quote A) 1 cycle)))"
            )
            .unwrap(),
        ":std.typed.schema/cyclic-reference"
    );
}
