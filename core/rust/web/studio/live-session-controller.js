import {
  LIVE_SESSION_CAPABILITIES_SCHEMA,
  LIVE_SESSION_PROTOCOL,
  LIVE_SESSION_REPLY_SCHEMA,
  LIVE_SESSION_STATE_SCHEMA,
} from "../host/live-session-model.js";

const GENERATION_ADVANCING_OPERATIONS = new Set(["update", "reset"]);
const TERMINAL_STATUSES = new Set(["cancelled", "disposed"]);
let nextControllerId = 0;

export class StudioLiveSessionError extends Error {
  constructor(code, message, detail = null) {
    super(message);
    this.name = "StudioLiveSessionError";
    this.code = code;
    this.detail = detail;
  }
}

/** Return the canonical UTF-8 SHA-256 revision used to fence Studio source. */
export async function sourceRevision(source, { cryptoImpl = globalThis.crypto } = {}) {
  if (typeof source !== "string") {
    throw new StudioLiveSessionError(
      "studio-live-session/source",
      "Studio live-session source must be a string",
    );
  }
  if (typeof cryptoImpl?.subtle?.digest !== "function") {
    throw new StudioLiveSessionError(
      "studio-live-session/revision-unavailable",
      "SHA-256 source revision requires Web Crypto",
    );
  }
  const digest = await cryptoImpl.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(source),
  );
  return `sha256:${[...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("")}`;
}

/**
 * Studio-side owner of live-session requests and reply state.
 *
 * The controller never chooses a fallback backend and never exposes an
 * interpreter or HBC session. It computes source revisions, copies the active
 * generation/revision into every command, serializes commands per session, and
 * accepts state changes only from a matching canonical reply.
 */
export class StudioLiveSessionController {
  constructor({
    dispatch,
    cryptoImpl = globalThis.crypto,
    requestPrefix = `studio/live-session-${++nextControllerId}`,
  } = {}) {
    const invoke = typeof dispatch === "function"
      ? dispatch
      : dispatch?.dispatch?.bind(dispatch);
    if (typeof invoke !== "function") {
      throw new StudioLiveSessionError(
        "studio-live-session/transport",
        "Studio live-session controller requires dispatch(request)",
      );
    }
    this.dispatch = invoke;
    this.cryptoImpl = cryptoImpl;
    this.requestPrefix = nonEmptyString(requestPrefix, "Studio request prefix");
    this.nextRequestId = 0;
    this.records = new Map();
    this.pendingStarts = new Set();
    this.queues = new Map();
  }

  async start({
    sessionId,
    backend = "interpreter",
    sourceId,
    source,
    revision = null,
  } = {}) {
    sessionId = nonEmptyString(sessionId, "Studio live-session id");
    backend = nonEmptyString(backend, "Studio live-session backend");
    sourceId = nonEmptyString(sourceId, "Studio source id");
    if (this.records.has(sessionId) || this.pendingStarts.has(sessionId)) {
      throw new StudioLiveSessionError(
        "studio-live-session/already-exists",
        `Studio live session already exists: ${sessionId}`,
        { sessionId },
      );
    }
    this.pendingStarts.add(sessionId);
    try {
      const resolvedRevision = revision == null
        ? await sourceRevision(source, { cryptoImpl: this.cryptoImpl })
        : nonEmptyString(revision, "Studio source revision");
      const request = this.request("start", sessionId, {
        backend,
        "source-id": sourceId,
        revision: resolvedRevision,
        source: sourceString(source),
      });
      const reply = await this.dispatch(request);
      const state = validateReply(request, reply, null, {
        backend,
        sourceId,
        revision: resolvedRevision,
      });
      const capabilities = validateCapabilities(
        reply.payload?.capabilities,
        state.backend,
      );
      this.records.set(sessionId, recordValue(state, capabilities));
      return reply;
    } finally {
      this.pendingStarts.delete(sessionId);
    }
  }

  async update(sessionId, {
    sourceId,
    source,
    revision = null,
    policy = "restart",
  } = {}) {
    const record = this.requireRecord(sessionId);
    this.requireOperation(record, "update");
    policy = nonEmptyString(policy, "Studio replacement policy");
    if (!record.replacementPolicies.has(policy)) {
      throw new StudioLiveSessionError(
        "studio-live-session/unsupported-replacement",
        `${record.state.backend} does not advertise ${policy} replacement`,
        { backend: record.state.backend, policy },
      );
    }
    sourceId = nonEmptyString(sourceId, "Studio source id");
    const sourceValue = sourceString(source);
    const revisionPromise = revision == null
      ? sourceRevision(sourceValue, { cryptoImpl: this.cryptoImpl })
      : Promise.resolve(nonEmptyString(revision, "Studio source revision"));
    const requestId = this.allocateRequestId();
    return this.enqueue(sessionId, async () => {
      const resolvedRevision = await revisionPromise;
      const request = this.fencedRequest(record.state, "update", {
        policy,
        "source-id": sourceId,
        revision: resolvedRevision,
        source: sourceValue,
      }, requestId);
      const current = this.requireRecord(sessionId);
      assertCurrentFence(request, current.state);
      const reply = await this.dispatch(request);
      const state = validateReply(request, reply, current.state, {
        policy,
        sourceId,
        revision: resolvedRevision,
      });
      const pending = policy === "replace-on-next-start"
        ? Object.freeze({ sourceId, revision: resolvedRevision })
        : null;
      this.records.set(
        sessionId,
        recordValue(state, current.capabilities, pending),
      );
      return reply;
    });
  }

  async command(sessionId, operation, payload = {}) {
    const record = this.requireRecord(sessionId);
    operation = nonEmptyString(operation, "Studio live-session operation");
    this.requireOperation(record, operation);
    const request = this.fencedRequest(record.state, operation, payload);
    return this.enqueue(sessionId, async () => {
      const current = this.requireRecord(sessionId);
      assertCurrentFence(request, current.state);
      const reply = await this.dispatch(request);
      const state = validateReply(request, reply, current.state, {
        pending: current.pending,
      });
      const pending = operation === "reset" || TERMINAL_STATUSES.has(state.status)
        ? null
        : current.pending;
      this.records.set(
        sessionId,
        recordValue(state, current.capabilities, pending),
      );
      return reply;
    });
  }

  snapshot(sessionId) { return this.command(sessionId, "snapshot"); }
  step(sessionId) { return this.command(sessionId, "step"); }
  run(sessionId, { boundaryLimit = 100_000 } = {}) {
    return this.command(sessionId, "run", { "boundary-limit": boundaryLimit });
  }
  pause(sessionId) { return this.command(sessionId, "pause"); }
  resume(sessionId, settlement = null) {
    return this.command(sessionId, "resume", { settlement });
  }
  resolve(sessionId, value = null) {
    return this.command(sessionId, "resolve", { value });
  }
  reject(sessionId, error = null) {
    return this.command(sessionId, "reject", { error });
  }
  reset(sessionId) { return this.command(sessionId, "reset"); }
  cancel(sessionId) { return this.command(sessionId, "cancel"); }
  dispose(sessionId) { return this.command(sessionId, "dispose"); }

  state(sessionId) { return this.requireRecord(sessionId).state; }
  capabilities(sessionId) { return this.requireRecord(sessionId).capabilities; }
  pendingSource(sessionId) { return this.requireRecord(sessionId).pending; }
  supports(sessionId, operation) {
    return this.requireRecord(sessionId).operations.has(String(operation));
  }
  supportsReplacement(sessionId, policy) {
    return this.requireRecord(sessionId).replacementPolicies.has(String(policy));
  }
  list() {
    return Object.freeze(
      [...this.records.values()].map((record) => record.state),
    );
  }

  async disposeAll() {
    const results = [];
    const errors = [];
    for (const sessionId of [...this.records.keys()]) {
      try {
        results.push(await this.dispose(sessionId));
      } catch (error) {
        errors.push(error);
      }
    }
    if (errors.length) {
      throw new AggregateError(
        errors,
        "One or more Studio live sessions failed to dispose",
      );
    }
    return Object.freeze(results);
  }

  allocateRequestId() {
    return `${this.requestPrefix}/${++this.nextRequestId}`;
  }

  request(operation, sessionId, payload, requestId = this.allocateRequestId()) {
    return Object.freeze({
      protocol: LIVE_SESSION_PROTOCOL,
      "request-id": requestId,
      "session-id": sessionId,
      op: operation,
      payload: Object.freeze({ ...payload }),
    });
  }

  fencedRequest(
    state,
    operation,
    payload,
    requestId = this.allocateRequestId(),
  ) {
    return Object.freeze({
      protocol: LIVE_SESSION_PROTOCOL,
      "request-id": requestId,
      "session-id": state["session-id"],
      generation: state.generation,
      revision: state.revision,
      op: operation,
      payload: Object.freeze({ ...payload }),
    });
  }

  requireRecord(sessionId) {
    sessionId = nonEmptyString(sessionId, "Studio live-session id");
    const record = this.records.get(sessionId);
    if (!record) {
      throw new StudioLiveSessionError(
        "studio-live-session/not-found",
        `Unknown Studio live session: ${sessionId}`,
        { sessionId },
      );
    }
    return record;
  }

  requireOperation(record, operation) {
    if (!record.operations.has(operation)) {
      throw new StudioLiveSessionError(
        "studio-live-session/unsupported-operation",
        `${record.state.backend} does not advertise ${operation}`,
        { backend: record.state.backend, operation },
      );
    }
  }

  enqueue(sessionId, operation) {
    const previous = this.queues.get(sessionId) ?? Promise.resolve();
    const current = previous.catch(() => undefined).then(operation);
    this.queues.set(sessionId, current);
    void current.then(
      () => this.clearQueue(sessionId, current),
      () => this.clearQueue(sessionId, current),
    );
    return current;
  }

  clearQueue(sessionId, current) {
    if (this.queues.get(sessionId) === current) this.queues.delete(sessionId);
  }
}

function assertCurrentFence(request, state) {
  if (request.generation !== state.generation ||
      request.revision !== state.revision) {
    throw new StudioLiveSessionError(
      "studio-live-session/stale-command",
      `Studio command ${request["request-id"]} targets ` +
        `${request.generation} / ${request.revision}, not ` +
        `${state.generation} / ${state.revision}`,
      { request, state },
    );
  }
}

function recordValue(state, capabilities, pending = null) {
  const operations = new Set(capabilities.operations);
  const replacementPolicies = new Set(capabilities["replacement-policies"]);
  return Object.freeze({
    state,
    capabilities,
    operations,
    replacementPolicies,
    pending,
  });
}

function validateReply(request, value, previousState, expectation) {
  const reply = objectValue(value, "Studio live-session reply");
  if (reply.schema !== LIVE_SESSION_REPLY_SCHEMA ||
      reply.protocol !== LIVE_SESSION_PROTOCOL) {
    throw protocolError("reply", reply);
  }
  if (reply["request-id"] !== request["request-id"]) {
    throw new StudioLiveSessionError(
      "studio-live-session/request-mismatch",
      `Live-session reply ${reply["request-id"] ?? "missing"} does not match ` +
        request["request-id"],
    );
  }
  const state = validateState(reply.state, request["session-id"]);

  if (previousState == null) {
    if (state.generation !== 0 || state.backend !== expectation.backend ||
        state["source-id"] !== expectation.sourceId ||
        state.revision !== expectation.revision) {
      throw new StudioLiveSessionError(
        "studio-live-session/start-state",
        "Live-session start reply does not describe the requested source",
        state,
      );
    }
    return state;
  }

  if (state.backend !== previousState.backend) {
    throw new StudioLiveSessionError(
      "studio-live-session/backend-changed",
      `Live-session backend changed from ${previousState.backend} to ${state.backend}`,
      state,
    );
  }
  const advances = state.generation === previousState.generation + 1;
  const stable = state.generation === previousState.generation;
  if (!stable && !advances) {
    throw staleReply(previousState, state);
  }
  if (advances && !GENERATION_ADVANCING_OPERATIONS.has(request.op)) {
    throw staleReply(previousState, state);
  }
  if (stable && state.sequence < previousState.sequence) {
    throw new StudioLiveSessionError(
      "studio-live-session/non-monotonic-sequence",
      `Live-session sequence moved backwards from ${previousState.sequence} to ${state.sequence}`,
      state,
    );
  }

  if (request.op === "update") {
    const queued = expectation.policy === "replace-on-next-start";
    if (queued) {
      if (!stable || state.revision !== previousState.revision ||
          state["source-id"] !== previousState["source-id"]) {
        throw staleReply(previousState, state);
      }
    } else if (!advances || state.revision !== expectation.revision ||
               state["source-id"] !== expectation.sourceId) {
      throw staleReply(previousState, state);
    }
    return state;
  }

  if (request.op === "reset") {
    const expectedSource = expectation.pending ?? {
      sourceId: previousState["source-id"],
      revision: previousState.revision,
    };
    if (!advances || state.revision !== expectedSource.revision ||
        state["source-id"] !== expectedSource.sourceId) {
      throw staleReply(previousState, state);
    }
    return state;
  }

  if (!stable || state.revision !== previousState.revision ||
      state["source-id"] !== previousState["source-id"]) {
    throw staleReply(previousState, state);
  }
  return state;
}

function validateState(value, sessionId) {
  const state = objectValue(value, "Studio live-session state");
  if (state.schema !== LIVE_SESSION_STATE_SCHEMA ||
      state.protocol !== LIVE_SESSION_PROTOCOL) {
    throw protocolError("state", state);
  }
  if (state["session-id"] !== sessionId) {
    throw new StudioLiveSessionError(
      "studio-live-session/session-mismatch",
      `Live-session reply targets ${state["session-id"] ?? "missing"}, ` +
        `expected ${sessionId}`,
    );
  }
  nonEmptyString(state["source-id"], "Live-session source id");
  nonEmptyString(state.revision, "Live-session source revision");
  nonEmptyString(state.backend, "Live-session backend");
  nonEmptyString(state.status, "Live-session status");
  nonNegativeInteger(state.generation, "Live-session generation");
  nonNegativeInteger(state.sequence, "Live-session sequence");
  return Object.freeze({ ...state });
}

function validateCapabilities(value, backend) {
  const capabilities = objectValue(value, "Studio live-session capabilities");
  if (capabilities.schema !== LIVE_SESSION_CAPABILITIES_SCHEMA ||
      capabilities.protocol !== LIVE_SESSION_PROTOCOL ||
      capabilities.backend !== backend ||
      !Array.isArray(capabilities.operations) ||
      !Array.isArray(capabilities["replacement-policies"])) {
    throw protocolError("capabilities", capabilities);
  }
  return Object.freeze({
    ...capabilities,
    operations: Object.freeze(capabilities.operations.map(String)),
    "replacement-policies": Object.freeze(
      capabilities["replacement-policies"].map(String),
    ),
  });
}

function staleReply(previousState, state) {
  return new StudioLiveSessionError(
    "studio-live-session/stale-reply",
    "Live-session reply does not follow the active generation and revision",
    { previous: previousState, state },
  );
}

function protocolError(kind, value) {
  return new StudioLiveSessionError(
    "studio-live-session/protocol",
    `Invalid live-session ${kind} schema or protocol`,
    value,
  );
}

function objectValue(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new StudioLiveSessionError(
      "studio-live-session/protocol",
      `${label} must be an object`,
      value,
    );
  }
  return value;
}

function nonEmptyString(value, label) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new StudioLiveSessionError(
      "studio-live-session/value",
      `${label} must be a non-empty string`,
      value,
    );
  }
  return value;
}

function sourceString(value) {
  const source = typeof value === "string" ? value : null;
  if (source == null || source.length === 0) {
    throw new StudioLiveSessionError(
      "studio-live-session/source",
      "Studio live-session source must be a non-empty string",
    );
  }
  return source;
}

function nonNegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new StudioLiveSessionError(
      "studio-live-session/protocol",
      `${label} must be a non-negative safe integer`,
      value,
    );
  }
  return value;
}
