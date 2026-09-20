import {test,expect} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
async function appearance(page: import('@playwright/test').Page) {
  await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'角色与场景',exact:true}).click();
}
test('missing runtime falls back, model can be removed and preferences survive reload',async({page})=>{
  const folder=test.info().outputPath('broken-model');await mkdir(folder,{recursive:true});
  const png=Buffer.alloc(24);png.writeUInt32BE(0x89504e47,0);png.writeUInt32BE(0x0d0a1a0a,4);png.write('IHDR',12);png.writeUInt32BE(1,16);png.writeUInt32BE(1,20);
  await writeFile(path.join(folder,'fake.model3.json'),JSON.stringify({Version:3,FileReferences:{Moc:'fake.moc3',Textures:['texture.png']}}));
  await writeFile(path.join(folder,'fake.moc3'),'MOC3');await writeFile(path.join(folder,'texture.png'),png);
  await page.route('**/live2d-runtime/**',route=>route.abort());
  await page.goto('/');await appearance(page);
  await page.getByLabel('导入模型文件夹',{exact:true}).setInputFiles(folder);
  await expect(page.locator('.pet-character svg')).toBeVisible();
  await page.getByRole('button',{name:'返回互动'}).click();
  await expect(page.getByRole('alert')).toContainText('已恢复栖栖');
  await page.getByRole('button',{name:'角色与场景',exact:true}).click();
  await page.getByLabel('场景',{exact:true}).selectOption('none');
  await expect(page.locator('.pet-scene')).toHaveCount(0);
  await page.getByRole('button',{name:'移除本机模型'}).click();
  await expect(page.getByRole('button',{name:'移除本机模型'})).toHaveCount(0);
  await expect(page.getByRole('button',{name:'2D 数字人',exact:true})).toBeDisabled();
  await page.reload();await appearance(page);
  await expect(page.getByLabel('场景',{exact:true})).toHaveValue('none');
  await expect(page.getByRole('button',{name:'2D 数字人',exact:true})).toBeDisabled();
});
test('appearance changes keep character opaque and zero opacity removes background hit target',async({page})=>{
  await page.goto('/');await appearance(page);
  expect((await page.locator('.pet-scene').boundingBox())!.height).toBe(64);
  await page.getByLabel('场景',{exact:true}).selectOption('room');
  await expect(page.locator('.pet-scene')).toHaveCSS('height','428px');
  const range=page.getByLabel('背景不透明度',{exact:true});
  await range.fill('0');await range.dispatchEvent('pointerup');
  await expect(page.locator('.pet-scene')).toHaveCount(0);
  await expect(page.locator('.pet-character')).toHaveCSS('opacity','1');
  await page.reload();await appearance(page);
  await expect(range).toHaveValue('0');
  await range.fill('40');await range.dispatchEvent('pointerup');
  await expect(page.locator('.pet-scene')).toHaveCSS('opacity','0.4');
});
test('imported Live2D renders, survives reload, quiet pauses draws and hide releases canvas',async({page})=>{
  test.skip(process.env.QIBAN_TEST_LIVE2D!=='1','Explicitly prepare licensed local samples before this integration check');
  await page.addInitScript(()=>{
    const w=window as any;w.testDraws=0;
    for(const prototype of [WebGLRenderingContext.prototype,WebGL2RenderingContext.prototype]){
      const draw=prototype.drawElements;
      prototype.drawElements=function(...args:Parameters<typeof draw>){w.testDraws++;return draw.apply(this,args);};
    }
  });
  const remote:string[]=[];page.on('request',r=>{if(/^https?:/.test(r.url())&&!r.url().startsWith('http://127.0.0.1:1430'))remote.push(r.url());});
  await page.goto('/');await appearance(page);
  await page.getByLabel('导入模型文件夹',{exact:true}).setInputFiles(path.resolve('public/live2d-local/Hiyori'));
  const canvas=page.getByLabel('Live2D 数字人',{exact:true});
  await expect(canvas).toHaveAttribute('data-loaded','true',{timeout:20000});
  await expect.poll(()=>page.evaluate(()=>(window as any).testDraws)).toBeGreaterThan(0);
  await page.getByRole('button',{name:'返回互动'}).click();
  await page.getByRole('button',{name:'聊一聊',exact:true}).click();
  await expect(page.getByText('浏览器预览不调用模型，请运行桌面版',{exact:true})).toBeVisible();
  await expect(canvas).toHaveAttribute('data-state','attentive');
  await page.keyboard.press('Escape');
  await page.screenshot({path:test.info().outputPath('live2d-stage.png')});
  await page.reload();
  await expect(canvas).toHaveAttribute('data-loaded','true',{timeout:20000});
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.waitForTimeout(80);
  const reducedCount=await page.evaluate(()=>(window as any).testDraws);await page.waitForTimeout(150);
  expect(await page.evaluate(()=>(window as any).testDraws)).toBe(reducedCount);
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'安静陪伴',exact:true}).click();
  await expect(canvas).toHaveAttribute('data-active','false');
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  const count=await page.evaluate(()=>(window as any).testDraws);await page.waitForTimeout(250);
  expect(await page.evaluate(()=>(window as any).testDraws)).toBe(count);
  await page.getByRole('button',{name:'恢复角色预览'}).click();
  await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'隐藏',exact:true}).click();
  await expect(canvas).toHaveCount(0);expect(remote).toEqual([]);
});
