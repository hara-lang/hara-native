import assert from "node:assert/strict";
import { createHash, webcrypto } from "node:crypto";
import test from "node:test";

import {
  StudioLiveSessionController,
  StudioLiveSessionError,
  sourceRevision,
} from "./studio/live-session-controller.js";
import {
  LIVE_SESSION_CAPABILITIES_SCHEMA,
  LIVE_SESSION_PROTOCOL,
  LIVE_SESSION_REPLY_SCHEMA,
  LIVE_SESSION_STATE_SCHEMA,
} from "./host/live-session-model.js";

class ProtocolFixture {
  constructor() {
    this.requests = [];
    this.sessions = new Map();
    this.blockedUpdate = null;
  }

  blockNextUpdate() {
    let release;
    const gate = new Promise((resolve) => { release = resolve; });
    this.blockedUpdate = { gate, release };
    return release;
  }

  async dispatch(request) {
    this.requests.push(clone(request));
    assert.equal(request.protocol, LIVE_SESSION_PROTOCOL);
    if (request.op === "update" && this.blockedUpdate) {
      const blocked = this.blockedUpdate;
      this.blockedUpdate = null;
      await blocked.gate;
    }
    if (request.op === "start") return this.start(request);
    const session = this.sessions.get(request["session-id"]);
    if (!session) throw protocolError("live-session/not-found", "unknown session");
    this.assertFence(session, request);
    if (session.status === "cancelled" && request.op !== "dispose") {
      throw protocolError("live-session/cancelled", "cancelled session");
    }
    if (session.status === "disposed" && request.op !== "dispose") {
      throw protocolError("live-session/disposed", "disposed session");
    }
    if (!session.operations.includes(request.op)) {
      throw protocolError("live-session/unsupported-operation", request.op);
    }

    let payload = null;
    switch (request.op) {
      case "snapshot":
        payload = { status: session.status };
        break;
      case "step":
        session.sequence += 1;
        session.status = "running";
        payload = { sequence: session.sequence };
        break;
      case "run":
        session.sequence += 1;
        session.status = "returned";
        payload = { limit: request.payload["boundary-limit"] };
        break;
      case "pause":
        session.status = "paused";
        payload = true;
        break;
      case "resume":
        session.sequence += 1;
        session.status = "ready";
        payload = { settlement: request.payload.settlement };
        break;
      case "resolve":
        payload = { accepted: true, value: request.payload.value };
        break;
      case "reject":
        payload = { accepted: true, error: request.payload.error };
        break;
      case "update":
        payload = this.update(session, request.payload);
        break;
      case "reset":
        session.generation += 1;
        session.sequence = 0;
        session.status = "ready";
        if (session.pending) {
          session.sourceId = session.pending.sourceId;
          session.revision = session.pending.revision;
          session.pending = null;
        }
        payload = { reset: true };
        break;
      case "cancel":
        session.status = "cancelled";
        session.pending = null;
        payload = { status: "cancelled" };
        break;
      case "dispose": {
        const disposed = session.status !== "disposed";
        session.status = "disposed";
        session.pending = null;
        payload = disposed;
        break;
      }
      default:
        throw protocolError("live-session/unsupported-operation", request.op);
    }
    return this.reply(request, session, payload);
  }

  start(request) {
    const sessionId = request["session-id"];
    if (this.sessions.has(sessionId)) {
      throw protocolError("live-session/already-exists", sessionId);
    }
    const backend = request.payload.backend;
    const operations = backend === "hbc"
      ? ["snapshot", "step", "run", "pause", "resume", "resolve", "reject", "update", "reset", "cancel", "dispose"]
      : ["snapshot", "step", "run", "resume", "resolve", "reject", "update", "reset", "cancel", "dispose"];
    const session = {
      sessionId,
      backend,
      sourceId: request.payload["source-id"],
      revision: request.payload.revision,
      generation: 0,
      sequence: 0,
      status: "ready",
      operations,
      replacements: ["restart", "replace-on-next-start"],
      pending: null,
    };
    this.sessions.set(sessionId, session);
    return this.reply(request, session, {
      started: true,
      capabilities: {
        schema: LIVE_SESSION_CAPABILITIES_SCHEMA,
        protocol: LIVE_SESSION_PROTOCOL,
        backend,
        operations,
        "replacement-policies": session.replacements,
      },
    });
  }

  update(session, payload) {
    if (String(payload.source).includes("INVALID")) {
      throw protocolError("live-session/source", "invalid source");
    }
    if (!session.replacements.includes(payload.policy)) {
      throw protocolError("live-session/unsupported-replacement", payload.policy);
    }
    if (payload.policy === "replace-on-next-start") {
      session.pending = {
        sourceId: payload["source-id"],
        revision: payload.revision,
      };
      return { accepted: true, activation: "next-start", revision: payload.revision };
    }
    session.generation += 1;
    session.sequence = 0;
    session.status = "ready";
    session.sourceId = payload["source-id"];
    session.revision = payload.revision;
    session.pending = null;
    return { replaced: true };
  }

  assertFence(session, request) {
    if (request.generation !== session.generation) {
      throw protocolError("live-session/stale-generation", "stale generation");
    }
    if (request.revision !== session.revision) {
      throw protocolError("live-session/stale-revision", "stale revision");
    }
  }

  reply(request, session, payload) {
    return {
      schema: LIVE_SESSION_REPLY_SCHEMA,
      protocol: LIVE_SESSION_PROTOCOL,
      "request-id": request["request-id"],
      state: {
        schema: LIVE_SESSION_STATE_SCHEMA,
        protocol: LIVE_SESSION_PROTOCOL,
        "session-id": session.sessionId,
        "source-id": session.sourceId,
        generation: session.generation,
        revision: session.revision,
        sequence: session.sequence,
        backend: session.backend,
        status: session.status,
      },
      payload,
    };
  }
}

const protocolError = (code, message) => Object.assign(new Error(message), { code });
const clone = (value) => JSON.parse(JSON.stringify(value));

const start = (controller, overrides = {}) => controller.start({
  sessionId: "studio/ROOT/example",
  backend: "interpreter",
  sourceId: "/example.hal",
  source: "(+ 20 22)",
  revision: "sha256:one",
  ...overrides,
});

test("source revisions are canonical UTF-8 SHA-256 values", async () => {
  const source = "(+ 20 22) λ";
  const expected = createHash("sha256").update(source, "utf8").digest("hex");
  assert.equal(
    await sourceRevision(source, { cryptoImpl: webcrypto }),
    `sha256:${expected}`,
  );
});

test("Studio controls build canonical fenced requests and retain capabilities", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({
    dispatch: fixture.dispatch.bind(fixture),
    cryptoImpl: webcrypto,
    requestPrefix: "studio-test",
  });
  await start(controller);
  await controller.step("studio/ROOT/example");
  await controller.run("studio/ROOT/example", { boundaryLimit: 64 });

  assert.equal(fixture.requests[0].op, "start");
  assert.equal(fixture.requests[0].payload.backend, "interpreter");
  assert.equal(fixture.requests[1].generation, 0);
  assert.equal(fixture.requests[1].revision, "sha256:one");
  assert.equal(fixture.requests[2].payload["boundary-limit"], 64);
  assert.equal(new Set(fixture.requests.map((request) => request["request-id"])).size, 3);
  assert.equal(controller.state("studio/ROOT/example").status, "returned");
  assert.equal(controller.state("studio/ROOT/example").sequence, 2);
  assert.equal(controller.supports("studio/ROOT/example", "step"), true);
  assert.equal(controller.supports("studio/ROOT/example", "pause"), false);
  assert.equal(controller.capabilities("studio/ROOT/example").backend, "interpreter");
});

test("failed replacement leaves the active Studio fence unchanged", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({ dispatch: fixture.dispatch.bind(fixture) });
  await start(controller);
  const before = controller.state("studio/ROOT/example");

  await assert.rejects(
    controller.update("studio/ROOT/example", {
      sourceId: "/example.hal",
      source: "INVALID",
      revision: "sha256:invalid",
    }),
    (error) => error.code === "live-session/source",
  );
  assert.deepEqual(controller.state("studio/ROOT/example"), before);

  await controller.update("studio/ROOT/example", {
    sourceId: "/example.hal",
    source: "(+ 40 2)",
    revision: "sha256:two",
  });
  assert.equal(controller.state("studio/ROOT/example").generation, 1);
  assert.equal(controller.state("studio/ROOT/example").revision, "sha256:two");
  await controller.step("studio/ROOT/example");
  assert.equal(fixture.requests.at(-1).generation, 1);
  assert.equal(fixture.requests.at(-1).revision, "sha256:two");
});

test("deferred replacement remains pending until reset activates it", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({ dispatch: fixture.dispatch.bind(fixture) });
  await start(controller);
  await controller.update("studio/ROOT/example", {
    sourceId: "/example-v2.hal",
    source: "(+ 40 2)",
    revision: "sha256:two",
    policy: "replace-on-next-start",
  });
  assert.equal(controller.state("studio/ROOT/example").generation, 0);
  assert.equal(controller.state("studio/ROOT/example").revision, "sha256:one");
  assert.deepEqual(controller.pendingSource("studio/ROOT/example"), {
    sourceId: "/example-v2.hal",
    revision: "sha256:two",
  });

  await controller.reset("studio/ROOT/example");
  assert.equal(controller.state("studio/ROOT/example").generation, 1);
  assert.equal(controller.state("studio/ROOT/example").revision, "sha256:two");
  assert.equal(controller.state("studio/ROOT/example")["source-id"], "/example-v2.hal");
  assert.equal(controller.pendingSource("studio/ROOT/example"), null);
});

test("a queued reset observes deferred replacement committed ahead of it", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({ dispatch: fixture.dispatch.bind(fixture) });
  await start(controller);
  const queued = controller.update("studio/ROOT/example", {
    sourceId: "/example-v2.hal",
    source: "(+ 40 2)",
    revision: "sha256:two",
    policy: "replace-on-next-start",
  });
  const reset = controller.reset("studio/ROOT/example");
  await queued;
  await reset;
  assert.equal(controller.state("studio/ROOT/example").generation, 1);
  assert.equal(controller.state("studio/ROOT/example").revision, "sha256:two");
  assert.equal(controller.pendingSource("studio/ROOT/example"), null);
});

test("source hashing remains ordered ahead of controls invoked after update", async () => {
  const fixture = new ProtocolFixture();
  let releaseDigest;
  const digestGate = new Promise((resolve) => { releaseDigest = resolve; });
  const cryptoImpl = {
    subtle: {
      async digest(algorithm, bytes) {
        await digestGate;
        return webcrypto.subtle.digest(algorithm, bytes);
      },
    },
  };
  const controller = new StudioLiveSessionController({
    dispatch: fixture.dispatch.bind(fixture),
    cryptoImpl,
  });
  await start(controller);
  const replacement = controller.update("studio/ROOT/example", {
    sourceId: "/example.hal",
    source: "(+ 40 2)",
  });
  const staleStep = controller.step("studio/ROOT/example");
  releaseDigest();
  await replacement;
  await assert.rejects(
    staleStep,
    (error) => error instanceof StudioLiveSessionError &&
      error.code === "studio-live-session/stale-command",
  );
  assert.deepEqual(fixture.requests.map((request) => request.op), ["start", "update"]);
});

test("capabilities reject unsupported controls without selecting another backend", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({ dispatch: fixture.dispatch.bind(fixture) });
  await start(controller);
  const requestCount = fixture.requests.length;
  await assert.rejects(
    controller.pause("studio/ROOT/example"),
    (error) => error instanceof StudioLiveSessionError &&
      error.code === "studio-live-session/unsupported-operation",
  );
  await assert.rejects(
    controller.update("studio/ROOT/example", {
      sourceId: "/example.hal",
      source: "(+ 40 2)",
      revision: "sha256:two",
      policy: "preserve-runtime",
    }),
    (error) => error instanceof StudioLiveSessionError &&
      error.code === "studio-live-session/unsupported-replacement",
  );
  assert.equal(fixture.requests.length, requestCount);
  assert.equal(controller.state("studio/ROOT/example").backend, "interpreter");

  await start(controller, {
    sessionId: "studio/ROOT/hbc",
    backend: "hbc",
    sourceId: "/hbc.hal",
  });
  await controller.pause("studio/ROOT/hbc");
  assert.equal(controller.state("studio/ROOT/hbc").status, "paused");
  assert.equal(controller.state("studio/ROOT/hbc").backend, "hbc");
});

test("a queued stale UI command is rejected after replacement advances the fence", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({ dispatch: fixture.dispatch.bind(fixture) });
  await start(controller);
  const release = fixture.blockNextUpdate();
  const replacement = controller.update("studio/ROOT/example", {
    sourceId: "/example.hal",
    source: "(+ 40 2)",
    revision: "sha256:two",
  });
  const staleStep = controller.step("studio/ROOT/example");
  release();
  await replacement;
  await assert.rejects(
    staleStep,
    (error) => error instanceof StudioLiveSessionError &&
      error.code === "studio-live-session/stale-command",
  );
  assert.equal(controller.state("studio/ROOT/example").generation, 1);
  assert.equal(controller.state("studio/ROOT/example").sequence, 0);
});

test("malformed or non-monotonic replies never replace the accepted Studio state", async () => {
  const fixture = new ProtocolFixture();
  let corruptSnapshot = false;
  const controller = new StudioLiveSessionController({
    dispatch: async (request) => {
      const reply = await fixture.dispatch(request);
      if (corruptSnapshot && request.op === "snapshot") {
        return { ...reply, state: { ...reply.state, sequence: 0 } };
      }
      return reply;
    },
  });
  await start(controller);
  await controller.step("studio/ROOT/example");
  const before = controller.state("studio/ROOT/example");
  corruptSnapshot = true;
  await assert.rejects(
    controller.snapshot("studio/ROOT/example"),
    (error) => error instanceof StudioLiveSessionError &&
      error.code === "studio-live-session/non-monotonic-sequence",
  );
  assert.deepEqual(controller.state("studio/ROOT/example"), before);
});

test("cancellation stays terminal and disposal remains idempotent", async () => {
  const fixture = new ProtocolFixture();
  const controller = new StudioLiveSessionController({ dispatch: fixture.dispatch.bind(fixture) });
  await start(controller);
  await controller.cancel("studio/ROOT/example");
  assert.equal(controller.state("studio/ROOT/example").status, "cancelled");
  await assert.rejects(
    controller.step("studio/ROOT/example"),
    (error) => error.code === "live-session/cancelled",
  );
  assert.equal((await controller.dispose("studio/ROOT/example")).payload, true);
  assert.equal((await controller.dispose("studio/ROOT/example")).payload, false);
  assert.equal(controller.state("studio/ROOT/example").status, "disposed");
});
