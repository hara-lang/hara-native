import { createBrowserBroker } from "./studio/broker.js";
import { createHostServices } from "./studio/host-services.js";
import { GraphHost } from "./studio/graph-host.js";
import { SessionRouter } from "./studio/session-router.js";
import { CapabilityRegistry } from "./studio/capability-registry.js";
import { createCanvasCapability } from "./studio/capabilities/canvas.js";
import { createClockCapability } from "./studio/capabilities/clock.js";
import { CanvasRuntime } from "./studio/canvas-runtime.js";
import { mountStudio } from "./studio/ui.js";

// Smoke-page bootstrap. The website supplies source-owned Studio modules from
// verified HARP packages before this host starts; a native artifact never
// fetches a sibling checkout's HAL files.
const resources = window.__HARA_STUDIO_PACKAGE_RESOURCES__;
if (!resources || typeof resources !== "object") {
  throw new Error("studio/package-resources-missing: activate verified HARP packages before starting Studio");
}
const wasmUrl = window.__HARA_NATIVE_WASM_URL__;
if (typeof wasmUrl !== "string" || wasmUrl.length === 0) {
  throw new Error("studio/native-wasm-url-missing: provide a release-owned Wasm artifact URL");
}
const bytes = new Uint8Array(await (await fetch(wasmUrl)).arrayBuffer());
const sessionRouter = new SessionRouter();
const canvasRuntime = new CanvasRuntime();
const capabilityRegistry = new CapabilityRegistry({ adapters: {
  "surface/canvas-2d": createCanvasCapability(canvasRuntime),
  "clock/frame": createClockCapability()
} });
const graphHost = new GraphHost({ workerUrl: "./studio/program-worker.js", sessionRouter, capabilityRegistry });
const broker = createBrowserBroker({
  workerUrl: "./packages/hta/worker.mjs",
  moduleBytes: bytes,
  hostCalls: createHostServices({ canvasRuntime, graphHost, graphHostOptions: { sessionRouter } }),
  resources,
  onKernelStarting: async (kernel) => {
    const mount = await kernel.context.createFilesystem({ provider: "indexeddb", key: "studio-default" });
    await kernel.context.session().attachFilesystem(mount);
  },
  onKernelCreated: async (kernel) => sessionRouter.register(kernel.name, kernel.context, {
    onRelease: (sessionId) => graphHost.releaseSession(sessionId)
  }),
  onKernelClosed: (kernel) => sessionRouter.unregister(kernel.name)
});
window.studio = mountStudio(document.getElementById("hara-studio-mount"), { broker });
