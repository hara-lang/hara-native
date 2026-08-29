import {
  JsonWasmTransport,
  instantiateJsonWasm,
  objectValue,
} from "./json-wasm-transport.js";

const MAX_SAFE_INTEGER = 9_007_199_254_740_991;
const DEFAULT_WASM_PATH = "../interpreter-observation.wasm";

const stringValue = (value, label) => {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new TypeError(`${label} must be a non-empty string`);
  }
  return value;
};

const integerValue = (value, label, maximum = MAX_SAFE_INTEGER) => {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) {
    throw new RangeError(`${label} must be an integer between 0 and ${maximum}`);
  }
  return value;
};

const settlementValue = (value) => {
  if (value == null) return null;
  const settlement = objectValue(value, "interpreter settlement");
  const status = stringValue(settlement.status, "interpreter settlement status");
  if (status === "fulfilled") return { status, value: settlement.value ?? null };
  if (status === "rejected") return { status, error: settlement.error ?? null };
  if (status === "pending") return { status };
  throw new TypeError(`unsupported interpreter settlement status: ${status}`);
};

const sessionInfo = (value) => {
  const info = objectValue(value, "interpreter observation session info");
  return Object.freeze({
    handle: integerValue(info.handle, "interpreter observation handle"),
    sessionId: stringValue(info.sessionId, "interpreter session id"),
    sourceId: stringValue(info.sourceId, "interpreter source id"),
    generation: integerValue(info.generation, "interpreter session generation"),
    sequence: integerValue(info.sequence, "interpreter session sequence"),
    status: stringValue(info.status, "interpreter session status"),
    retained: integerValue(info.retained, "interpreter retained history"),
    dropped: integerValue(info.dropped, "interpreter dropped history"),
  });
};

/**
 * Plain-C JSON transport for the on-demand authoritative interpreter
 * observation module. Responses are copied before guest memory is released;
 * no Runtime, fiber, Value, Promise, pointer, or handle escapes this boundary.
 */
export class InterpreterObservationWasmTransport extends JsonWasmTransport {
  constructor(exports) {
    super(exports, {
      label: "interpreter observation Wasm",
      requestLabel: "interpreter observation request",
      errorCode: "interpreter-observation/error",
      names: {
        abiVersion: "interpreter_observation_abi_version",
        alloc: "interpreter_observation_alloc",
        dealloc: "interpreter_observation_dealloc",
        invoke: "interpreter_observation_invoke",
      },
    });
  }
}

export async function loadInterpreterObservationRuntime({
  wasmUrl = new URL(DEFAULT_WASM_PATH, import.meta.url),
  wasmBytes = null,
  fetchImpl = globalThis.fetch,
  imports = {},
} = {}) {
  const instance = await instantiateJsonWasm({
    wasmUrl,
    wasmBytes,
    fetchImpl,
    imports,
    label: "interpreter observation Wasm",
    fetchDescription: "the interpreter observation Wasm module",
    loadDescription: "interpreter observation Wasm",
  });
  const transport = new InterpreterObservationWasmTransport(instance.exports);
  return new InterpreterObservationRuntime({ invoke: transport.invoke.bind(transport) });
}

/** Owns one interpreter observation Wasm instance and all sessions inside it. */
export class InterpreterObservationRuntime {
  constructor({ invoke }) {
    if (typeof invoke !== "function") {
      throw new TypeError("InterpreterObservationRuntime requires an invoke function");
    }
    this.invoke = invoke;
    this.sessions = new Set();
    this.nextSessionId = 1;
    this.disposed = false;
  }

  start(source, { sessionId, sourceId } = {}) {
    const id = this.nextSessionId++;
    return this.startNamed(
      sessionId ?? `interpreter/session-${id}`,
      sourceId ?? `interpreter/session-${id}.hal`,
      source,
    );
  }

  startNamed(sessionId, sourceId, source) {
    this.assertActive();
    const info = this.invoke({
      op: "start",
      sessionId: stringValue(sessionId, "interpreter session id"),
      sourceId: stringValue(sourceId, "interpreter source id"),
      source: stringValue(source, "interpreter source"),
    });
    return this.track(info);
  }

  dispose() {
    if (this.disposed) return false;
    this.disposed = true;
    this.invoke({ op: "dispose-all" });
    for (const session of this.sessions) session.markDisposed();
    this.sessions.clear();
    return true;
  }

  assertActive() {
    if (this.disposed) throw new Error("interpreter observation runtime is disposed");
  }

  track(info) {
    const session = new InterpreterObservationSession(this, sessionInfo(info));
    this.sessions.add(session);
    return session;
  }

  forget(session) {
    this.sessions.delete(session);
  }
}

export class InterpreterObservationSession {
  constructor(runtime, info) {
    this.runtime = runtime;
    this.info = info;
    this.disposed = false;
  }

  get handle() { return this.info.handle; }
  get sessionId() { return this.info.sessionId; }
  get sourceId() { return this.info.sourceId; }
  get generation() { return this.info.generation; }
  get sequence() { return this.info.sequence; }
  get status() { return this.info.status; }
  get retained() { return this.info.retained; }
  get dropped() { return this.info.dropped; }

  snapshot() { return this.call("snapshot"); }
  history() { return this.call("history"); }

  step() {
    const evidence = this.call("step");
    this.refresh();
    return evidence;
  }

  run(boundaryLimit) {
    const evidence = this.call("run", {
      boundaryLimit: integerValue(boundaryLimit, "interpreter run boundary limit", 100_000),
    });
    this.refresh();
    return evidence;
  }

  resume(settlement = null) {
    const evidence = this.call("resume", { settlement: settlementValue(settlement) });
    this.refresh();
    return evidence;
  }

  resolveSuspension(value) { return this.call("resolve-suspension", { value }); }
  rejectSuspension(error) { return this.call("reject-suspension", { error }); }
  suspensionState() { return this.call("suspension-state"); }

  reset() {
    const snapshot = this.call("reset");
    this.refresh();
    return snapshot;
  }

  cancel() {
    const snapshot = this.call("cancel");
    this.refresh();
    return snapshot;
  }

  resultDisplay() { return this.call("result-display"); }
  errorMessage() { return this.call("error-message"); }

  setObservationLimits({ bindings, displayChars }) {
    this.assertActive();
    return this.runtime.invoke({
      op: "set-observation-limits",
      handle: this.handle,
      bindings: integerValue(bindings, "interpreter binding limit", 4_096),
      displayChars: integerValue(displayChars, "interpreter display character limit", 16_384),
    });
  }

  setRetentionLimits({ history }) {
    this.assertActive();
    return this.runtime.invoke({
      op: "set-retention-limits",
      handle: this.handle,
      history: integerValue(history, "interpreter history limit", 100_000),
    });
  }

  refresh() {
    this.assertActive();
    this.info = sessionInfo(this.runtime.invoke({ op: "info", handle: this.handle }));
    return this.info;
  }

  dispose() {
    if (this.disposed) return false;
    const disposed = this.call("dispose");
    this.markDisposed();
    this.runtime.forget(this);
    return disposed;
  }

  markDisposed() {
    this.disposed = true;
    this.info = Object.freeze({ ...this.info, status: "disposed" });
  }

  assertActive() {
    this.runtime.assertActive();
    if (this.disposed) throw new Error("interpreter observation session is disposed");
  }

  call(op, fields = {}) {
    this.assertActive();
    return this.runtime.invoke({ op, handle: this.handle, ...fields });
  }
}
