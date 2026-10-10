import { test, expect } from '@playwright/test';
test.beforeEach(async({page})=>{
  await page.addInitScript(()=>{
    const w=window as any;Object.defineProperty(window,'isTauri',{value:true});
    const callbacks=new Map(),events=new Map();let seq=0;
    w.scope={baseUrl:'https://fixture.test',model:'model-a'};w.epoch=1;w.policy={enabled:false,revision:0,selectedIds:[]};w.writes=[];w.impactCalls=[];w.impactTurns=1;
    w.items=Array.from({length:6},(_,i)=>({id:`00000000-0000-4000-8000-00000000000${i}`,kind:'preference',body:`${i}`+'文'.repeat(199),eventDate:null,sourceKind:'user_manual',sourceLabel:'用户在记忆面板填写',createdAt:1,confirmedAt:1,updatedAt:1,revision:1}));
    Object.defineProperty(window,'__TAURI_EVENT_PLUGIN_INTERNALS__',{value:{unregisterListener:()=>{}}});
    Object.defineProperty(window,'__TAURI_INTERNALS__',{value:{transformCallback:(cb:any)=>{callbacks.set(++seq,cb);return seq;},unregisterCallback:()=>{},invoke:async(cmd:string,args:any)=>{
      if(cmd==='plugin:event|listen'){events.set(args.event,callbacks.get(args.handler));return ++seq;}
      if(cmd==='get_runtime_info')return {protocolVersion:3,appVersion:'test',runtime:'desktop',persistence:'sqlite',executorAvailable:false};
      if(cmd==='list_tasks')return [];
      if(cmd==='personal_memory_overview')return{online:false,stats:null,serviceUrl:'http://127.0.0.1:4322'};
      if(cmd==='memory_list')return {items:w.items,contextEpoch:w.epoch};
      if(cmd==='chat_usage_impact'){w.impactCalls.push(args.request);return {scopes:[{scope:structuredClone(w.scope),affectedTurns:w.impactTurns,keptTurns:0}],affectedTurnsTotal:w.impactTurns};}
      if(cmd==='chat_context_preview'){const items=w.policy.selectedIds.map((id:string)=>w.items.find((m:any)=>m.id===id));return {scope:structuredClone(w.scope),contextEpoch:w.epoch,policy:structuredClone(w.policy),items,bodyChars:items.length*200,contextChars:items.length*500,personal:{status:'offline',policy:{enabled:false,revision:0,selectedIds:[]},items:[],inactiveSelectedIds:[],bodyChars:0,contextChars:0}};}
      if(cmd==='memory_policy_set'){
        const r=args.request;w.writes.push(r);
        if(r.expectedScope.model!==w.scope.model||r.expectedEpoch!==w.epoch)throw {code:'context_changed'};
        if(w.fail)throw {code:'storage_unavailable'};
        const cleared=w.policy.selectedIds.some((id:string)=>!r.selectedIds.includes(id));
        if(cleared&&!r.restartConversation)throw {code:'confirmation_required'};
        w.policy={enabled:r.enabled&&!!r.selectedIds.length,selectedIds:r.selectedIds,revision:w.policy.revision+1};w.epoch++;
        return {contextEpoch:w.epoch,chatCleared:cleared,clearedTurns:cleared?1:0,notificationsDelivered:true};
      }
      return 1;
    }}});
  });
  await page.goto('/?view=panel');
});
test('requires explicit selection, enforces budgets, and confirms revocation',async({page})=>{
  const region=page.getByRole('region',{name:'模型记忆许可'}),checks=region.getByRole('checkbox');
  await expect(checks.first()).not.toBeChecked();await expect(checks.nth(1)).toBeDisabled();
  await checks.first().check();for(let i=1;i<=6;i++)await checks.nth(i).check();
  const save=region.getByRole('button',{name:'保存此模型的记忆设置'});await expect(save).toBeDisabled();
  await checks.nth(6).uncheck();await expect(save).toBeDisabled();await checks.nth(5).uncheck();await save.click();
  await expect(region).toContainText('启用 · 4条');expect(await page.evaluate(()=>(window as any).writes[0].selectedIds.length)).toBe(4);
  await checks.first().uncheck();await save.click();await expect(region.getByRole('alertdialog')).toContainText('仍在其他模型选用的对话不动');
  // Upper-bound precount: the consult cannot see the still-selecting exemption.
  await expect(region.getByRole('alertdialog')).toContainText('预计最多清除最近 1 轮');
  const removed=await page.evaluate(()=>(window as any).items.slice(0,4).map((m:any)=>m.id));
  expect(await page.evaluate(()=>(window as any).impactCalls[0])).toEqual({appIds:removed,personalIds:[]});
  await region.getByRole('button',{name:'返回',exact:true}).click();expect(await page.evaluate(()=>(window as any).writes.length)).toBe(1);
  // Reopening consults again; a zero report keeps the receipt authoritative.
  await page.evaluate(()=>{(window as any).impactTurns=0;});
  await save.click();await expect(region.getByRole('alertdialog')).toContainText('目前没有本机对话使用过所移除条目');
  await expect(region.getByRole('alertdialog')).toContainText('预计聊天记录保持不变');
  await region.getByRole('button',{name:'返回',exact:true}).click();
  await page.evaluate(()=>{(window as any).impactTurns=1;});
  await save.click();await region.getByRole('button',{name:'确认收回并开始新对话'}).click();await expect(region).toContainText('已保存状态：关闭');
  await expect(region).toContainText('已清除使用过所移除条目的最近 1 轮对话');
  expect(await page.evaluate(()=>(window as any).writes[1].restartConversation)).toBe(true);
  // Three dialog opens, three consults: reopen always refreshes the precount.
  expect(await page.evaluate(()=>(window as any).impactCalls.length)).toBe(3);
});
test('stale model scope is rejected and failed saves retain selection',async({page})=>{
  const region=page.getByRole('region',{name:'模型记忆许可'}),checks=region.getByRole('checkbox');
  await checks.first().check();await checks.nth(1).check();await page.evaluate(()=>{(window as any).fail=true;});
  await region.getByRole('button',{name:'保存此模型的记忆设置'}).click();await expect(region).toContainText('本机记忆不可用');await expect(checks.nth(1)).toBeChecked();
  await page.evaluate(()=>{const w=window as any;w.fail=false;w.scope.model='model-b';w.epoch++;});
  await region.getByRole('button',{name:'保存此模型的记忆设置'}).click();await expect(region).toContainText('已变化');
  expect(await page.evaluate(()=>(window as any).policy.enabled)).toBe(false);
  await page.getByRole('button',{name:'刷新记忆'}).click();await expect(region).toContainText('model-b');await expect(checks.first()).not.toBeChecked();
});
