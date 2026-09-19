import { test, expect } from '@playwright/test';
test('output budget is validated, saved, and reloaded without changing provider or model', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    let config = { baseUrl: 'https://example.com/v1', model: 'custom-model', useApiKey: true, hasApiKey: true, maxOutputTokens: 2048 };
    w.savedBudgets = [];
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 1, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'model_settings_get') return config;
        if (cmd === 'model_settings_save') { config = { ...config, ...args.config }; w.savedBudgets.push(args.config); return; }
        return 1;
      }
    } });
  });
  await page.goto('/?view=panel');
  const budget = page.getByLabel('最大输出 tokens', { exact: true });
  await expect(budget).toHaveValue('2048');
  for (const value of ['', '0', '8193', '256.5']) {
    await budget.fill(value);
    await page.getByRole('button', { name: '保存模型设置', exact: true }).click();
    expect(await budget.evaluate((e: HTMLInputElement) => e.checkValidity())).toBe(false);
    expect(await page.evaluate(() => (window as any).savedBudgets.length)).toBe(0);
  }
  await budget.fill('4096');
  await page.getByRole('button', { name: '保存模型设置', exact: true }).click();
  await expect(page.locator('.model-settings-note')).toContainText('已保存');
  expect(await page.evaluate(() => (window as any).savedBudgets)).toEqual([{ baseUrl: 'https://example.com/v1', model: 'custom-model', useApiKey: true, maxOutputTokens: 4096 }]);
  await expect(budget).toHaveValue('4096');
  await expect(page.getByLabel('模型编码', { exact: true })).toHaveValue('custom-model');
});