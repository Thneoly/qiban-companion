import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    const callbacks = new Map(); const events = new Map(); let sequence = 0;
    w.memoryItems = []; w.memoryEpoch = 0; w.memoryMutations = [];
    w.memoryFail = ''; w.exportOutcome = {status: 'cancelled'};
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (callback: any) => { callbacks.set(++sequence, callback); return sequence; }, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') { events.set(args.event, callbacks.get(args.handler)); return ++sequence; }
        if (cmd === 'get_runtime_info') return {protocolVersion:1,appVersion:'test',runtime:'desktop',persistence:'sqlite',executorAvailable:false};
        if (cmd === 'list_tasks') return [];
        if (cmd === 'memory_list') { if (w.failRead) throw {code:'storage_unavailable'}; return {items:structuredClone(w.memoryItems),contextEpoch:w.memoryEpoch,modelUseEnabled:false}; }
        if (cmd === 'memory_export') { if (w.memoryFail) throw {code:w.memoryFail}; return w.exportOutcome; }
        if (cmd === 'memory_mutate') {
          w.memoryMutations.push(args.request);
          if (w.memoryFail) throw {code:w.memoryFail};
          const r=args.request;
          if (r.expectedEpoch!==w.memoryEpoch) throw {code:'context_changed'};
          if (r.action!=='create' && r.restartConversation!==true) throw {code:'confirmation_required'};
          if (r.action==='create') w.memoryItems.push({id:crypto.randomUUID(),...r.draft,body:r.draft.body.trim(),sourceKind:'user_manual',sourceLabel:'用户在记忆面板填写',createdAt:1000,confirmedAt:1000,updatedAt:1000,revision:1});
          if (r.action==='update') w.memoryItems=w.memoryItems.map((m:any)=>m.id===r.id?{...m,...r.draft,revision:m.revision+1,confirmedAt:2000,updatedAt:2000}:m);
          if (r.action==='delete') w.memoryItems=w.memoryItems.filter((m:any)=>m.id!==r.id);
          if (r.action==='delete_all') w.memoryItems=[];
          w.memoryEpoch++;
          return {contextEpoch:w.memoryEpoch,chatCleared:r.action!=='create',notificationsDelivered:!w.failNotice};
        }
        return 1;
      }
    } });
  });
  await page.goto('/?view=panel');
});

test('saves explicit memories, confirms corrections and deletes without claiming model use', async ({ page }) => {
  const panel=page.getByRole('region',{name:'我们的记忆'});
  await expect(panel).toContainText('还没有留下记忆');
  await panel.getByLabel('记忆内容').fill('先说结论🌱');
  await panel.getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(panel.getByRole('status')).toContainText('尚未用于模型对话');
  const card=panel.locator('article'); await expect(card).toContainText('用户在记忆面板填写'); await expect(card).toContainText('未指定日期');
  await card.getByRole('button',{name:'更正',exact:true}).click();
  await panel.getByLabel('记忆内容').fill('先说结论，再列证据');
  await panel.getByRole('button',{name:'检查更正影响'}).click();
  await expect(panel.getByRole('alertdialog')).toContainText('全部模型的聊天记录');
  await panel.getByRole('button',{name:'返回，不修改'}).click();
  expect(await page.evaluate(()=>(window as any).memoryMutations.length)).toBe(1);
  await panel.getByRole('button',{name:'检查更正影响'}).click();
  await panel.getByRole('button',{name:'确认并清空聊天'}).click();
  await expect(card).toContainText('先说结论，再列证据');
  await card.getByRole('button',{name:'删除',exact:true}).click();
  await panel.getByRole('button',{name:'确认并清空聊天'}).click();
  await expect(panel).toContainText('还没有留下记忆');
});

test('keeps drafts and records on failure, and distinguishes cancelled exports and notification failures', async ({ page }) => {
  const panel=page.getByRole('region',{name:'我们的记忆'});
  await panel.getByLabel('记忆内容').fill('不要丢掉草稿');
  await page.evaluate(()=>{(window as any).memoryFail='storage_unavailable';});
  await panel.getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(panel.getByRole('alert')).toContainText('本机记忆不可用');
  await expect(panel.getByLabel('记忆内容')).toHaveValue('不要丢掉草稿');
  await page.evaluate(()=>{const w=window as any;w.memoryFail='';w.failNotice=true;});
  await panel.getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(panel.getByRole('status')).toContainText('另一窗口待刷新');
  await panel.getByRole('button',{name:'导出 JSON'}).click();
  await expect(panel.getByRole('status')).toContainText('已取消导出');
  await page.evaluate(()=>{(window as any).exportOutcome={status:'saved',count:1};});
  await panel.getByRole('button',{name:'导出 JSON'}).click();
  await expect(panel.getByRole('status')).toContainText('已导出1条');
  await panel.getByRole('button',{name:'删除全部记忆'}).click();
  await page.evaluate(()=>{(window as any).memoryEpoch++;});
  await panel.getByRole('button',{name:'确认并清空聊天'}).click();
  await expect(panel.getByRole('alert')).toContainText('已变化');
  await expect(panel.locator('article')).toHaveCount(1);
});

test('unicode count, full capacity, unavailable store and narrow layout remain actionable', async ({ page }) => {
  const panel=page.getByRole('region',{name:'我们的记忆'});
  await panel.getByLabel('记忆内容').fill('🌱'.repeat(201));
  await panel.getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(panel.getByRole('alert')).toContainText('1～200');
  await page.evaluate(()=>{const w=window as any; w.memoryItems=Array.from({length:30},()=>({id:crypto.randomUUID(),kind:'preference',body:'合成样本',eventDate:null,sourceKind:'user_manual',sourceLabel:'用户在记忆面板填写',createdAt:1,confirmedAt:1,updatedAt:1,revision:1}));});
  await panel.getByRole('button',{name:'刷新记忆'}).click();
  await expect(panel.getByRole('button',{name:'保存记忆',exact:true})).toBeDisabled();
  await page.setViewportSize({width:840,height:900});
  expect(await panel.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
  await page.evaluate(()=>{(window as any).failRead=true;}); await panel.getByRole('button',{name:'刷新记忆'}).click();
  await expect(panel).toContainText('记录尚未载入');
  await expect(panel).not.toContainText('还没有留下记忆');
  await expect(panel.getByRole('button',{name:'导出 JSON'})).toBeDisabled();
});
