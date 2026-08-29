import {
  JsonWasmTransport,
  byteArray,
  instantiateJsonWasm,
  objectValue,
} from "./json-wasm-transport.js";

const MAX_SAFE_INTEGER = 9_007_199_254_740_991;
const DEFAULT_WASM_PATH = "../bytecode-observation.wasm";

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
  const settlement = objectValue(value, "bytecode settlement");
  const status = stringValue(settlement.status, "bytecode settlement status");
  if (!new Set(["pending", "fulfilled", "rejected"]).has(status)) {
    throw new TypeError(`unsupported bytecode settlement status: ${status}`);
  }
  if (status === "fulfilled") return { status, value: settlement.value ?? null };
  if (status === "rejected") return { status, error: settlement.error ?? null };
  return { status };
};

const sessionInfo = (value) => {
  const info = objectValue(value, "bytecode observation session info");
  return Object.freeze({
    handle: integerValue(info.handle, "bytecode observation handle"),
    sessionId: stringValue(info.sessionId, "bytecode session id"),
    sourceId: stringValue(info.sourceId, "bytecode source id"),
    traceId: stringValue(info.traceId, "bytecode trace id"),
    status: stringValue(info.status, "bytecode session status"),
    sequence: integerValue(info.sequence, "bytecode session sequence"),
  });
};

/**
 * Plain-C JSON transport for the on-demand bytecode observation Wasm module.
 * It copies every response before releasing Wasm memory and never exposes a
 * machine pointer, guest value, promise or runtime allocation to application
 * state.
 */
export class BytecodeObservationWasmTransport extends JsonWasmTransport {
  constructor(exports) {
    super(exports, {
      label: "bytecode observation Wasm",
      requestLabel: "bytecode observation request",
      errorCode: "bytecode-observation/error",
      names: {
        abiVersion: "observation_abi_version",
        alloc: "observation_alloc",
        dealloc: "observation_dealloc",
        invoke: "observation_invoke",
      },
    });
  }
}

export async function loadBytecodeObservationRuntime({
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
    label: "bytecode observation Wasm",
    fetchDescription: "the bytecode observation Wasm module",
    loadDescription: "bytecode observation Wasm",
  });
  const transport = new BytecodeObservationWasmTransport(instance.exports);
  return new BytecodeObservationRuntime({ invoke: transport.invoke.bind(transport) });
}

/**
 * Owns one on-demand observation Wasm instance and all opaque sessions created
 * inside it. Normal Hara evaluation does not instantiate this module.
 */
export class BytecodeObservationRuntime {
  constructor({ invoke }) {
    if (typeof invoke !== "function") {
      throw new TypeError("BytecodeObservationRuntime requires an invoke function");
    }
    this.invoke = invoke;
    this.sessions = new Set();
    this.nextSessionId = 1;
    this.disposed = false;
  }

  compile(source, { sessionId, sourceId } = {}) {
    const id = this.nextSessionId++;
    return this.compileNamed(
      sessionId ?? `bytecode/session-${id}`,
      sourceId ?? `bytecode/session-${id}.hal`,
      source,
    );
  }

  compileNamed(sessionId, sourceId, source) {
    this.assertActive();
    const info = this.invoke({
      op: "compile",
      sessionId: stringValue(sessionId, "bytecode session id"),
      sourceId: stringValue(sourceId, "bytecode source id"),
      source: stringValue(source, "bytecode source"),
    });
    return this.track(info);
  }

  fromArtifact(artifact, { sessionId, sourceId } = {}) {
    const id = this.nextSessionId++;
    return this.fromNamedArtifact(
      sessionId ?? `bytecode/session-${id}`,
      sourceId ?? `bytecode/session-${id}.hbc`,
      artifact,
    );
  }

  fromNamedArtifact(sessionId, sourceId, artifact) {
    this.assertActive();
    const info = this.invoke({
      op: "from-artifact",
      sessionId: stringValue(sessionId, "bytecode session id"),
      sourceId: stringValue(sourceId, "bytecode source id"),
      artifact: [...byteArray(artifact, "artifact")],
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
    if (this.disposed) throw new Error("bytecode observation runtime is disposed");
  }

  track(info) {
    const session = new BytecodeObservationSession(this, sessionInfo(info));
    this.sessions.add(session);
    return session;
  }

  forget(session) {
    this.sessions.delete(session);
  }
}

export class BytecodeObservationSession {
  constructor(runtime, info) {
    this.runtime = runtime;
    this.info = info;
    this.disposed = false;
  }

  get handle() { return this.info.handle; }
  get sessionId() { return this.info.sessionId; }
  get sourceId() { return this.info.sourceId; }
  get traceId() { return this.info.traceId; }
  get status() { return this.info.status; }
  get sequence() { return this.info.sequence; }

  snapshot() { return this.call("snapshot"); }

  step() {
    const evidence = this.call("step");
    this.refresh();
    return evidence;
  }

  run(stepLimit) {
    const evidence = this.call("run", {
      stepLimit: integerValue(stepLimit, "bytecode run step limit", 100_000),
    });
    this.refresh();
    return evidence;
  }

  pause() {
    const accepted = this.call("pause");
    this.refresh();
    return accepted;
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

  metrics() { return this.call("metrics"); }
  events() { return this.call("events"); }
  trace() { return this.call("trace"); }
  resultDisplay() { return this.call("result-display"); }
  errorMessage() { return this.call("error-message"); }

  setObservationLimits({ stack, locals, calls, handlers, displayChars }) {
    this.assertActive();
    this.info = sessionInfo(this.runtime.invoke({
      op: "set-observation-limits",
      handle: this.handle,
      stack: integerValue(stack, "stack limit", 4_096),
      locals: integerValue(locals, "locals limit", 4_096),
      calls: integerValue(calls, "calls limit", 4_096),
      handlers: integerValue(handlers, "handlers limit", 4_096),
      displayChars: integerValue(displayChars, "display character limit", 16_384),
    }));
    return this.info;
  }

  setRetentionLimits({ events, trace }) {
    this.assertActive();
    this.info = sessionInfo(this.runtime.invoke({
      op: "set-retention-limits",
      handle: this.handle,
      events: integerValue(events, "event retention limit", 100_000),
      trace: integerValue(trace, "trace retention limit", 100_000),
    }));
    return this.info;
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
    if (this.disposed) throw new Error("bytecode observation session is disposed");
  }

  call(op, fields = {}) {
    this.assertActive();
    return this.runtime.invoke({ op, handle: this.handle, ...fields });
  }
}
