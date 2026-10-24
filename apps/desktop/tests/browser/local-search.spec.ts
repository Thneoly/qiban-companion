import { test, expect } from '@playwright/test';

test('local search consent: list, add, remove, and the disclosure sentence', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    w.addCalls = 0; w.removeCalls = [];
    let roots: any[] = [
      { id: 1, path: 'D:\\Projects', label: null, addedAt: 1698000000000 },
    ];
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'personal_memory_overview') return { online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' };
        if (cmd === 'local_roots_get') return roots;
        if (cmd === 'local_roots_add') {
          w.addCalls += 1;
          const result = { status: 'added', root: { id: 2, path: 'E:\\量化', label: null, addedAt: 1698000001000 } };
          roots = [...roots, result.root];
          return result;
        }
        if (cmd === 'local_roots_remove') { w.removeCalls.push(args.id); roots = roots.filter(r => r.id !== args.id); return roots; }
        return 1;
      }
    } });
  });
  await page.goto('/?view=panel#settings');
  const panel = page.getByLabel('本地检索设置', { exact: true });
  await expect(panel).toBeVisible();
  // The privacy sentence is fixed copy, not optional decoration.
  await expect(panel).toContainText('检索结果会发送给你配置的模型服务');
  await expect(panel).toContainText('只读检索');
  await expect(panel.getByRole('listitem')).toContainText('D:\\Projects');
  await page.getByRole('button', { name: '添加目录…' }).click();
  await expect(panel.getByRole('listitem')).toHaveCount(2);
  await expect(panel).toContainText('已添加');
  expect(await page.evaluate(() => (window as any).addCalls)).toBe(1);
  // Removal goes by id and the list reflects the store's answer.
  await panel.getByRole('listitem').filter({ hasText: 'E:\\量化' }).getByRole('button', { name: '移除' }).click();
  await expect(panel.getByRole('listitem')).toHaveCount(1);
  expect(await page.evaluate(() => (window as any).removeCalls)).toEqual([2]);
});
