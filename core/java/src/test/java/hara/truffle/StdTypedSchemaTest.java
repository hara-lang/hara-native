package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdTypedSchemaTest {
  @Test
  public void portableSchemaAcceptsCanonicalAndNativeForms() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[true true true false true false true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns typed-schema-truffle-probe) "
                      + "(require 'std.typed.schema {:reload true}) "
                      + "(let [primitive (std.foundation/schema :int) "
                      + "      user (std.foundation/schema [:map [:name :str]])] "
                      + "  (pr-str "
                      + "    [(= (std.typed.schema/normalize :int) "
                      + "        (std.typed.schema/normalize [:int])) "
                      + "     (= (std.typed.schema/normalize :int) "
                      + "        (std.typed.schema/normalize primitive)) "
                      + "     (std.typed.schema/valid? [:int] 42) "
                      + "     (std.typed.schema/valid? [:int] \"42\") "
                      + "     (std.typed.schema/valid? user {:name \"Ada\"}) "
                      + "     (std.typed.schema/valid? user {:name 42}) "
                      + "     (std.typed.schema/compatible? primitive :int)]))")
              .asString());
    }
  }

  @Test
  public void nativeSchemaAstIsThePortableNormalForm() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[[] true true [:union :fn :set :primitive :function]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns typed-schema-ast-truffle-probe) "
                      + "(require 'std.typed.schema {:reload true}) "
                      + "(defn canonical-ast-failure [surface] "
                      + "  (let [compiled (std.foundation/schema surface) "
                      + "        normalized (std.typed.schema/normalize surface) "
                      + "        ast (Schema/ast compiled) "
                      + "        renormalized (std.typed.schema/normalize ast) "
                      + "        recompiled (std.foundation/schema ast) "
                      + "        recompiled-ast (Schema/ast recompiled) "
                      + "        checks [(= normalized ast) "
                      + "                (= ast renormalized) "
                      + "                (= ast recompiled-ast)]] "
                      + "    (if (every? true? checks) "
                      + "      nil "
                      + "      {:surface surface "
                      + "       :checks checks "
                      + "       :normalized normalized "
                      + "       :ast ast "
                      + "       :renormalized renormalized "
                      + "       :recompiled recompiled-ast}))) "
                      + "(let [surfaces "
                      + "      [:int "
                      + "       :vendor/type "
                      + "       (quote [:int {:title \"Age\" :owner :accounts}]) "
                      + "       (quote [:map {:title \"User record\" :version 2 :owner :accounts} [:name {:required true :description \"Display name\" :default \"Anonymous\"} :str]]) "
                      + "       (quote [:or :int :str :int]) "
                      + "       (quote [:vector [:maybe :int]]) "
                      + "       (quote [:str {:min-count 1 :max-count 8 :pattern \"^a\"}]) "
                      + "       (quote [:keyword {:qualified true}]) "
                      + "       (quote [:vector {:min-count 1 :max-count 3 :distinct true} :int]) "
                      + "       (quote [:set {:min-count 1 :max-count 3} :keyword]) "
                      + "       (quote [:tuple :keyword :int :str]) "
                      + "       (quote [:map [:name :str] [:tags [:vector :keyword]]]) "
                      + "       (quote [:map {:closed true} [:id :int] [:nickname {:optional true} :str]]) "
                      + "       (quote [:fn [:str & :any] :str]) "
                      + "       (quote [:function [:fn [:int] :int] "
                      + "                         [:fn [:str & :any] :str]]) "
                      + "       (quote [:enum :must :may]) "
                      + "       (quote [:test/tagged 42]) "
                      + "       (quote [:vendor/vector :int]) "
                      + "       (quote (var demo/Customer))]] "
                      + "  (pr-str "
                      + "   [(vec (keep canonical-ast-failure surfaces)) "
                      + "    (= (std.typed.schema/normalize "
                      + "        (quote [:map [:name :str] "
                      + "                     [:tags [:vector :keyword]]])) "
                      + "       {:kind :map "
                      + "        :fields "
                      + "        [{:name :name "
                      + "          :type {:kind :primitive :name :str}} "
                      + "         {:name :tags "
                      + "          :type {:kind :vector "
                      + "                 :item {:kind :primitive "
                      + "                        :name :keyword}}}]}) "
                      + "    (= (std.typed.schema/normalize "
                      + "        (quote [:map {:closed true} "
                      + "                     [:id :int] "
                      + "                     [:nickname {:optional true} :str]])) "
                      + "       {:kind :map "
                      + "        :properties {:closed true} "
                      + "        :fields "
                      + "        [{:name :id :type {:kind :primitive :name :int}} "
                      + "         {:name :nickname :properties {:optional true} "
                      + "          :type {:kind :primitive :name :str}}]}) "
                      + "    [(Schema/kind "
                      + "      (std.foundation/schema (quote [:or :int :str]))) "
                      + "     (Schema/kind "
                      + "      (std.foundation/schema (quote [:fn [:int] :int]))) "
                      + "     (Schema/kind "
                      + "      (std.foundation/schema (quote [:set :int]))) "
                      + "     (Schema/kind "
                      + "      (std.foundation/schema (quote [:str {:min-count 1}]))) "
                      + "     (Schema/kind "
                      + "      (std.foundation/schema "
                      + "       (quote [:function [:fn [:int] :int] "
                      + "                         [:fn [:str] :str]])))]]))")
              .asString());
    }
  }
  @Test
  public void portableSchemaRegistryResolvesRecursiveReferences() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[demo/Node true false [] :std.typed.schema/cyclic-reference]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns typed-registry-truffle-probe) "
                      + "(require 'std.typed.registry {:reload true}) "
                      + "(require 'std.typed.schema {:reload true}) "
                      + "(def nodes "
                      + "  (std.typed.registry/local "
                      + "   (quote demo) "
                      + "   {(quote Node) "
                      + "    (quote [:map "
                      + "            [:value :int] "
                      + "            [:next [:maybe Node]]])})) "
                      + "(def cycle "
                      + "  (std.typed.registry/local "
                      + "   (quote cycle) "
                      + "   {(quote A) (quote B) "
                      + "    (quote B) (quote A)})) "
                      + "(pr-str "
                      + " [(std.typed.registry/qualify nodes (quote Node)) "
                      + "  (std.typed.schema/valid? "
                      + "   (quote Node) "
                      + "   {:value 1 :next {:value 2 :next nil}} "
                      + "   nodes) "
                      + "  (std.typed.schema/valid? "
                      + "   (quote Node) "
                      + "   {:value 1 :next {:value \"two\" :next nil}} "
                      + "   nodes) "
                      + "  (std.typed.schema/unresolved-references "
                      + "   (quote Node) nodes) "
                      + "  (:finding/type "
                      + "   (first "
                      + "    (std.typed.schema/validate "
                      + "     (quote A) 1 cycle)))])")
              .asString());
    }
  }
}
