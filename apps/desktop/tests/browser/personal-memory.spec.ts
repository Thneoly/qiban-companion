import { test, expect } from '@playwright/test';

const OFFLINE = { online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' };

function record(id: number, patch: Record<string, unknown> = {}) {
  return {
    id, seq: id, type: 'decision', project: 'R2R', title: `决定 ${id}`, content: '内容正文。',
    importance: 5, createdAt: '2026-09-25 02:10:12', updatedAt: '2026-09-25 02:10:12',
    validUntil: null, supersededBy: null, contradicts: null, tags: ['论文'], origin: 'mcp', ...patch,
  };
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    // Configurable so the preview test can turn the surface back into a
    // plain browser page.
    Object.defineProperty(window, 'isTauri', { value: true, configurable: true });
    const callbacks = new Map(); const events = new Map(); let sequence = 0;
    w.personalOverview = { online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' };
    w.personalResults = null; w.personalDetail = null; w.personalCalls = [];
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (callback: any) => { callbacks.set(++sequence, callback); return sequence; }, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') { events.set(args.event, callbacks.get(args.handler)); return ++sequence; }
        if (cmd === 'get_runtime_info') return { protocolVersion: 2, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'personal_memory_overview') { w.personalCalls.push(['overview']); return structuredClone(w.personalOverview); }
        if (cmd === 'personal_memory_recall') { w.personalCalls.push(['recall', args]); return structuredClone(w.personalResults); }
        if (cmd === 'personal_memory_detail') { w.personalCalls.push(['detail', args]); return structuredClone(w.personalDetail); }
        return 1;
      },
    } });
  });
});

test('offline state names the service and never shows sample data', async ({ page }) => {
  await page.goto('/?view=panel');
  const panel = page.getByRole('region', { name: '个人记忆' });
  await expect(panel.getByRole('status').filter({ hasText: '个人记忆服务未运行' })).toBeVisible();
  await expect(panel).toContainText('http://127.0.0.1:4322');
  await expect(panel).toContainText('不会显示示例数据');
  // Offline replaces the whole interaction area: no search form to misuse.
  await expect(panel.getByRole('button', { name: '检索' })).toHaveCount(0);
  await expect(panel.locator('.personal-memory-card')).toHaveCount(0);
});

test('online search shows cards and the supersession chain detail', async ({ page }) => {
  const superseded = record(1, { supersededBy: 2, seq: 3 });
  const current = record(2, { title: '决定 2（现行）', seq: 4 });
  await page.goto('/?view=panel');
  const panel = page.getByRole('region', { name: '个人记忆' });
  // The page starts offline (the init default); flip the fixture and reconnect.
  await page.evaluate(([overview, results, detail]) => {
    const w = window as any;
    w.personalOverview = overview;
    w.personalResults = results;
    w.personalDetail = detail;
  }, [{
    online: true,
    stats: { total: 2, active: 1, superseded: 1, byType: { decision: 1 }, byProject: { '(global)': 1 } },
    serviceUrl: 'http://127.0.0.1:4322',
  }, { count: 2, memories: [superseded, current] }, { memory: superseded, chain: [superseded, current] }]);
  await panel.getByRole('button', { name: '重新连接' }).click();
  await expect(panel).toContainText('共 2 条 · 活跃 1 · 已取代 1');
  await panel.getByLabel('关键词').fill('决定');
  await panel.getByRole('button', { name: '检索', exact: true }).click();
  await expect(panel.locator('.personal-memory-card')).toHaveCount(2);
  await expect(panel.locator('.personal-memory-card').first()).toContainText('已被取代');
  await panel.locator('.personal-memory-card').first().getByRole('button', { name: '查看详情' }).click();
  await expect(panel.locator('.personal-memory-detail ol')).toBeVisible();
  await expect(panel.locator('.personal-memory-detail')).toContainText('当前查看');
  await expect(panel.locator('.personal-memory-detail')).toContainText('→ 被 #2 取代');
  const calls = await page.evaluate(() => (window as any).personalCalls);
  expect(calls.some((call: unknown[]) => call[0] === 'recall' && (call[1] as { query?: string }).query === '决定')).toBe(true);
});

test('browser preview shows a preview-only note and issues no IPC', async ({ page }) => {
  // Registered after the beforeEach mock, so it wins and turns the surface
  // back into a plain browser preview.
  await page.addInitScript(() => { Object.defineProperty(window, 'isTauri', { value: false, configurable: true }); });
  await page.goto('/?view=panel');
  const panel = page.getByRole('region', { name: '个人记忆' });
  await expect(panel).toContainText('浏览器仅预览界面，不连接个人记忆服务');
  await expect(panel.getByRole('button', { name: '重新连接' })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).personalCalls.length)).toBe(0);
});
