import { test, expect } from '@playwright/test';

test('an already open panel refreshes pet tasks without duplicating its own creation', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    let next = 0;
    const callbacks = new Map<number, (value: unknown) => void>();
    const tasks: any[] = [];
    Object.defineProperty(window, 'isTauri', { value: true });
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    const task = (title: string) => ({ id: crypto.randomUUID(), title, status: 'queued', createdAt: Date.now(), updatedAt: Date.now(), revision: 0 });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (callback: (value: unknown) => void) => { callbacks.set(++next, callback); return next; },
      unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 2, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return tasks;
        if (cmd === 'personal_memory_overview') return {online:false,stats:null,serviceUrl:'http://127.0.0.1:4322'};
        if (cmd === 'model_settings_get') return { baseUrl: 'https://example.com/v1', model: 'test', useApiKey: false, hasApiKey: false, maxOutputTokens: 1024 };
        if (cmd === 'plugin:event|listen' && args.event === 'panel-refresh') {
          w.petCreatedTask = () => { tasks.unshift(task('来自桌宠的待办')); callbacks.get(args.handler)?.({ event: 'panel-refresh', payload: null }); };
        }
        if (cmd === 'create_task') { const created = task(args.title); tasks.unshift(created); return created; }
        return 1;
      },
    } });
  });
  await page.goto('/?view=panel');
  await expect(page.getByText('留一点空间，给下一个好想法')).toBeVisible();
  await page.evaluate(() => (window as any).petCreatedTask());
  await expect(page.getByText('来自桌宠的待办', { exact: true })).toHaveCount(1);
  await page.getByLabel('任务标题', { exact: true }).fill('面板自己创建的待办');
  await page.getByRole('button', { name: '记下来', exact: true }).click();
  await expect(page.getByText('面板自己创建的待办', { exact: true })).toHaveCount(1);
});
