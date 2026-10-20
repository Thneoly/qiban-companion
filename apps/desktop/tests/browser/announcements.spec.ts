import { test, expect } from '@playwright/test';
test('announcements render as text, mark read, and survive reload', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    w.announcementsSeen = 0;
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string) => {
        if (cmd === 'get_runtime_info')
          return { protocolVersion: 3, appVersion: '0.2.0', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'model_settings_get')
          return { baseUrl: 'https://example.com/v1', model: 'test', useApiKey: false, hasApiKey: false, maxOutputTokens: 1024 };
        if (cmd === 'account_lan_info') return { port: 4318, embedded: false, lanAddresses: [] };
        return 1;
      },
    } });
    const originalFetch = window.fetch.bind(window);
    (window as any).fetch = async (input: any, init?: any) => {
      const url = String(input);
      if (url.includes('announcements.json')) {
        w.announcementsSeen++;
        return new Response(JSON.stringify({
          items: [
            { id: 'a1', date: '2026-10-20', title: '试用开始', body: '第一条公告<b>不应渲染为 HTML</b>' },
            { id: 'a2', date: '2026-10-21', title: '第二条', body: '内容' },
          ],
        }), { status: 200 });
      }
      return originalFetch(input, init);
    };
  });
  await page.goto('/?view=panel#settings');
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  const section = page.getByRole('region', { name: '公告' });
  await expect(section).toContainText('2 条新', { timeout: 15000 });
  await section.getByRole('button', { name: '查看' }).click();
  // Feed text renders as TEXT: the markup characters stay visible, nothing is injected.
  await expect(section).toContainText('不应渲染为 HTML');
  await expect(section.locator('b')).toHaveCount(0);
  await section.getByRole('button', { name: '全部标为已读' }).click();
  // Read state survives a reload via localStorage: the badge stays gone
  // while the section (with its 查看 entry) remains.
  await page.reload();
  await expect(page.getByRole('region', { name: '公告' })).toContainText('查看', { timeout: 15000 });
  await expect(page.getByRole('region', { name: '公告' })).not.toContainText('条新');
});
