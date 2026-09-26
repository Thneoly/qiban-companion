import { test, expect } from '@playwright/test';

test('default entry is a pet; bubble records a task and opens the auxiliary panel', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByRole('button', { name:'和栖栖互动' })).toBeVisible();
  await expect(page.getByRole('heading', { name:'今天，也一起慢慢来。' })).toHaveCount(0);
  await expect(page.getByRole('region', { name:'栖栖的交互气泡' })).toHaveCount(0);
  await page.getByRole('button', { name:'和栖栖互动' }).click();
  await expect(page.getByRole('region', { name:'栖栖的交互气泡' })).toBeVisible();
  await expect(page.getByLabel('想记下什么？', { exact:true })).toBeEnabled();
  await page.getByLabel('想记下什么？', { exact:true }).fill('从角色气泡记录的想法');
  await page.getByRole('button', { name:'记下来', exact:true }).click();
  await expect(page.getByRole('status')).toContainText('已经记下了');
  await page.screenshot({ path:test.info().outputPath('pet-preview.png'), fullPage:true });
  await page.getByRole('button', { name:'任务面板 ↗' }).click();
  await expect(page.getByRole('heading', { name:'今天，也一起慢慢来。' })).toBeVisible();
  await expect(page.getByText('从角色气泡记录的想法', { exact:true })).toBeVisible();
  await page.getByRole('button', { name:'← 回到桌面角色预览' }).click();
  await expect(page.getByRole('button', { name:'和栖栖互动' })).toBeVisible();
  expect(errors).toEqual([]);
});

test('character drag moves the scene without clicking, while a small move and keyboard still open chat', async ({ page }) => {
  await page.goto('/');
  const character = page.getByRole('button', { name: '和栖栖互动' });
  const scene = page.getByRole('group', { name: '栖栖的小天地，可拖动背景移动' });
  const before = (await scene.boundingBox())!;
  const body = (await character.boundingBox())!;
  await page.mouse.move(body.x + 90, body.y + 110);
  await page.mouse.down();
  await page.mouse.move(body.x + 15, body.y + 35, { steps: 10 });
  await page.mouse.up();
  expect((await scene.boundingBox())!.x).toBeCloseTo(before.x - 75, 0);
  expect((await scene.boundingBox())!.y).toBeCloseTo(before.y - 75, 0);
  await expect(page.getByRole('region', { name: '栖栖的交互气泡' })).toHaveCount(0);
  const moved = (await character.boundingBox())!;
  await page.mouse.move(moved.x + 90, moved.y + 110);
  await page.mouse.down();
  await page.mouse.move(moved.x + 92, moved.y + 112);
  await page.mouse.up();
  await expect(page.getByRole('region', { name: '栖栖的交互气泡' })).toBeVisible();
  await page.keyboard.press('Escape');
  await character.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: '栖栖的交互气泡' })).toBeVisible();
});

test('dialog controls do not drag; cancelling a scene gesture releases movement', async ({ page }) => {
  await page.goto('/');
  const scene = page.getByRole('group', { name: '栖栖的小天地，可拖动背景移动' });
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  const before = (await scene.boundingBox())!;
  const field = page.getByLabel('想记下什么？', { exact: true });
  await field.fill('可以选择文字');
  const input = (await field.boundingBox())!;
  await page.mouse.move(input.x + 20, input.y + 10); await page.mouse.down();
  await page.mouse.move(input.x + 90, input.y + 10); await page.mouse.up();
  expect((await scene.boundingBox())!.x).toBe(before.x);
  await page.getByRole('button', { name: '记下来', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('已经记下了');
  await page.keyboard.press('Escape');
  await page.mouse.move(before.x + 18, before.y + 40); await page.mouse.down();
  await scene.dispatchEvent('pointercancel', { pointerId: 1 });
  await page.mouse.move(before.x - 30, before.y + 10); await page.mouse.up();
  expect((await scene.boundingBox())!.x).toBe(before.x);
  await page.screenshot({ path: test.info().outputPath('scene.png'), fullPage: true });
});

test('quiet and hidden pet can be recovered; dragging does not open the bubble', async ({ page }) => {
  await page.setViewportSize({ width:390,height:844 });
  await page.goto('/');
  const character=page.getByRole('button',{name:'和栖栖互动'});
  const handle=page.getByRole('group',{name:'栖栖的小天地，可拖动背景移动'});
  const before=await handle.boundingBox();
  if(!before) throw new Error('Missing drag handle');
  await page.mouse.move(before.x+15,before.y+45);
  await page.mouse.down();
  await page.mouse.move(before.x+15,before.y-35,{steps:8});
  await page.mouse.up();
  const after=await handle.boundingBox();
  expect(after!.y).toBeLessThan(before.y-50);
  await expect(page.getByRole('region',{name:'栖栖的交互气泡'})).toHaveCount(0);
  await character.click();
  await page.getByRole('button',{name:'安静陪伴',exact:true}).click();
  await expect(character).toBeDisabled();
  await page.getByRole('button',{name:'恢复角色预览'}).click();
  await expect(character).toBeEnabled();
  await character.click();
  await page.getByRole('button',{name:'隐藏',exact:true}).click();
  await expect(character).toBeHidden();
  await page.getByRole('button',{name:'恢复角色预览'}).click();
  await expect(character).toBeVisible();
  await character.click();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('region',{name:'栖栖的交互气泡'})).toHaveCount(0);
});

test('native handoff starts once after threshold and includes the whole scene in hit regions', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.dragCalls = 0; w.hitRegions = [];
    Object.defineProperty(window, 'isTauri', { value: true });
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        if (command === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (command === 'guide_status') return true;
        if (command === 'pet_action' && args.action === 'drag') w.dragCalls++;
        if (command === 'set_pet_regions') w.hitRegions = args.regions;
        return 1;
      },
    } });
  });
  await page.setViewportSize({ width: 320, height: 440 });
  await page.goto('/');
  const character = page.getByRole('button', { name: '和栖栖互动' });
  await expect(character).toBeEnabled();
  await expect.poll(() => page.evaluate(() => (window as any).hitRegions.some((r: any) => r.width === 242 && r.height === 64))).toBe(true);
  const body = (await character.boundingBox())!;
  await page.mouse.move(body.x + 90, body.y + 100); await page.mouse.down();
  await page.mouse.move(body.x + 92, body.y + 102);
  expect(await page.evaluate(() => (window as any).dragCalls)).toBe(0);
  await page.mouse.move(body.x + 115, body.y + 110, { steps: 5 }); await page.mouse.up();
  expect(await page.evaluate(() => (window as any).dragCalls)).toBe(1);
  await expect(page.getByRole('region', { name: '栖栖的交互气泡' })).toHaveCount(0);
  await character.click();
  await expect(page.getByLabel('想记下什么？', { exact: true })).toBeVisible();
});
