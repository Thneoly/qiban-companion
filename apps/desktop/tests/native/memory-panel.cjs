// Windows acceptance: fresh app identifier, synthetic data and localhost-only model fixture.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),http=require('node:http');
const {spawn,execFile}=require('node:child_process');
const {promisify}=require('node:util'); const execute=promisify(execFile);
const {DatabaseSync}=require('node:sqlite'); const {chromium,expect}=require('@playwright/test');
const exe=process.env.QIBAN_ACCEPTANCE_EXE,id=process.env.QIBAN_ACCEPTANCE_ID;
assert(exe&&fs.existsSync(exe));assert(/^dev\.qiban\.companion\.acceptance\.m2-[a-z0-9-]+$/.test(id||''));
const directory=path.join(process.env.LOCALAPPDATA,id); assert(!fs.existsSync(directory),'Refusing an existing profile');
const evidence=path.resolve('.cache',id+'-'+require('node:crypto').randomUUID());fs.mkdirSync(evidence,{recursive:true});
const port=Number(process.env.QIBAN_CDP_PORT||9442),delay=ms=>new Promise(r=>setTimeout(r,ms));
let child,browser,pet,panel,heldResponse;const requests=[];
const server=http.createServer((req,res)=>{let body='';req.on('data',chunk=>body+=chunk);req.on('end',()=>{
  assert(!req.headers.authorization);requests.push(JSON.parse(body));heldResponse=res;
  res.writeHead(200,{'Content-Type':'text/event-stream'});res.write('data: '+JSON.stringify({choices:[{delta:{content:'合成未完成片段'}}]})+'\n\n');
});});
async function start(){
  browser=undefined;
  child=spawn(exe,[],{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(directory,'test-webview')}});
  for(let n=0;n<100;n++){try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);break;}catch{await delay(100);}}
  assert(browser);pet=panel=undefined;
  for(let n=0;n<100;n++){const pages=browser.contexts().flatMap(c=>c.pages());pet=pages.find(p=>p.url().includes('tauri')&&!p.url().includes('view=panel'));panel=pages.find(p=>p.url().includes('view=panel'));if(pet&&panel)break;await delay(100);}
  assert(pet&&panel);await pet.getByRole('button',{name:'和栖栖互动'}).waitFor();
  assert(fs.existsSync(path.join(directory,'instance.lock')),'Wrong build profile; refusing mutations');
  const db=new DatabaseSync(path.join(directory,'chat-history.db'),{readOnly:true});assert.equal(db.prepare('PRAGMA user_version').get().user_version,3);db.close();
  await invoke(pet,'pet_action',{action:'open_memory'});
  await panel.getByRole('region',{name:'我们的记忆'}).getByRole('button',{name:'保存记忆',exact:true}).waitFor();
}
async function stop(){if(browser)await browser.close();browser=undefined;if(child){const process=child;child=undefined;const exited=new Promise(r=>process.once('exit',r));process.kill();await exited;}await delay(400);}
const invoke=(page,command,args)=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
const dialog=(action,destination)=>execute('powershell.exe',['-NoProfile','-File',path.join(__dirname,'export-dialog.ps1'),'-TargetProcessId',String(child.pid),'-Action',action,...(destination?['-Destination',destination]:[])],{windowsHide:true,encoding:'utf8',timeout:15000});
async function exportFile(destination,overwrite=false){
  await panel.getByRole('button',{name:'导出 JSON'}).click();
  if(!destination){await dialog('cancel');await expect(panel.locator('.memory-panel').getByRole('status')).toContainText('已取消导出');return;}
  const save=dialog('save',destination);
  if(overwrite){await delay(750);await dialog('overwrite');}
  await save;await expect(panel.locator('.memory-panel').getByRole('status')).toContainText('已导出');
}
(async()=>{try{
  let occupied=false;try{await fetch(`http://127.0.0.1:${port}/json/version`);occupied=true;}catch{} assert(!occupied);
  await new Promise(r=>server.listen(0,'127.0.0.1',r));await start();
  const memory=()=>panel.getByRole('region',{name:'我们的记忆'});
  await expect(memory()).toContainText('还没有留下记忆');
  await memory().getByLabel('记忆类型').selectOption('experience');await memory().getByLabel('记忆内容').fill('合成经历：完成一次徒步🌱');await memory().getByLabel('经历日期（可选）').fill('2024-02-29');
  await memory().getByRole('button',{name:'保存记忆',exact:true}).click();await expect(memory().locator('article')).toContainText('2024-02-29');
  await stop();await start();await expect(memory().locator('article')).toContainText('合成经历：完成一次徒步🌱');
  let snapshot=await invoke(panel,'memory_list');assert.equal(snapshot.items.length,1);assert.equal((await invoke(pet,'chat_context_preview')).policy.enabled,false);
  await assert.rejects(invoke(pet,'memory_mutate',{request:{action:'delete_all',expectedEpoch:snapshot.contextEpoch,restartConversation:true}}));
  await assert.rejects(invoke(panel,'memory_mutate',{request:{action:'delete_all',expectedEpoch:snapshot.contextEpoch,restartConversation:false}}));
  assert.equal((await invoke(panel,'memory_list')).items.length,1);
  await exportFile();const exported=path.join(evidence,'memories.json');await exportFile(exported);
  assert.equal(JSON.parse(fs.readFileSync(exported,'utf8')).items[0].body,'合成经历：完成一次徒步🌱');
  await memory().locator('article').getByRole('button',{name:'更正',exact:true}).click();await memory().getByLabel('记忆内容').fill('合成经历：已更正');
  await memory().getByRole('button',{name:'检查更正影响'}).click();await memory().getByRole('button',{name:'确认并清空聊天'}).click();await expect(memory().locator('article')).toContainText('合成经历：已更正');
  await exportFile(exported,true);assert.equal(JSON.parse(fs.readFileSync(exported,'utf8')).items[0].body,'合成经历：已更正');
  await memory().screenshot({path:path.join(evidence,'memory-panel.png')});
  await invoke(panel,'model_settings_save',{config:{baseUrl:`http://127.0.0.1:${server.address().port}`,model:'m2-fixture',useApiKey:false,maxOutputTokens:1024}});
  const requestContext=await invoke(pet,'chat_context_preview');
  await pet.evaluate(requestContext=>{const native=window.__TAURI_INTERNALS__;window.testDeltas=[];window.testResult=null;
    const callback=native.transformCallback(raw=>{if(raw.message)window.testDeltas.push(raw.message);});
    native.invoke('chat_generate',{request:{requestId:'m2-stream',prompt:'本机测试',expectedScope:requestContext.scope,expectedContextEpoch:requestContext.contextEpoch},onDelta:`__CHANNEL__:${callback}`}).then(value=>{window.testResult={ok:true,value};},error=>{window.testResult={ok:false,error};});
  },requestContext);
  for(let n=0;n<100&&!requests.length;n++)await delay(100);assert.equal(requests.length,1);
  assert(!JSON.stringify(requests[0]).includes('合成经历'),'M2 must not inject memory');
  await memory().getByRole('button',{name:'删除全部记忆'}).click();await memory().getByRole('button',{name:'确认并清空聊天'}).click();await expect(memory()).toContainText('还没有留下记忆');
  heldResponse.end('data: {"choices":[{"delta":{"content":"迟到文本"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
  await expect.poll(()=>pet.evaluate(()=>window.testResult?.ok)).toBe(false);
  assert.deepEqual(await invoke(pet,'chat_history'),[]);
  const emptyExport=path.join(evidence,'empty.json');await exportFile(emptyExport);assert.deepEqual(JSON.parse(fs.readFileSync(emptyExport,'utf8')).items,[]);
  await stop();await start();assert.deepEqual((await invoke(panel,'memory_list')).items,[]);assert.deepEqual(await invoke(pet,'chat_history'),[]);
  await stop();
  const db=new DatabaseSync(path.join(directory,'chat-history.db'),{readOnly:true});assert.equal(db.prepare('SELECT count(*) AS n FROM memories WHERE deleted_at IS NOT NULL AND body IS NULL AND source_label IS NULL').get().n,1);db.close();
  const result={passed:true,identifier:id,localhostRequests:requests.length,cases:['create and restart','window permission and confirmation','native export cancel/save/overwrite','correct and clear','delete during stream','restart empty and tombstone','empty export without deleted text']};
  fs.writeFileSync(path.join(evidence,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{await stop();server.closeAllConnections();server.close();}})().catch(error=>{console.error(error);process.exitCode=1;});
