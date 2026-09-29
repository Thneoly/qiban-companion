// Invoked ONLY by the Rust #[cfg(test)] fixture. No real SMTP or local account DB.
import { chromium, expect } from "@playwright/test";
import { createHash, randomBytes, randomUUID } from "node:crypto";
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
  // Scoped to the global notice: panel banners are separate role=status
  // elements and would make an unscoped locator strict-mode ambiguous.
  await expect(phone.locator("main > .notice")).toContainText("连接中断");
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
  // --- Device presence: a third alice session plays the desktop sender ---
  async function directLogin(email) {
    const request = await fetch(`${upstream}/v1/auth/request-code`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email }),
    });
    const receipt = await request.json();
    const codeResponse = await fetch(
      `${upstream}/__test/code/${encodeURIComponent(email)}`,
    );
    const { code } = await codeResponse.json();
    const verify = await fetch(`${upstream}/v1/auth/verify-code`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        challengeId: receipt.challengeId,
        code,
        // The coordinator requires a 43-char base64url nonce (32 bytes).
        nonce: randomBytes(32).toString("base64url"),
      }),
    });
    const grant = await verify.json();
    return grant.accessToken;
  }
  // The per-email OTP cooldown is 60s between consecutive sends; the
  // phone's re-login above consumed the latest one. Wait out the remainder
  // before requesting a third code for alice.
  await new Promise((resolve) => setTimeout(resolve, 61000));
  const desktopToken = await directLogin("alice@example.com");
  const offerResponse = await fetch(`${upstream}/v1/pairings/offer`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${desktopToken}`,
    },
    body: JSON.stringify({ name: "客厅电脑" }),
  });
  assert.equal(offerResponse.status, 200);
  const offer = await offerResponse.json();
  const beat = async () => {
    const response = await fetch(`${upstream}/v1/devices/heartbeat`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${desktopToken}`,
      },
      body: JSON.stringify({ capabilities: ["document_excerpt"] }),
    });
    assert.equal(response.status, 200);
  };
  await beat();
  await phone.getByRole("button", { name: "刷新配对", exact: true }).click();
  await expect(phone.locator(".pairing .presence")).toContainText("电脑在线");
  await expect(phone.locator(".pairing .presence")).toContainText(
    "可执行：文档摘录",
  );
  await expect(phone.locator(".pairing .presence")).toContainText(
    "每次动作仍需确认",
  );
  // --- Document task progress: accept the pending pairing on the phone, then
  // drive real shares/admits/receipts through the coordinator as the desktop ---
  const pairingPanel = phone.locator(".pairing");
  await pairingPanel
    .getByLabel("电脑配对码", { exact: true })
    .fill(`${offer.code.slice(0, 3)} ${offer.code.slice(3)}`);
  await pairingPanel
    .getByRole("button", { name: "核对电脑与权限", exact: true })
    .click();
  await expect(pairingPanel.locator(".notice")).toContainText("客厅电脑");
  await pairingPanel
    .getByRole("button", { name: "确认配对这台电脑", exact: true })
    .click();
  await expect(pairingPanel.getByRole("status")).toContainText("配对成功");
  const listed = await (
    await fetch(`${upstream}/v1/pairings`, {
      headers: { Authorization: `Bearer ${desktopToken}` },
    })
  ).json();
  const pair = listed.find((p) => p.id === offer.pairing.id);
  assert.equal(pair?.status, "active");
  const desktopCall = async (path, body) => {
    const response = await fetch(`${upstream}${path}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${desktopToken}`,
      },
      body: JSON.stringify(body),
    });
    assert.equal(response.status, 200, `${path} -> ${response.status}`);
    return response.json();
  };
  const share = (name, preview) =>
    desktopCall("/v1/documents", {
      pairingId: pair.id,
      binding: {
        actionId: randomUUID(),
        resourceId: randomUUID(),
        resourceVersion: 1,
        parametersDigest: createHash("sha256")
          .update(`remote-excerpt-v1\0${name}\0${preview}`)
          .digest("hex"),
        pairRevision: pair.revision,
        scope: "document_excerpt",
      },
      sourceName: name,
      preview,
    });
  const admit = (doc) =>
    desktopCall(
      `/v1/documents/${doc.authorization.binding.actionId}/admit`,
      doc.authorization.binding,
    );
  const receipt = (doc, state) =>
    desktopCall(`/v1/documents/${doc.authorization.binding.actionId}/receipt`, {
      binding: doc.authorization.binding,
      state,
      artifactHash: doc.artifactHash,
    });
  const documents = phone.locator(".remote-documents");
  const row = (name) => documents.locator("article").filter({ hasText: name });
  const refreshDocuments = () =>
    phone
      .getByRole("button", { name: "刷新文档状态", exact: true })
      .click();

  // A: happy path through every derived phase, ending in the verified entry.
  await beat();
  const docA = await share("进度.txt", "手机端任务进度回归的第一份摘录");
  const rowA = row("进度.txt");
  await expect(rowA).toContainText("等待手机确认", { timeout: 15000 });
  phone.once("dialog", (dialog) => dialog.accept());
  await rowA
    .getByRole("button", { name: "确认电脑保存这份摘录", exact: true })
    .click();
  await expect(rowA.getByTestId("remote-document-status")).toContainText(
    "已确认，等待电脑保存",
  );
  await beat();
  await refreshDocuments();
  await expect(rowA.getByTestId("remote-document-phase-hint")).toContainText(
    "电脑在线",
  );
  await admit(docA);
  await beat();
  await rowA
    .getByRole("button", { name: "刷新此任务 进度.txt", exact: true })
    .click();
  await expect(rowA.getByTestId("remote-document-status")).toHaveText(
    "执行中（推断）",
  );
  await receipt(docA, "completed");
  await refreshDocuments();
  await expect(rowA.getByTestId("remote-document-status")).toHaveText(
    "已保存并核验",
  );
  await rowA.locator("summary").click();
  await expect(rowA).toContainText("服务端已核对该哈希与这份预览的摘要一致");
  // B: unknown result resolves after the desktop reconciles.
  await beat();
  const docB = await share("核对.txt", "先报告未知再补齐结果的摘录");
  const rowB = row("核对.txt");
  await expect(rowB).toContainText("等待手机确认", { timeout: 15000 });
  phone.once("dialog", (dialog) => dialog.accept());
  await rowB
    .getByRole("button", { name: "确认电脑保存这份摘录", exact: true })
    .click();
  await expect(rowB.getByTestId("remote-document-status")).toContainText(
    "已确认，等待电脑保存",
  );
  await admit(docB);
  await receipt(docB, "unknown");
  await beat();
  await refreshDocuments();
  await expect(rowB.getByTestId("remote-document-status")).toHaveText(
    "结果未知，待自动核对",
  );
  await expect(rowB.getByTestId("remote-document-phase-hint")).toContainText(
    "自动核对",
  );
  await receipt(docB, "completed");
  await refreshDocuments();
  await expect(rowB.getByTestId("remote-document-status")).toHaveText(
    "已保存并核验",
  );
  // C: cancelling before admission hides the cancel affordance afterwards.
  const docC = await share("停止.txt", "确认前就取消的摘录");
  const rowC = row("停止.txt");
  await expect(rowC).toContainText("等待手机确认", { timeout: 15000 });
  await rowC
    .getByRole("button", { name: "取消或请求停止", exact: true })
    .click();
  await expect(rowC.getByTestId("remote-document-status")).toHaveText("已取消");
  await expect(
    rowC.getByRole("button", { name: "取消或请求停止" }),
  ).toHaveCount(0);
  // D: admitted while online, then the desktop goes dark for the lease test.
  const docD = await share("离线.txt", "准入后电脑离线的摘录");
  const rowD = row("离线.txt");
  phone.once("dialog", (dialog) => dialog.accept());
  await rowD
    .getByRole("button", { name: "确认电脑保存这份摘录", exact: true })
    .click();
  await expect(rowD.getByTestId("remote-document-status")).toContainText(
    "已确认，等待电脑保存",
  );
  await admit(docD);
  await beat();
  await refreshDocuments();
  await expect(rowD.getByTestId("remote-document-status")).toHaveText(
    "执行中（推断）",
  );
  await phone.screenshot({
    path: `${output}mobile-documents.png`,
    fullPage: true,
  });
  // Stop beating and outlive the 15s lease: the phone must show offline with
  // the last-contact stamp (the agreed timeout is lease + poll) — and the
  // admitted task must switch from 执行中（推断） to the offline-executing label.
  await new Promise((resolve) => setTimeout(resolve, 15500));
  await phone.getByRole("button", { name: "刷新配对", exact: true }).click();
  await expect(phone.locator(".pairing .presence")).toContainText("电脑离线");
  await expect(phone.locator(".pairing .presence")).toContainText(
    "最后联系",
  );
  // Refresh the document panel too instead of waiting for its 5s tick.
  await refreshDocuments();
  await expect(rowD.getByTestId("remote-document-status")).toHaveText(
    "已开始，电脑离线",
  );
  await expect(rowD.getByTestId("remote-document-phase-hint")).toContainText(
    "恢复联网后会自动核对",
  );
  // Recovery: one beat flips it back online — and a revoked old session can
  // never beat again (its token died with the phone logout at the end).
  await beat();
  await phone.getByRole("button", { name: "刷新配对", exact: true }).click();
  await expect(phone.locator(".pairing .presence")).toContainText("电脑在线");
  await refreshDocuments();
  await expect(rowD.getByTestId("remote-document-status")).toHaveText(
    "执行中（推断）",
  );
  // Service unreachable ≠ offline: the banner says status unknown and keeps
  // the last successful rows — in BOTH panels.
  await mobile.setOffline(true);
  await phone.getByRole("button", { name: "刷新配对", exact: true }).click();
  // Scoped to the banner: the pairing-success note from the accept above
  // legitimately persists as a second status line.
  await expect(phone.locator(".pairing p[role=status].notice")).toContainText(
    "无法连接服务，电脑在线状态未知",
  );
  await expect(phone.locator(".pairing .task-list li")).not.toHaveCount(0);
  await refreshDocuments();
  await expect(
    documents.locator("p[role=status].notice"),
  ).toContainText("无法连接服务，任务状态未知");
  await expect(
    documents.locator("article").filter({ hasText: "离线.txt" }),
  ).toHaveCount(1);
  await mobile.setOffline(false);
  await phone.getByRole("button", { name: "刷新配对", exact: true }).click();
  await expect(phone.locator(".pairing p[role=status].notice")).toHaveCount(0);
  await refreshDocuments();
  await expect(documents.locator("p[role=status].notice")).toHaveCount(0);
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
    "PASS: two device logins share companion and tasks; account isolation, cookie recovery, lost-response dedup, reconnect, cancellation, device presence lease, six-state document progress with recovery, stale response protection and logout verified.",
  );
} finally {
  await browser.close();
  server.close();
  server.closeAllConnections();
}
