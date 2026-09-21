import {test,expect} from '@playwright/test';
test('browser preview explains execution boundary',async({page})=>{
  await page.goto('/?view=panel');await page.getByRole('button',{name:'打开文档任务',exact:true}).click();
  await expect(page.getByText('请运行桌面版来处理本地文档。浏览器预览不模拟执行成功。')).toBeVisible();
  await expect(page.getByRole('button',{name:'确认保存草稿'})).toHaveCount(0);
});
test('document preview requires confirmation and a failed receipt preserves the recovery path',async({page})=>{
  await page.addInitScript(()=>{
    const w=window as any;w.executionCalls=[];
    Object.defineProperty(window,'isTauri',{value:true});
    Object.defineProperty(window,'__TAURI_EVENT_PLUGIN_INTERNALS__',{value:{unregisterListener:()=>{}}});
    const id='00000000-0000-4000-8000-000000000001',actionId='00000000-0000-4000-8000-000000000002';let task:any=null;let failed=false;
    const detail=()=>({task,attempts:[],events:[{sequence:task.revision+1,taskId:id,revision:task.revision,status:task.status,createdAt:1}]});
    Object.defineProperty(window,'__TAURI_INTERNALS__',{value:{transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(cmd:string,args:any)=>{
      if(cmd==='get_runtime_info')return {protocolVersion:2,appVersion:'test',runtime:'desktop',persistence:'sqlite',executorAvailable:false};
      if(cmd==='list_tasks')return [];
      if(cmd==='model_settings_get')return {baseUrl:'https://example.com/v1',model:'fixture',useApiKey:false,hasApiKey:false,maxOutputTokens:1024};
      if(cmd==='memory_list')return {items:[],contextEpoch:0};
      if(cmd==='execution_list')return task?[task]:[];
      if(cmd==='execution_prepare'){w.executionCalls.push({cmd,args});task={id,actionId,sourceName:args.sourceName,preview:'原文摘录示例',artifactName:actionId+'.md',artifactHash:'a'.repeat(64),status:'waiting_confirmation',revision:0,createdAt:1,updatedAt:1,note:'尚未创建草稿文件'};return detail();}
      if(cmd==='execution_confirm'){w.executionCalls.push({cmd,args});if(!failed){failed=true;throw {message:'回执读取失败，请核对执行记录'};}task={...task,status:'completed',revision:2,note:'草稿存在且内容核验通过'};return detail();}
      if(cmd==='execution_detail')return detail();
      if(cmd==='execution_result')return '已经保存的实际内容';
      return 1;
    }}});
  });
  await page.goto('/?view=panel');await page.getByRole('button',{name:'打开文档任务',exact:true}).click();
  await page.getByLabel('选择文档',{exact:true}).setInputFiles({name:'notes.txt',mimeType:'text/plain',buffer:Buffer.from('本机资料')});
  await page.getByRole('button',{name:'生成摘录预览',exact:true}).click();
  await expect(page.getByLabel('摘录预览',{exact:true})).toContainText('原文摘录示例');
  expect(await page.evaluate(()=>(window as any).executionCalls.filter((c:any)=>c.cmd==='execution_confirm').length)).toBe(0);
  await page.getByRole('button',{name:'确认保存草稿',exact:true}).click();
  await expect(page.getByRole('alert').filter({hasText:'回执读取失败'})).toBeVisible();
  await expect(page.getByRole('button',{name:'确认保存草稿',exact:true})).toBeEnabled();
  await page.getByRole('button',{name:'确认保存草稿',exact:true}).click();
  await page.getByRole('button',{name:'读取已保存草稿',exact:true}).click();
  await expect(page.getByLabel('已保存草稿',{exact:true})).toHaveText('已经保存的实际内容');
  const calls=await page.evaluate(()=>(window as any).executionCalls);expect(calls[1].args).toEqual(calls[2].args);
});
