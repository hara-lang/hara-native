import { expect, test } from "@playwright/test";

test("Chromium starts isolated native host profiles without a source checkout", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const result = await page.evaluate(async () => {
    const vm = await import("/rust/web/packages/browser/dist/native-vm/hara.mjs");
    const full = await import("/rust/web/packages/browser/dist/native-full/hara.mjs");
    const first = await vm.start({
      resources: { "native.fixture": "(ns native.fixture) (def answer 42)" }
    });
    const second = await vm.start();
    const whole = await full.start();
    try {
      first.require("native.fixture");
      const compiled = await whole.compileWholeWasm("(+ 20 22)");
      return {
        first: first.eval("native.fixture/answer"),
        isolated: (() => {
          try {
            second.eval("native.fixture/answer");
            return false;
          } catch {
            return true;
          }
        })(),
        whole: String(compiled.call())
      };
    } finally {
      await first.dispose();
      await second.dispose();
      await whole.dispose();
    }
  });
  expect(result).toEqual({ first: "42", isolated: true, whole: "42" });
});
