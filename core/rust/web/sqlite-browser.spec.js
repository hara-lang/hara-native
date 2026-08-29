import { expect, test } from "@playwright/test";

test("SQLite OPFS persists an acknowledged receipt across worker replacement", async ({ page }) => {
  await page.goto("/rust/web/sqlite-browser.html");
  await expect
    .poll(() => page.evaluate(() => window.sqliteOpfsConformance))
    .toEqual({ input: 42, firstDelivery: 1, redelivery: 0, acknowledged: 1 });
});
