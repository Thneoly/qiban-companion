import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.actions = []; w.modelCalls = 0;
    Object.defineProperty(window, 'isTauri', { value:true });
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value:{ unregisterListener:()=>{} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value:{
      transformCallback:()=>1, unregisterCallback:()=>{},
      invoke:async (cmd:string,args:any) => {
        if(cmd==='get_runtime_info') return { protocolVersion:2,appVersion:'test',runtime:'desktop',persistence:'sqlite',executorAvailable:false };
        if(cmd==='guide_status') return sessionStorage.getItem('guide-fixture')==='done';
        if(cmd==='guide_complete') { if(w.failSave) throw '保存使用指南状态失败'; sessionStorage.setItem('guide-fixture','done'); return; }
        if(cmd==='chat_config') return {configured:false,model:'fixture',maxOutputTokens:1024};
        if(cmd==='chat_generate'||cmd==='voice_probe') w.modelCalls++;
        if(cmd==='pet_action') w.actions.push(args.action);
        return 1;
      }
    }});
  });
  await page.setViewportSize({width:320,height:440});
  await page.goto('/');
  await page.getByRole('button',{name:'和栖栖互动'}).click();
});

test('first click explains storage and recovery, can open settings without calling a model', async ({ page }) => {
  const guide=page.getByLabel('初次使用指南');
  await expect(guide).toBeVisible();
  await expect(guide).toContainText('明文保存');
  await expect(guide).toContainText('右键可找回角色、退出栖伴');
  const bounds=await page.locator('.pet-dialog').boundingBox();
  expect(bounds!.y+bounds!.height).toBeLessThanOrEqual(440);
  const skip = await page.getByRole('button',{name:'先随便看看'}).boundingBox();
  expect(skip!.y+skip!.height).toBeLessThanOrEqual(bounds!.y+bounds!.height);
  expect(await guide.evaluate(e=>e.scrollWidth-e.clientWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({path:test.info().outputPath('first-use.png')});
  await page.getByRole('button',{name:'去设置模型'}).click();
  expect(await page.evaluate(()=>(window as any).actions)).toContain('open_settings');
  expect(await page.evaluate(()=>(window as any).modelCalls)).toBe(0);
  await expect(guide).toHaveCount(0);
});

test('skipping is session-only; successful completion survives reload and manual guide remains', async ({ page }) => {
  await page.getByRole('button',{name:'先随便看看'}).click();
  await expect(page.getByLabel('初次使用指南')).toHaveCount(0);
  await page.reload();await page.getByRole('button',{name:'和栖栖互动'}).click();
  await expect(page.getByLabel('初次使用指南')).toBeVisible();
  await page.getByRole('button',{name:'知道了，开始相处'}).click();
  await page.reload();await page.getByRole('button',{name:'和栖栖互动'}).click();
  await expect(page.getByLabel('初次使用指南')).toHaveCount(0);
  await page.getByRole('button',{name:'使用指南',exact:true}).click();
  await expect(page.getByLabel('初次使用指南')).toBeVisible();
});

test('failed completion stays actionable and never claims the guide was saved', async ({ page }) => {
  await page.evaluate(()=>{(window as any).failSave=true;});
  await page.getByRole('button',{name:'知道了，开始相处'}).click();
  await expect(page.getByRole('alert')).toContainText('保存使用指南状态失败');
  await expect(page.getByLabel('初次使用指南')).toBeVisible();
  expect(await page.evaluate(()=>sessionStorage.getItem('guide-fixture'))).toBeNull();
  await page.getByRole('button',{name:'先随便看看'}).click();
  await expect(page.getByLabel('想记下什么？',{exact:true})).toBeEnabled();
});
