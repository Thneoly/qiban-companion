import { test, expect } from '@playwright/test';

test('workspace shows one destination, preserves drafts, and opens a wide conversation', async ({page}) => {
  await page.goto('/?view=panel');
  await expect(page.getByLabel('日常空间', {exact:true})).toBeVisible();
  await expect(page.getByLabel('账号邮箱', {exact:true})).not.toBeVisible();
  await page.getByLabel('任务标题', {exact:true}).fill('尚未提交的想法');
  await page.getByRole('link', {name:'设置',exact:true}).click();
  await expect(page.getByLabel('设置空间', {exact:true})).toBeVisible();
  await expect(page.getByLabel('日常空间', {exact:true})).not.toBeVisible();
  await page.getByRole('link', {name:'日常',exact:true}).click();
  await expect(page.getByLabel('任务标题', {exact:true})).toHaveValue('尚未提交的想法');
  await page.screenshot({path:test.info().outputPath('workspace-home.png'),fullPage:true});
  await page.getByRole('button', {name:'打开文字对话 ↗'}).click();
  await expect(page.locator('.pet-shell-wide')).toBeVisible();
  await expect(page.getByRole('region', {name:'对话阅读区'})).toBeVisible();
  await expect(page.getByText('浏览器预览不调用模型，请运行桌面版')).toBeVisible();
  await page.screenshot({path:test.info().outputPath('wide-chat.png')});
});

test('the desktop surface suppresses the WebView context menu', async ({page}) => {
  await page.addInitScript(() => {
    (window as unknown as { isTauri: boolean }).isTauri = true;
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string) => {
        if (cmd === 'get_runtime_info')
          return { protocolVersion: 3, appVersion: '0.2.0', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        return [];
      },
    } });
  });
  await page.goto('/?view=panel');
  await expect(page.getByLabel('日常空间', {exact:true})).toBeVisible();
  // The listener lives on document; a cancelable event must come back suppressed.
  const prevented = await page.evaluate(() => {
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    document.body.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(prevented).toBe(true);
});

test('navigation is available on narrow screens and supports direct links and back', async ({page}) => {
  await page.setViewportSize({width:390,height:844});
  await page.goto('/?view=panel#memories');
  await expect(page.getByLabel('记忆空间', {exact:true})).toBeVisible();
  await page.getByRole('link', {name:'设备',exact:true}).click();
  await expect(page.getByLabel('设备空间', {exact:true})).toBeVisible();
  await page.goBack();
  await expect(page.getByLabel('记忆空间', {exact:true})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth)).toBe(false);
});
