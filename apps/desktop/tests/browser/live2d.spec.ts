import {test,expect} from '@playwright/test';
test('missing experimental runtime falls back to the existing character',async({page})=>{
  await page.route('**/live2d-local/**',route=>route.abort());
  await page.goto('/');await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'Live2D 实验',exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('已恢复栖栖');
  await expect(page.getByRole('button',{name:'和栖栖互动'}).locator('svg')).toBeVisible();
});
test('local Live2D sample renders, quiet pauses draws and hide releases canvas',async({page})=>{
  test.skip(process.env.QIBAN_TEST_LIVE2D!=='1','Explicitly prepare licensed local samples before this integration check');
  await page.addInitScript(()=>{
    const w=window as any;w.testDraws=0;
    for(const prototype of [WebGLRenderingContext.prototype,WebGL2RenderingContext.prototype]){
      const draw=prototype.drawElements;
      prototype.drawElements=function(...args:Parameters<typeof draw>){w.testDraws++;return draw.apply(this,args);};
    }
  });
  await page.goto('/');await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'Live2D 实验',exact:true}).click();
  const canvas=page.getByLabel('Live2D实验角色');
  await expect(canvas).toHaveAttribute('data-loaded','true',{timeout:20000});
  await expect.poll(()=>page.evaluate(()=>(window as any).testDraws)).toBeGreaterThan(0);
  await page.getByRole('button',{name:'聊一聊',exact:true}).click();
  await expect(page.getByText('浏览器预览不调用模型，请运行桌面版',{exact:true})).toBeVisible();
  await page.screenshot({path:test.info().outputPath('live2d-preview.png')});
  await page.getByRole('button',{name:'安静陪伴',exact:true}).click();
  await expect(canvas).toHaveAttribute('data-active','false');
  // Wait across several animation frames; count must stay unchanged.
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  const count=await page.evaluate(()=>(window as any).testDraws);
  await page.waitForTimeout(250);
  expect(await page.evaluate(()=>(window as any).testDraws)).toBe(count);
  await page.getByRole('button',{name:'恢复角色预览'}).click();
  await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'隐藏',exact:true}).click();
  await expect(canvas).toHaveCount(0);
});
