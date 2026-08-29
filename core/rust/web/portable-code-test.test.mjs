import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import init, { Runtime } from "../../../website/hara-www/docs/rust/pkg/hara_wasm.js";

const moduleBytes = await readFile(
  new URL("../../../website/hara-www/docs/rust/pkg/hara_wasm_bg.wasm", import.meta.url),
);
await init({ module_or_path: moduleBytes });

test("portable code.test runs through the browser wasm runtime", () => {
  const runtime = new Runtime();
  const result = runtime.eval(
    '(ns code.test-browser-probe (:use code.test))' +
      " (def lifecycle (atom []))" +
      ' (fact "promise assertion"' +
      "   {:before (fn []" +
      "              (swap! lifecycle" +
      "                     (fn [events] (conj events :before))))" +
      "    :after (fn []" +
      "             (swap! lifecycle" +
      "                    (fn [events] (conj events :after))))}" +
      "   (promise/from 42) => 42" +
      "   (+ 1 1) => 2)" +
      ' (let [summary (run {:namespace "code.test-browser-probe"})' +
      "       timed (check (fn [] (promise/from 42)) 42" +
      "                    {:work/timeout-promise" +
      "                     (fn [promise milliseconds]" +
      "                       {:promise (promise/from {:test/status :timeout})" +
      "                        :timeout milliseconds})" +
      "                     :work/cancel-timeout (fn [timeout] timeout)" +
      "                     :timeout 25})]" +
      " [(:status summary)" +
      "  (:passed (:counts summary))" +
      "  (count (:checks (first (:results summary))))" +
      "  (:status timed)" +
      "  (:timeout timed)])",
  );

  assert.equal(result, "[:passed 1 2 :timeout 25]");
});

test("canonical context libraries run through browser wasm", () => {
  const runtime = new Runtime();
  const result = runtime.eval(
    "(ns std-context-browser-probe" +
      " (:require [std.context :as context]" +
      "           [std.lib.context :as legacy]))" +
      " (let [runtime (context/runtime-null)" +
      "       legacy-runtime (legacy/runtime-null)]" +
      " [(context/call runtime :a :b)" +
      "  (instance? context/runtime-null-type legacy-runtime)" +
      "  (legacy/call legacy-runtime :c)])",
  );

  assert.equal(result, "[[:a :b] true [:c]]");
});

test("portable command templates emit structured reports through browser wasm", () => {
  const runtime = new Runtime();
  const result = runtime.eval(
    "(ns std-work-command-browser-probe" +
      " (:require [work.base :as work] [work.flow.task.command :as command]))" +
      " (def double-command" +
      "  (command/single {:id :probe/double :version 1}" +
      "   {:process (work/pure :probe/process" +
      "              (fn [value context] (* 2 value)))}))" +
      " (let [observer (work/recording-observer)" +
      "       host (work/local-runtime {:observer observer})" +
      "       output (work/run host double-command 4)]" +
      " [output" +
      "  (:op (work/work-spec double-command))" +
      "  (count (filter (fn [event]" +
      "                   (= :command/completed (:event event)))" +
      "                 (work/observer-events observer)))])",
  );

  assert.equal(
    result,
    "[8 :chain 1]",
  );
});

test("portable blocks preserve source, value, and structure through browser wasm", () => {
  const runtime = new Runtime();
  const result = runtime.eval(
    "(ns std-block-browser-probe" +
      " (:require [std.block :as block]))" +
      ' (let [parsed (block/parse-string "[1 2 3]")' +
      '       first-block (block/parse-first "[1 2 3]")' +
      "       spaces (block/spaces 3)]" +
      " [(block/string parsed)" +
      "  (block/value parsed)" +
      "  (block/type first-block)" +
      "  (block/tag first-block)" +
      "  (vec (map block/value" +
      "            (filter block/code? (block/children first-block))))" +
      "  (block/string spaces)" +
      "  (block/space? spaces)])",
  );

  assert.equal(
    result,
    '["[1 2 3]" [1 2 3] :container :vector [1 2 3] "   " true]',
  );
});
