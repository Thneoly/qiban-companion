import { test, expect } from "@playwright/test";

test("preview never offers real account credential access", async ({
  page,
}) => {
  await page.goto("/?view=panel");
  await expect(
    page.getByRole("button", { name: "获取登录验证码", exact: true }),
  ).toBeDisabled();
  await expect(page.locator(".account-notice")).toContainText(
    "浏览器预览不读写系统凭据",
  );
});

test("native account UI separates local data, deduplicates retry and clears late responses after logout", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, "isTauri", { value: true });
    Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
      value: { unregisterListener: () => {} },
    });
    let authenticated = false;
    let pendingLogout = false;
    let failedOnce = false;
    const tasks: any[] = [];
    const pairings: any[] = [];
    const documents: any[] = [];
    let prepared: any;
    w.documentShares = 0;
    w.connectPair = () => {
      const p = pairings[0];
      p.controllerId = crypto.randomUUID();
      p.controllerName = "测试手机";
      p.status = "active";
      p.revision++;
    };
    const requestIds = new Map();
    const snapshot = () => ({
      port: 4318,
      status: pendingLogout
        ? "logout_pending"
        : authenticated
          ? "authenticated"
          : "signed_out",
      profile: authenticated
        ? {
            accountId: "ea80aff6-a661-49d2-8175-74ec3c67dc21",
            companionId: "e8b6c3d9-694f-4b13-86f7-2bd8c7b9cd6d",
          }
        : null,
      tasks: authenticated ? structuredClone(tasks) : [],
    });
    w.holdSnapshot = false;
    w.offlineLogout = false;
    w.requests = [];
    w.finishRevocation = () => {
      pendingLogout = false;
    };
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (cmd: string, args: any) => {
          if (cmd === "get_runtime_info")
            return {
              protocolVersion: 3,
              appVersion: "test",
              runtime: "desktop",
              persistence: "sqlite",
              executorAvailable: true,
            };
          if (cmd === "list_tasks")
            return [
              {
                id: "local",
                title: "只存在本机的手记",
                status: "queued",
                createdAt: 1,
                updatedAt: 1,
                revision: 0,
              },
            ];
          if (cmd === "model_settings_get")
            return {
              baseUrl: "https://example.com/v1",
              model: "test",
              useApiKey: false,
              hasApiKey: false,
              maxOutputTokens: 1024,
            };
          if (cmd === "account_snapshot") {
            const value = snapshot();
            if (w.holdSnapshot) {
              w.holdSnapshot = false;
              return new Promise((resolve) => {
                w.deliverOldSnapshot = () => resolve(value);
              });
            }
            return value;
          }
          if (cmd === "remote_document_sync") return structuredClone(documents);
          if (cmd === "remote_document_prepare") {
            const actionId = crypto.randomUUID();
            prepared = {
              id: crypto.randomUUID(),
              actionId,
              sourceName: args.sourceName,
              preview: args.text,
              artifactName: `${actionId}.md`,
              artifactHash: "a".repeat(64),
              status: "waiting_confirmation",
              revision: 0,
              createdAt: 1,
              updatedAt: 1,
              note: "",
            };
            return { task: structuredClone(prepared) };
          }
          if (cmd === "remote_document_share") {
            w.documentShares++;
            const d = {
              authorization: {
                pairingId: pairings[0].id,
                binding: {
                  actionId: prepared.actionId,
                  resourceId: prepared.id,
                  resourceVersion: 1,
                  parametersDigest: "b".repeat(64),
                  pairRevision: pairings[0].revision,
                  scope: "document_excerpt",
                },
                expiresAt: Date.now() + 300000,
                state: "awaiting_confirmation",
              },
              sourceName: prepared.sourceName,
              preview: prepared.preview,
              artifactHash: prepared.artifactHash,
              currentRole: "desktop",
            };
            documents.push(d);
            return structuredClone(d);
          }
          if (cmd === "account_pairings") return structuredClone(pairings);
          if (cmd === "account_pairing_offer") {
            const p = {
              id: crypto.randomUUID(),
              desktopId: crypto.randomUUID(),
              desktopName: args.name,
              controllerId: null,
              controllerName: null,
              scope: "document_excerpt",
              revision: 1,
              status: "pending",
              expiresAt: Date.now() + 300000,
              currentRole: "desktop",
            };
            pairings.push(p);
            return { pairing: structuredClone(p), code: "a".repeat(32) };
          }
          if (cmd === "account_pairing_revoke") {
            pairings[0].status = "revoked";
            pairings[0].revision++;
            return;
          }
          if (cmd === "account_code_request") return;
          if (cmd === "account_login") {
            if (args.code !== "12345678")
              throw { code: "invalid_code", message: "验证码无效或已过期" };
            authenticated = true;
            return snapshot();
          }
          if (cmd === "account_task_create") {
            w.requests.push(args.requestId);
            if (!requestIds.has(args.requestId)) {
              const task = {
                id: crypto.randomUUID(),
                title: args.title,
                status: "queued",
                createdAt: Date.now(),
                updatedAt: Date.now(),
                revision: 0,
              };
              requestIds.set(args.requestId, task);
              tasks.push(task);
            }
            if (!failedOnce) {
              failedOnce = true;
              throw {
                code: "unavailable",
                message: "操作结果尚未确认，请重试",
              };
            }
            return requestIds.get(args.requestId);
          }
          if (cmd === "account_task_cancel") {
            const task = tasks.find((t) => t.id === args.id);
            task.status = "cancelled";
            task.revision++;
            return task;
          }
          if (cmd === "account_logout") {
            authenticated = false;
            pendingLogout = w.offlineLogout;
            return snapshot();
          }
          return 1;
        },
      },
    });
  });
  await page.goto("/?view=panel");
  const panel = page.locator(".account-panel");
  await page.getByLabel("账号邮箱", { exact: true }).fill("alice@example.com");
  await page
    .getByRole("button", { name: "获取登录验证码", exact: true })
    .click();
  await page.getByLabel("8 位登录验证码", { exact: true }).fill("00000000");
  await page
    .getByRole("button", { name: "登录并接续伙伴", exact: true })
    .click();
  await expect(panel.getByRole("status")).toContainText("验证码无效");
  await page.getByLabel("8 位登录验证码", { exact: true }).fill("12345678");
  await page
    .getByRole("button", { name: "登录并接续伙伴", exact: true })
    .click();
  await expect(page.getByTestId("desktop-companion-id")).toHaveText(
    "e8b6c3d9-694f-4b13-86f7-2bd8c7b9cd6d",
  );
  await expect(panel).not.toContainText("只存在本机的手记");
  await expect(page.locator(".task-panel")).toContainText("只存在本机的手记");
  await page
    .getByLabel("新增共享待办", { exact: true })
    .fill("手机也能看到的事");
  await page.getByRole("button", { name: "保存共享待办", exact: true }).click();
  await expect(panel.getByRole("status")).toContainText("操作结果尚未确认");
  await page.getByRole("button", { name: "保存共享待办", exact: true }).click();
  await expect(panel.locator(":scope > .account-tasks li")).toHaveCount(1);
  const ids = await page.evaluate(() => (window as any).requests);
  expect(ids).toHaveLength(2);
  expect(ids[0]).toBe(ids[1]);
  await page.screenshot({
    path: "test-results/desktop-account.png",
    fullPage: true,
  });
  await page
    .getByRole("button", {
      name: "取消共享待办：手机也能看到的事",
      exact: true,
    })
    .click();
  await expect(panel.locator(":scope > .account-tasks")).toContainText(
    "已取消",
  );
  const pairing = panel.locator(".pairing-panel");
  await pairing
    .getByRole("button", { name: "生成五分钟配对码", exact: true })
    .click();
  await expect(pairing.getByTestId("pairing-code")).toHaveText("a".repeat(32));
  await expect(pairing).toContainText("等待手机确认");
  await page.evaluate(() => (window as any).connectPair());
  await pairing.getByRole("button", { name: "刷新配对", exact: true }).click();
  await expect(pairing).toContainText("已配对");
  await expect(pairing.getByTestId("pairing-code")).toHaveCount(0);
  await pairing.screenshot({ path: "test-results/desktop-pairing.png" });
  const remote = panel.locator(".remote-documents");
  await remote
    .getByRole("button", { name: "刷新文档协作", exact: true })
    .click();
  await expect(remote.locator("select option")).toHaveCount(2);
  await remote.getByLabel("接收确认的手机").selectOption({ index: 1 });
  await remote
    .getByLabel("选择要分享摘录的文档")
    .setInputFiles({
      name: "桌面测试.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("只分享用户核对过的摘录"),
    });
  await remote
    .getByRole("button", { name: "生成本地预览", exact: true })
    .click();
  await expect(remote.getByLabel("待分享摘录预览")).toContainText(
    "只分享用户核对过的摘录",
  );
  expect(await page.evaluate(() => (window as any).documentShares)).toBe(0);
  await expect(remote.locator("article")).toHaveCount(0);
  await remote
    .getByRole("button", { name: "分享预览并等待手机确认", exact: true })
    .click();
  await expect(remote.locator("article")).toContainText("等待手机确认");
  expect(await page.evaluate(() => (window as any).documentShares)).toBe(1);
  await remote.screenshot({ path: "test-results/remote-document-desktop.png" });
  page.once("dialog", (dialog) => dialog.accept());
  await pairing
    .getByRole("button", { name: "撤销配对 我的电脑", exact: true })
    .click();
  await expect(pairing).toContainText("已撤销");
  await page.evaluate(() => {
    (window as any).holdSnapshot = true;
    (window as any).offlineLogout = true;
  });
  await page.getByRole("button", { name: "刷新账号", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => typeof (window as any).deliverOldSnapshot))
    .toBe("function");
  await page
    .getByRole("button", { name: "退出此桌面会话", exact: true })
    .click();
  await expect(panel.getByRole("status")).toContainText("服务端撤销待连接恢复");
  await page.evaluate(() => (window as any).deliverOldSnapshot());
  await expect(panel.locator(":scope > .account-tasks li")).toHaveCount(0);
  await expect(page.getByTestId("desktop-companion-id")).toHaveCount(0);
  await expect(page.locator(".task-panel")).toContainText("只存在本机的手记");
  await page.evaluate(() => (window as any).finishRevocation());
  await page
    .getByRole("button", { name: "重试服务端撤销", exact: true })
    .click();
  await expect(page.getByLabel("账号邮箱", { exact: true })).toBeVisible();
});
