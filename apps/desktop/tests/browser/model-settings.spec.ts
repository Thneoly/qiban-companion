import { test, expect } from '@playwright/test';
test('output budget is validated, saved, and reloaded without changing provider or model', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    let config = { baseUrl: 'https://example.com/v1', model: 'custom-model', useApiKey: true, hasApiKey: true, maxOutputTokens: 2048, protocol: 'openai' };
    w.savedBudgets = [];
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'personal_memory_overview') return {online:false,stats:null,serviceUrl:'http://127.0.0.1:4322'};
        if (cmd === 'model_settings_get') return config;
        if (cmd === 'model_settings_save') { config = { ...config, ...args.config }; w.savedBudgets.push(args.config); return; }
        return 1;
      }
    } });
  });
  await page.goto('/?view=panel#settings');
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
  expect(await page.evaluate(() => (window as any).savedBudgets)).toEqual([{ baseUrl: 'https://example.com/v1', model: 'custom-model', useApiKey: true, maxOutputTokens: 4096, protocol: 'openai' }]);
  await expect(budget).toHaveValue('4096');
  await expect(page.getByLabel('模型编码', { exact: true })).toHaveValue('custom-model');
});

test('test connection probes the draft values and reports the classified outcome', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    w.probes = [];
    w.probeOutcome = { ok: true, detail: '连接成功：服务接受了地址、密钥与模型编码。测试发送了一条 1 token 的最小请求，可能产生极小消耗。' };
    const config = { baseUrl: 'https://example.com/v1', model: 'custom-model', useApiKey: true, hasApiKey: true, maxOutputTokens: 2048 };
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'personal_memory_overview') return {online:false,stats:null,serviceUrl:'http://127.0.0.1:4322'};
        if (cmd === 'model_settings_get') return config;
        if (cmd === 'model_settings_test') { w.probes.push(args.config); return w.probeOutcome; }
        return 1;
      }
    } });
  });
  await page.goto('/?view=panel#settings');
  const test = page.getByRole('button', { name: '测试连接', exact: true });
  // Draft values ride the probe without saving: edit the model, probe, and
  // assert the wire carried the DRAFT model, not the stored one.
  await page.getByLabel('模型编码', { exact: true }).fill('draft-model');
  await test.click();
  await expect(page.locator('.model-test-ok')).toContainText('连接成功');
  expect(await page.evaluate(() => (window as any).probes)).toEqual([{ baseUrl: 'https://example.com/v1', model: 'draft-model', useApiKey: true, protocol: 'openai' }]);
  // A failing probe renders the honest verdict under the fail style.
  await page.evaluate(() => { (window as any).probeOutcome = { ok: false, detail: '服务拒绝了密钥：API Key 无效、过期或无权限。' }; });
  await test.click();
  await expect(page.locator('.model-test-fail')).toContainText('密钥');
  await expect(page.locator('.model-test-ok')).toHaveCount(0);
});