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
    const callbacks=new Map<number,(value:unknown)=>void>();
    Object.defineProperty(window,'__TAURI_EVENT_PLUGIN_INTERNALS__',{value:{unregisterListener:()=>{}}});
    Object.defineProperty(window,'__TAURI_INTERNALS__',{value:{
      transformCallback:(cb:(value:unknown)=>void)=>{callbacks.set(++next,cb);return next;},
      unregisterCallback:(id:number)=>callbacks.delete(id),
      invoke:async(cmd:string,args:any)=>{
        if(cmd==='get_runtime_info')return {protocolVersion:1,appVersion:'test',runtime:'desktop',persistence:'sqlite',executorAvailable:false};
        if(cmd==='chat_config')return {configured:true,model:'glm-test-fixture'};
        if(cmd==='chat_history')return history;
        if(cmd==='chat_clear'){history=[];return;}
        if(cmd==='chat_cancel'){cancel();return;}
        if(cmd==='chat_generate'){
          count++;
          const id=args.request.requestId;
          args.onDelta.onmessage({requestId:'another-request',text:'错误请求'});
          if(count===1) {
            args.onDelta.onmessage({requestId:id,text:'第一段'});
            return new Promise((_,reject)=>{cancel=()=>{
              reject('fixture cancelled');
              setTimeout(()=>args.onDelta.onmessage({requestId:id,text:'迟到旧文本'}),40);
            };});
          }
          args.onDelta.onmessage({requestId:id,text:'新的回答'});
          history.push({user:args.request.prompt,assistant:'新的回答'});
          return {requestId:id,elapsedMs:8,usage:{total_tokens:12}};
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
  await expect(page.getByLabel('本次对话记录')).toContainText('测试取消');
  await page.getByRole('button',{name:'清空对话',exact:true}).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('想聊点什么？');
  await expect(page.getByText('最近 0 轮',{exact:true})).toBeVisible();
});
