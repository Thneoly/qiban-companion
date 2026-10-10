import {test,expect} from '@playwright/test';
test('preview chat explains missing native model and does not fake a reply',async({page})=>{
  await page.goto('/');await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'聊一聊',exact:true}).click();
  await expect(page.getByRole('status')).toContainText('浏览器预览不调用模型');
  await page.getByLabel('和栖栖说句话',{exact:true}).fill('你好');
  await expect(page.getByRole('button',{name:'发送',exact:true})).toBeDisabled();
});
test('mocked native stream stops late text and permits a fresh request',async({page})=>{
  await page.addInitScript(()=>{
    Object.defineProperty(window,'isTauri',{value:true});
    let next=0;let cancel=()=>{};let count=0;let history:{user:string;assistant:string}[]=[];
    const usage={scope:{baseUrl:'https://fixture.test',model:'glm-test-fixture'},contextEpoch:1,memories:[],bodyChars:0,contextChars:0,personal:{status:'sent',memories:[],bodyChars:0,contextChars:0}};
    const callbacks=new Map<number,(value:unknown)=>void>();
    Object.defineProperty(window,'__TAURI_EVENT_PLUGIN_INTERNALS__',{value:{unregisterListener:()=>{}}});
    Object.defineProperty(window,'__TAURI_INTERNALS__',{value:{
      transformCallback:(cb:(value:unknown)=>void)=>{callbacks.set(++next,cb);return next;},
      unregisterCallback:(id:number)=>callbacks.delete(id),
      invoke:async(cmd:string,args:any)=>{
        if(cmd==='chat_context_preview')return {...usage,items:[],policy:{enabled:false,revision:0,selectedIds:[]},personal:{status:'offline',policy:{enabled:false,revision:0,selectedIds:[]},items:[],inactiveSelectedIds:[],bodyChars:0,contextChars:0}};
        if(cmd==='guide_status')return true;
        if(cmd==='get_runtime_info')return {protocolVersion:3,appVersion:'test',runtime:'desktop',persistence:'sqlite',executorAvailable:false};
        if(cmd==='personal_memory_overview')return{online:false,stats:null,serviceUrl:'http://127.0.0.1:4322'};
        if(cmd==='chat_config')return {configured:true,model:'glm-test-fixture',maxOutputTokens:1024};
        if(cmd==='chat_history')return history;
        if(cmd==='chat_clear'){history=[];return;}
        if(cmd==='chat_cancel'){cancel();return;}
        if(cmd==='chat_generate'){
          count++;
          (window as any).generateRequests?.push(structuredClone(args.request));
          const id=args.request.requestId;
          args.onDelta.onmessage({requestId:'another-request',memoryUsage:null,text:'错误请求'});
          if(count===1) {
            args.onDelta.onmessage({requestId:id,memoryUsage:null,text:'第一段'});
            return new Promise((_,reject)=>{cancel=()=>{
              reject('fixture cancelled');
              setTimeout(()=>args.onDelta.onmessage({requestId:id,memoryUsage:null,text:'迟到旧文本'}),40);
            };});
          }
          args.onDelta.onmessage({requestId:id,memoryUsage:usage,text:'新的回答'});
          history.push({user:args.request.prompt,assistant:'新的回答'});
          return {requestId:id,elapsedMs:8,memoryUsage:usage,historySaved:true,usage:{total_tokens:12}};
        }
        return 1;
      }
    }});
  });
  await page.goto('/');await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'聊一聊',exact:true}).click();
  // Empty history and no selection: the persistent boundary names the service, no turn or memory segment.
  await expect(page.locator('.chat-boundary')).toContainText('发送将把本条消息发往模型服务 fixture.test · glm-test-fixture');
  await page.getByLabel('和栖栖说句话',{exact:true}).fill('测试取消');
  await page.getByRole('button',{name:'发送',exact:true}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('第一段');
  await page.getByRole('button',{name:'停止',exact:true}).click();
  await expect(page.getByRole('button',{name:'发送',exact:true})).toBeEnabled();
  await page.getByRole('button',{name:'发送',exact:true}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('新的回答');
  await expect(page.getByRole('status')).toContainText('12 tokens');
  await page.waitForTimeout(80);
  await expect(page.getByText('迟到旧文本',{exact:false})).toHaveCount(0);
  await expect(page.getByText('最近 1 轮',{exact:true})).toBeVisible();
  await expect(page.locator('.chat-boundary')).toContainText('本条消息、最近 1 轮对话发往模型服务');
  await page.getByRole('button',{name:'收起气泡'}).click();
  await page.getByRole('button',{name:'和栖栖互动'}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('新的回答');
  await page.getByText('最近 1 轮',{exact:true}).click();
  await expect(page.getByLabel('本机对话记录')).toContainText('测试取消');
  await page.getByRole('button',{name:'清空对话',exact:true}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('想聊点什么？');
  await expect(page.getByText('最近 0 轮',{exact:true})).toBeVisible();
  await expect(page.locator('.chat-boundary')).not.toContainText('最近');
});

test('chat passes expectedPersonal and renders two-family preview and receipt', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'isTauri', { value: true });
    const w = window as any;
    w.generateRequests = [];
    let next = 0; const callbacks = new Map<number, (value: unknown) => void>(); w.__callbacks = callbacks;
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (cb: (value: unknown) => void) => { callbacks.set(++next, cb); return next; }, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') { (window as any).memoryListener = args.handler; return ++next; }
        if (cmd === 'chat_context_preview') return {
          scope: { baseUrl: 'https://fixture.test', model: 'glm-test-fixture' }, contextEpoch: 1,
          policy: { enabled: false, revision: 0, selectedIds: [] }, items: [], bodyChars: 0, contextChars: 0,
          personal: { status: 'offline', policy: { enabled: false, revision: 0, selectedIds: [] }, items: [], inactiveSelectedIds: [], bodyChars: 0, contextChars: 0 },
        };
        if (cmd === 'chat_config') return { configured: true, model: 'glm-test-fixture', maxOutputTokens: 1024 };
        if (cmd === 'chat_history') return [];
        if (cmd === 'chat_cancel') return;
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'guide_status') return true;
        if (cmd === 'personal_memory_overview') return { online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' };
        if (cmd === 'chat_generate') {
          w.generateRequests.push(structuredClone(args.request));
          const id = args.request.requestId;
          args.onDelta.onmessage({ requestId: id, text: '', memoryUsage: null });
          args.onDelta.onmessage({ requestId: id, text: '好的', memoryUsage: null });
          return { requestId: id, elapsedMs: 3, historySaved: true, usage: { total_tokens: 2 },
            memoryUsage: { scope: args.request.expectedScope, contextEpoch: 1, memories: [], bodyChars: 0, contextChars: 0,
              personal: { status: 'offline', memories: [], bodyChars: 0, contextChars: 0 } } };
        }
        return 1;
      },
    } });
  });
  await page.goto('/');await page.getByRole('button',{name:'和栖栖互动'}).click();
  await page.getByRole('button',{name:'聊一聊',exact:true}).click();
  const bubble = page.locator('.chat-bubble');
  // Offline personal preview: the preview summary says 未连接 and send passes null.
  await expect(bubble.locator('.chat-memory-preview')).toContainText('个人未启用');
  // No selection anywhere: the boundary line carries no memory segment.
  await expect(bubble.locator('.chat-boundary')).toContainText('发送将把本条消息发往模型服务 fixture.test · glm-test-fixture');
  await page.getByLabel('和栖栖说句话').fill('你好');
  await page.getByRole('button',{name:'发送',exact:true}).click();
  await expect(bubble.locator('.chat-memory-receipt')).toContainText('个人未含（服务未连接）');
  const first = await page.evaluate(() => (window as any).generateRequests.at(-1));
  expect(first.expectedPersonal).toBeNull();

  // Enabled but offline: the boundary keeps the plain message and says
  // honestly that nothing personal is carried this turn.
  await page.evaluate(() => {
    const inner = (window as any).__TAURI_INTERNALS__;
    const baseInvoke = inner.invoke;
    inner.invoke = async (cmd: string, args: any) => {
      if (cmd === 'chat_context_preview') {
        return {
          scope: { baseUrl: 'https://fixture.test', model: 'glm-test-fixture' }, contextEpoch: 1,
          policy: { enabled: false, revision: 0, selectedIds: [] }, items: [], bodyChars: 0, contextChars: 0,
          personal: { status: 'offline', policy: { enabled: true, revision: 1, selectedIds: [9] }, items: [], inactiveSelectedIds: [], bodyChars: 0, contextChars: 0 },
        };
      }
      return baseInvoke(cmd, args);
    };
  });
  await page.evaluate(() => {
    const w = window as any;
    const handler = w.memoryListener as number | undefined;
    const callbacks = w.__callbacks as Map<number, (payload: unknown) => void>;
    callbacks.get(handler)?.({ event: 'memory-changed', id: 0, payload: { contextEpoch: 2 } });
  });
  await expect(bubble.locator('.chat-boundary')).toContainText('发往模型服务 fixture.test · glm-test-fixture；个人记忆服务未连接，本次不携带');

  // Flip the personal family online with one item: the preview summary
  // counts it, send passes the ordered (id, seq) pair, and the receipt
  // cross-references by seq.
  await page.evaluate(() => {
    const inner = (window as any).__TAURI_INTERNALS__;
    const baseInvoke = inner.invoke;
    inner.invoke = async (cmd: string, args: any) => {
      if (cmd === 'chat_context_preview') {
        return {
          scope: { baseUrl: 'https://fixture.test', model: 'glm-test-fixture' }, contextEpoch: 1,
          policy: { enabled: false, revision: 0, selectedIds: [] }, items: [], bodyChars: 0, contextChars: 0,
          personal: {
            status: 'online', policy: { enabled: true, revision: 1, selectedIds: [9] },
            items: [{ id: 9, seq: 4, type: 'insight', project: null, title: '洞察', content: '内容正文', importance: 3,
              createdAt: '2026-09-25 02:10:12', updatedAt: '2026-09-25 02:10:12', validUntil: null,
              supersededBy: null, contradicts: null, tags: [], origin: 'mcp' }],
            inactiveSelectedIds: [], bodyChars: 4, contextChars: 120,
          },
        };
      }
      if (cmd === 'chat_generate') {
        (window as any).generateRequests.push(structuredClone(args.request));
        const id = args.request.requestId;
        args.onDelta.onmessage({ requestId: id, text: '', memoryUsage: null });
        args.onDelta.onmessage({ requestId: id, text: '再来', memoryUsage: null });
        return { requestId: id, elapsedMs: 2, historySaved: true, usage: { total_tokens: 3 },
          memoryUsage: { scope: args.request.expectedScope, contextEpoch: 1, memories: [], bodyChars: 0, contextChars: 0,
            personal: { status: 'sent', memories: [{ id: 9, seq: 4 }], bodyChars: 4, contextChars: 120 } } };
      }
      return baseInvoke(cmd, args);
    };
  });
  await page.evaluate(() => {
    const w = window as any;
    const handler = w.memoryListener as number | undefined;
    const callbacks = w.__callbacks as Map<number, (payload: unknown) => void>;
    // Bumped epoch forces reconcileMemory -> invalidate -> bracketed reload,
    // exactly what a personal policy save does in production.
    callbacks.get(handler)?.({ event: 'memory-changed', id: 0, payload: { contextEpoch: 3 } });
  });
  const previewDetails = bubble.locator('details.chat-memory-preview:not(.chat-memory-receipt)');
  await expect(previewDetails).toContainText('个人1条');
  await expect(previewDetails).toContainText('洞察');
  // One active personal item: the boundary line names both families.
  await expect(bubble.locator('.chat-boundary')).toContainText('和已选记忆（应用 0 条 · 个人 1 条）发往模型服务');
  await page.getByLabel('和栖栖说句话').fill('再聊');
  await page.getByRole('button',{name:'发送',exact:true}).click();
  await expect(bubble.locator('.chat-memory-receipt')).toContainText('个人1条');
  await expect(bubble.locator('.chat-memory-receipt')).toContainText('seq 4');
  const second = await page.evaluate(() => (window as any).generateRequests.at(-1));
  expect(second.expectedPersonal).toEqual([{ id: 9, seq: 4 }]);
});
