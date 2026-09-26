// Invoked ONLY by the Rust #[cfg(test)] fixture. No real SMTP or local account DB.
import { chromium, expect } from "@playwright/test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { mkdir } from "node:fs/promises";
import { createServer } from "node:net";
import { createWebServer } from "../../server/server.mjs";

const upstream = process.env.QIBAN_TEST_UPSTREAM;
assert.match(upstream ?? "", /^http:\/\/127\.0\.0\.1:\d+$/);
const port = await new Promise((resolve) => {
  const probe = createServer();
  probe.listen(0, "127.0.0.1", () => {
    const port = probe.address().port;
    probe.close(() => resolve(port));
  });
});
const origin = `http://127.0.0.1:${port}`;
const server = createWebServer({
  origin,
  upstream,
  dist: fileURLToPath(new URL("../../dist", import.meta.url)),
});
await new Promise((resolve) => server.listen(port, "127.0.0.1", resolve));
const browser = await chromium.launch({ channel: "msedge", headless: true });
const output = fileURLToPath(new URL("../../test-results/", import.meta.url));
await mkdir(output, { recursive: true });
try {
  const desktop = await browser.newContext({
    viewport: { width: 1280, height: 940 },
  });
  const mobile = await browser.newContext({
    viewport: { width: 390, height: 844 },
    isMobile: true,
    hasTouch: true,
  });
  const other = await browser.newContext();
  const pc = await desktop.newPage();
  const phone = await mobile.newPage();
  const bob = await other.newPage();
  async function login(page, email) {
    await page.goto(origin);
    await page.getByLabel("邮箱", { exact: true }).fill(email);
    await page.getByRole("button", { name: "获取验证码", exact: true }).click();
    await expect(page.getByLabel("8 位验证码")).toBeVisible();
    const response = await fetch(
      `${upstream}/__test/code/${encodeURIComponent(email)}`,
    );
    const { code } = await response.json();
    await page.getByLabel("8 位验证码").fill(code);
    await page.getByRole("button", { name: "与栖栖会合", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: /我们的待办/ }),
    ).toBeVisible();
  }
  await phone.goto(origin);
  await expect(
    phone.getByRole("heading", { name: "接回你的伙伴" }),
  ).toBeVisible();
  await phone.screenshot({ path: `${output}mobile-login.png`, fullPage: true });
  await login(pc, "alice@example.com");
  const firstLoginAt = Date.now();
  // Lose the successful creation response. A click retry must reuse requestId.
  await pc.route("**/api/tasks", async (route) => {
    if (route.request().method() === "POST") {
      await route.fetch();
      await route.abort();
      await pc.unroute("**/api/tasks");
    } else await route.continue();
  });
  await pc.getByLabel("想让栖栖记住什么？").fill("周末一起整理旅行清单");
  await pc.getByRole("button", { name: "记下来", exact: true }).click();
  await expect(pc.getByRole("status")).toContainText("连接中断");
  await pc.getByRole("button", { name: "记下来", exact: true }).click();
  await expect(pc.locator(".todos .task-list li")).toHaveCount(1);
  await pc.locator("summary").click();
  const accountId = await pc.getByTestId("account-id").textContent();
  const companionId = await pc.getByTestId("companion-id").textContent();
  await pc.reload();
  await expect(pc.locator(".todos .task-list li")).toHaveCount(1);
  await pc.screenshot({ path: `${output}desktop-tasks.png`, fullPage: true });
  assert.equal(await pc.evaluate(() => localStorage.length), 0);
  assert.equal(await pc.evaluate(() => sessionStorage.length), 0);
  assert.equal(await pc.evaluate(() => document.cookie), "");
  assert.equal(
    (await desktop.cookies()).find((c) => c.name === "qiban_local_session")
      ?.httpOnly,
    true,
  );
  await login(bob, "bob@example.com");
  await expect(bob.locator(".todos .task-list li")).toHaveCount(0);
  // The real coordinator enforces a 60-second mail cooldown. Do not bypass it.
  console.log(
    "Real coordinator: first login, recovery, task dedup and second-account isolation passed; waiting for OTP cooldown.",
  );
  await new Promise((resolve) =>
    setTimeout(resolve, Math.max(0, 61000 - (Date.now() - firstLoginAt))),
  );
  await login(phone, "alice@example.com");
  await phone.locator("summary").click();
  await expect(phone.getByTestId("account-id")).toHaveText(accountId);
  await expect(phone.getByTestId("companion-id")).toHaveText(companionId);
  await expect(phone.locator(".todos .task-list li")).toHaveCount(1);
  assert.equal(
    await phone.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await phone.screenshot({ path: `${output}mobile-tasks.png`, fullPage: true });
  await mobile.setOffline(true);
  await phone.getByRole("button", { name: "刷新", exact: true }).click();
  await expect(phone.getByRole("status")).toContainText("连接中断");
  await pc.getByLabel("想让栖栖记住什么？").fill("电脑新增的第二件小事");
  await pc.getByRole("button", { name: "记下来", exact: true }).click();
  await expect(pc.locator(".todos .task-list li")).toHaveCount(2);
  await mobile.setOffline(false);
  await expect(phone.locator(".todos .task-list li")).toHaveCount(2);
  await phone
    .getByRole("button", { name: "取消 周末一起整理旅行清单", exact: true })
    .click();
  await expect(phone.locator(".cancelled")).toContainText(
    "周末一起整理旅行清单",
  );
  await pc.getByRole("button", { name: "刷新", exact: true }).click();
  await expect(pc.locator(".cancelled")).toHaveCount(1);
  // A late pre-logout task response must not restore the old account's UI.
  let release;
  const held = new Promise((resolve) => {
    release = resolve;
  });
  let captured;
  const capture = new Promise((resolve) => {
    captured = resolve;
  });
  let delivered;
  const delivery = new Promise((resolve) => {
    delivered = resolve;
  });
  await phone.route(
    "**/api/state",
    async (route) => {
      const response = await route.fetch();
      captured();
      await held;
      await route.fulfill({ response });
      delivered();
    },
    { times: 1 },
  );
  await phone.getByRole("button", { name: "刷新", exact: true }).click();
  await capture;
  await phone.getByRole("button", { name: "退出本设备", exact: true }).click();
  await expect(
    phone.getByRole("heading", { name: "接回你的伙伴" }),
  ).toBeVisible();
  release();
  await delivery;
  await expect(phone.locator(".todos .task-list li")).toHaveCount(0);
  await pc.getByRole("button", { name: "刷新", exact: true }).click();
  await expect(pc.locator(".todos .task-list li")).toHaveCount(2);
  // A second context with a copy of the PC session checks global revocation
  // without sending a third code. The previous phone session was independent.
  const copy = await browser.newContext({
    storageState: await desktop.storageState(),
  });
  const mirror = await copy.newPage();
  await mirror.goto(origin);
  await expect(mirror.locator(".todos .task-list li")).toHaveCount(2);
  pc.once("dialog", (dialog) => dialog.accept());
  await pc.getByRole("button", { name: "退出所有设备", exact: true }).click();
  await expect(pc.getByRole("heading", { name: "接回你的伙伴" })).toBeVisible();
  await mirror.getByRole("button", { name: "刷新", exact: true }).click();
  await expect(
    mirror.getByRole("heading", { name: "接回你的伙伴" }),
  ).toBeVisible();
  await bob.getByRole("button", { name: "刷新", exact: true }).click();
  await expect(bob.getByRole("heading", { name: /我们的待办/ })).toBeVisible();
  console.log(
    "PASS: two device logins share companion and tasks; account isolation, cookie recovery, lost-response dedup, reconnect, cancellation, stale response protection and logout verified.",
  );
} finally {
  await browser.close();
  server.close();
  server.closeAllConnections();
}
