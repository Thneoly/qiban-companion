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
  await page.getByRole('button',{name:'收起气泡'}).click();
  await page.getByRole('button',{name:'和栖栖互动'}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('新的回答');
  await page.getByText('最近 1 轮',{exact:true}).click();
  await expect(page.getByLabel('本机对话记录')).toContainText('测试取消');
  await page.getByRole('button',{name:'清空对话',exact:true}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('想聊点什么？');
  await expect(page.getByText('最近 0 轮',{exact:true})).toBeVisible();
});

test('chat passes expectedPersonal and renders two-family preview and receipt', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'isTauri', { value: true });
    const w = window as any;
    w.generateRequests = [];
    let next = 0; const callbacks = new Map<number, (value: unknown) => void>();
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (cb: (value: unknown) => void) => { callbacks.set(++next, cb); return next; }, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') return ++next;
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
  await expect(bubble.locator('.chat-memory-preview')).toContainText('个人未连接');
  await page.getByLabel('和栖栖说句话').fill('你好');
  await page.getByRole('button',{name:'发送',exact:true}).click();
  await expect(bubble.locator('.chat-memory-receipt')).toContainText('个人未含（服务未连接）');
  const first = await page.evaluate(() => (window as any).generateRequests.at(-1));
  expect(first.expectedPersonal).toBeNull();
});
