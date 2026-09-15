import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    const history = [{ user: '之前的问题', assistant: ('之前的回答，需要逐段阅读。\n').repeat(70) + 'x'.repeat(240) }];
    let serial = 0;
    w.requestCount = 0;
    w.failHistoryOnce = false;
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => ++serial,
      unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'get_runtime_info') return { protocolVersion: 1, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'chat_config') return { configured: true, model: 'reading-fixture', maxOutputTokens: 1024 };
        if (cmd === 'chat_history') { if (w.failHistoryOnce) { w.failHistoryOnce = false; throw Error('fixture history read failed'); } return history; }
        if (cmd === 'chat_clear') { history.length = 0; return; }
        if (cmd === 'chat_cancel') { w.streamFixture.fail(); return; }
        if (cmd === 'chat_generate') {
          w.requestCount++;
          return new Promise((resolve, reject) => {
            let reply = '';
            w.streamFixture = {
              part: (text: string) => { reply += text; args.onDelta.onmessage({ requestId: args.request.requestId, text }); },
              fail: () => reject('测试连接中断'),
              finish: () => { history.push({ user: args.request.prompt, assistant: reply }); resolve({ requestId: args.request.requestId, elapsedMs: 400, usage: { total_tokens: 10 } }); }
            };
          });
        }
        return 1;
      }
    } });
  });
  await page.goto('/');
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByRole('button', { name: '聊一聊', exact: true }).click();
  await page.getByRole('button', { name: '展开阅读', exact: true }).click();
});

test('reading fits pet window and new text respects manual scrolling', async ({ page }) => {
  const reader = page.getByRole('region', { name: '对话阅读区' });
  await expect(reader).toContainText('之前的问题');
  const bounds = await page.locator('.pet-dialog').boundingBox();
  expect(bounds!.height).toBeGreaterThan(350);
  const overflow = await reader.evaluate(e => e.scrollWidth - e.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await page.getByLabel('和栖栖说句话').fill('请继续');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(page.locator('.chat-phase')).toHaveText('等待回复');
  await reader.evaluate(e => { e.scrollTop = 0; e.dispatchEvent(new Event('scroll')); });
  await expect(page.getByRole('button', { name: '回到最新 ↓' })).toBeVisible();
  await page.evaluate(() => (window as any).streamFixture.part('新增回复\n'.repeat(40)));
  await expect(page.locator('.chat-phase')).toHaveText('正在回复');
  expect(await reader.evaluate(e => e.scrollTop)).toBe(0);
  await page.getByRole('button', { name: '回到最新 ↓' }).click();
  expect(await reader.evaluate(e => e.scrollHeight - e.clientHeight - e.scrollTop)).toBeLessThan(2);
  await page.evaluate(() => (window as any).streamFixture.finish());
  await expect(page.locator('.chat-phase')).toHaveText('已完成');
  await expect(reader.locator('.chat-turn')).toHaveCount(2);
  await expect(reader).toContainText('已加入前文');
  await page.screenshot({ path: test.info().outputPath('reading.png') });
  await page.getByRole('button', { name: '收回气泡', exact: true }).click();
  await expect(reader).toHaveCount(0);
  await expect(page.locator('.pet-dialog')).not.toHaveClass(/pet-dialog-reading/);
  expect((await page.locator('.pet-dialog').boundingBox())!.height).toBeLessThanOrEqual(224);
});

test('partial failure stays distinct from completed reply whose history refresh fails', async ({ page }) => {
  await page.getByLabel('和栖栖说句话').fill('重试的问题');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await page.evaluate(() => { (window as any).streamFixture.part('只有半句'); (window as any).streamFixture.fail(); });
  await expect(page.locator('.chat-phase')).toHaveText('未完成');
  await expect(page.getByRole('region', { name: '对话阅读区' })).toContainText('未加入前文');
  await expect(page.getByLabel('和栖栖说句话')).toHaveValue('重试的问题');
  await expect(page.getByText('最近 1 轮', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await page.evaluate(() => { const w = window as any; w.streamFixture.part('完整的新回复'); w.failHistoryOnce = true; w.streamFixture.finish(); });
  await expect(page.locator('.chat-phase')).toHaveText('已完成');
  await expect(page.getByRole('status')).toContainText('回复已完成，但记录读取失败');
  expect(await page.evaluate(() => (window as any).requestCount)).toBe(2);
  await page.getByRole('button', { name: '收起气泡', exact: true }).click();
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('完整的新回复');
  await expect(page.getByText('最近 2 轮', { exact: true })).toBeVisible();
});