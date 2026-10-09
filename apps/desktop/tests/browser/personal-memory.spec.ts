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
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
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
  // Recall results only ever contain ACTIVE rows (the service filters
  // superseded/expired); supersession states appear in the detail chain.
  const superseded = record(1, { supersededBy: 2, seq: 3 });
  const current = record(2, { title: '决定 2（现行）', seq: 4 });
  const another = record(3, { title: '决定 3', seq: 5, type: 'fact' });
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
    stats: { total: 3, active: 2, superseded: 1, byType: { decision: 1, fact: 1 }, byProject: { R2R: 2 } },
    serviceUrl: 'http://127.0.0.1:4322',
  }, { count: 2, memories: [current, another] }, { memory: current, chain: [superseded, current] }]);
  await panel.getByRole('button', { name: '重新连接' }).click();
  await expect(panel).toContainText('共 3 条 · 活跃 2 · 已取代 1');
  await panel.getByLabel('关键词').fill('决定');
  await panel.getByRole('button', { name: '检索', exact: true }).click();
  await expect(panel.locator('.personal-memory-card')).toHaveCount(2);
  await panel.locator('.personal-memory-card').first().getByRole('button', { name: '查看详情' }).click();
  await expect(panel.locator('.personal-memory-detail ol')).toBeVisible();
  await expect(panel.locator('.personal-memory-detail')).toContainText('当前查看');
  await expect(panel.locator('.personal-memory-detail')).toContainText('已被取代');
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

test('injection policy panel: selection, budget guard, removal confirmation, offline disable', async ({ page }) => {
  const personalRecord = (id: number, title: string) => ({
    id, seq: id, type: 'insight', project: null, title, content: `${title}的内容`, importance: 3,
    createdAt: '2026-09-25 02:10:12', updatedAt: '2026-09-25 02:10:12', validUntil: null,
    supersededBy: null, contradicts: null, tags: [], origin: 'mcp',
  });
  await page.addInitScript(([records]) => {
    const w = window as any;
    w.policyState = { enabled: false, revision: 0, selectedIds: [] as number[], epoch: 1 };
    w.policyWrites = [];
    w.personalResults = { count: records.length, memories: records };
    const preview = () => ({
      scope: { baseUrl: 'https://fixture.test', model: 'glm-test-fixture' },
      contextEpoch: w.policyState.epoch,
      policy: { enabled: false, revision: 0, selectedIds: [] },
      items: [], bodyChars: 0, contextChars: 0,
      personal: {
        status: w.policyState.selectedIds.length ? 'online' : 'offline',
        policy: { ...w.policyState },
        items: w.policyState.selectedIds.map((id: number) => records.find((r: any) => r.id === id)).filter(Boolean),
        inactiveSelectedIds: [],
        bodyChars: w.policyState.selectedIds.reduce((n: number, id: number) => {
          const record = records.find((r: any) => r.id === id);
          return n + (record ? [...record.content].length : 0);
        }, 0),
        contextChars: w.policyState.selectedIds.length * 100,
      },
    });
    const inner = (window as any).__TAURI_INTERNALS__;
    const baseInvoke = inner.invoke;
    inner.invoke = async (cmd: string, args: any) => {
      if (cmd === 'chat_context_preview') return structuredClone(preview());
      if (cmd === 'personal_memory_policy_set') {
        const r = args.request;
        if (r.expectedRevision !== w.policyState.revision || r.expectedEpoch !== w.policyState.epoch) {
          if (r.expectedRevision !== w.policyState.revision) throw { code: 'conflict' };
          throw { code: 'context_changed' };
        }
        const removing = w.policyState.selectedIds.some((id: number) => !r.selectedIds.includes(id));
        if (removing && r.restartConversation !== true) throw { code: 'confirmation_required' };
        if (r.enabled && r.selectedIds.reduce((n: number, id: number) => { const record = records.find((m: any) => m.id === id); return n + (record ? [...record.content].length : 0); }, 0) > 800) throw { code: 'selection_too_large' };
        w.policyWrites.push(r);
        w.policyState = { enabled: r.enabled && r.selectedIds.length > 0, revision: w.policyState.revision + 1, selectedIds: r.selectedIds, epoch: w.policyState.epoch + 1 };
        return { contextEpoch: w.policyState.epoch, chatCleared: removing, clearedTurns: removing ? 1 : 0, notificationsDelivered: true };
      }
      return baseInvoke(cmd, args);
    };
  }, [[personalRecord(1, '洞察一'), personalRecord(2, '洞察二'), personalRecord(3, '洞察三')]]);
  await page.goto('/?view=panel');
  const panel = page.getByRole('region', { name: '个人记忆' });
  const section = page.locator('.personal-memory-injection');
  await expect(section).toContainText('已保存状态：关闭');

  // Candidates come from the search results; bring the panel online first.
  await page.evaluate(() => {
    const w = window as any;
    w.personalOverview = {
      online: true,
      stats: { total: 3, active: 3, superseded: 0, byType: { insight: 3 }, byProject: { '(global)': 3 } },
      serviceUrl: 'http://127.0.0.1:4322',
    };
  });
  await panel.getByRole('button', { name: '重新连接' }).click();
  await expect(panel).toContainText('共 3 条');
  await panel.getByRole('button', { name: '检索', exact: true }).click();
  await expect(panel.locator('.personal-memory-card')).toHaveCount(3);

  // Enable + pick two in order.
  await section.getByText('允许此模型使用所选个人记忆').check();
  await section.getByText('#1 洞察一').check();
  await section.getByText('#2 洞察二').check();
  await expect(section).toContainText('2 / 5 条');
  await section.getByRole('button', { name: '保存选择' }).click();
  await expect(section).toContainText('已保存状态：启用 · 2条');
  expect(await page.evaluate(() => (window as any).policyWrites[0].selectedIds)).toEqual([1, 2]);

  // Removal requires the confirm dialog; cancelling does not write.
  await section.getByText('#1 洞察一').uncheck();
  await section.getByRole('button', { name: '保存选择' }).click();
  await expect(section.getByRole('alertdialog')).toContainText('全部模型的聊天');
  await section.getByRole('button', { name: '返回，不修改' }).click();
  expect(await page.evaluate(() => (window as any).policyWrites.length)).toBe(1);

  // Disable: clearing the master checkbox empties the selection, which is a
  // removal — the same confirm dialog applies, then the write clears chats.
  await section.getByText('允许此模型使用所选个人记忆').uncheck();
  await section.getByRole('button', { name: '保存选择' }).click();
  await expect(section.getByRole('alertdialog')).toBeVisible();
  await section.getByRole('button', { name: '确认收回并清空聊天' }).click();
  await expect(section).toContainText('已保存状态：关闭');
  expect(await page.evaluate(() => (window as any).policyWrites.length)).toBe(2);
  expect(await page.evaluate(() => (window as any).policyWrites[1].restartConversation)).toBe(true);
});
