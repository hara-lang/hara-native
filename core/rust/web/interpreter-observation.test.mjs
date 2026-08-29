import assert from "node:assert/strict";
import test from "node:test";
import {
  InterpreterObservationRuntime,
  InterpreterObservationWasmTransport,
} from "./host/interpreter-observation.js";

const createFakeExports = (handler) => {
  const memory = new WebAssembly.Memory({ initial: 2 });
  let cursor = 1024;
  const allocations = [];
  const allocate = (length) => {
    const pointer = cursor;
    cursor += Math.max(1, length) + 16;
    allocations.push([pointer, length]);
    return pointer;
  };
  return {
    memory,
    allocations,
    interpreter_observation_abi_version: () => 1,
    interpreter_observation_alloc: allocate,
    interpreter_observation_dealloc() {},
    interpreter_observation_invoke(pointer, length) {
      const request = JSON.parse(new TextDecoder().decode(
        new Uint8Array(memory.buffer, pointer, length),
      ));
      const response = new TextEncoder().encode(JSON.stringify(handler(request)));
      const responsePointer = allocate(response.byteLength);
      new Uint8Array(memory.buffer, responsePointer, response.byteLength).set(response);
      return (BigInt(responsePointer) << 32n) | BigInt(response.byteLength);
    },
  };
};

const info = ({
  handle = 1,
  sessionId = "lesson",
  sourceId = "example/core.hal",
  generation = 0,
  sequence = 0,
  status = "ready",
  retained = 0,
  dropped = 0,
} = {}) => ({
  handle,
  sessionId,
  sourceId,
  generation,
  sequence,
  status,
  retained,
  dropped,
});

test("interpreter plain-C transport copies responses and preserves stable errors", () => {
  const exports = createFakeExports((request) => request.op === "fail"
    ? { ok: false, error: { code: "interpreter-observation/example", message: "broken" } }
    : { ok: true, value: { operation: request.op } });
  const transport = new InterpreterObservationWasmTransport(exports);
  assert.deepEqual(transport.invoke({ op: "snapshot" }), { operation: "snapshot" });
  assert.throws(
    () => transport.invoke({ op: "fail" }),
    (error) => error.code === "interpreter-observation/example" && error.message === "broken",
  );
  assert.ok(exports.allocations.length >= 4);
});

test("runtime owns interpreter sessions and refreshes lifecycle metadata", () => {
  const requests = [];
  let state = info();
  const invoke = (request) => {
    requests.push(request);
    switch (request.op) {
      case "start":
        state = info({ sessionId: request.sessionId, sourceId: request.sourceId });
        return state;
      case "info":
        return state;
      case "step":
        state = info({ ...state, status: "running", sequence: state.sequence + 1, retained: 1 });
        return { schema: "hal.interpreter-observation-entry/0-alpha", sequence: state.sequence };
      case "run":
        state = info({ ...state, status: "returned", sequence: state.sequence + 4, retained: 5 });
        return { schema: "hal.interpreter-observation-run/0-alpha", steps: 4 };
      case "history":
        return { schema: "hal.interpreter-observation-history/0-alpha", entries: [] };
      case "snapshot":
        return { schema: "hal.interpreter-observation-session/0-alpha", status: state.status };
      case "result-display":
        return "7";
      case "reset":
        state = info({ ...state, generation: state.generation + 1, sequence: 0, status: "ready" });
        return { schema: "hal.interpreter-observation-session/0-alpha", generation: state.generation };
      case "cancel":
        state = info({ ...state, status: "cancelled" });
        return { status: "cancelled" };
      case "dispose":
        return true;
      case "dispose-all":
        return 0;
      default:
        return true;
    }
  };

  const runtime = new InterpreterObservationRuntime({ invoke });
  const session = runtime.startNamed("lesson", "example/core.hal", "(+ 1 (* 2 3))");
  assert.equal(session.status, "ready");
  assert.equal(session.step().schema, "hal.interpreter-observation-entry/0-alpha");
  assert.equal(session.status, "running");
  assert.equal(session.run(100).schema, "hal.interpreter-observation-run/0-alpha");
  assert.equal(session.status, "returned");
  assert.equal(session.resultDisplay(), "7");
  assert.equal(session.reset().generation, 1);
  assert.equal(session.generation, 1);
  assert.equal(session.cancel().status, "cancelled");
  assert.equal(session.status, "cancelled");
  assert.equal(session.dispose(), true);
  assert.throws(() => session.history(), /disposed/);
  assert.equal(requests.some((request) => request.handle === 1), true);
  assert.equal(runtime.dispose(), true);
});

test("settlement and observation limits remain JSON-safe requests", () => {
  const requests = [];
  const invoke = (request) => {
    requests.push(request);
    if (request.op === "start" || request.op === "info") {
      return info({ handle: 3, sessionId: "suspend", sourceId: "suspend.hal", status: "suspended", sequence: 4 });
    }
    if (request.op === "resume") return { schema: "hal.interpreter-observation-entry/0-alpha" };
    if (request.op === "dispose-all") return 1;
    return true;
  };
  const runtime = new InterpreterObservationRuntime({ invoke });
  const session = runtime.startNamed("suspend", "suspend.hal", "(Coroutine/await pending)");
  session.setObservationLimits({ bindings: 12, displayChars: 256 });
  session.setRetentionLimits({ history: 24 });
  session.resume({ status: "fulfilled", value: { answer: 42 } });
  assert.deepEqual(
    requests.find((request) => request.op === "resume").settlement,
    { status: "fulfilled", value: { answer: 42 } },
  );
  runtime.dispose();
});
