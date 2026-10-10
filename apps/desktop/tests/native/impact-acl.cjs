// Windows acceptance: fresh app identifier, synthetic data and localhost-only model fixture.
// Verifies the chat_usage_impact wiring end to end: read-only consult on the panel
// window, ACL denial on the pet window, ledger attribution after one real turn,
// and the confirm dialog rendering the precomputed count.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),http=require('node:http');
const {spawn}=require('node:child_process');
const {DatabaseSync}=require('node:sqlite'); const {chromium,expect}=require('@playwright/test');
const exe=process.env.QIBAN_ACCEPTANCE_EXE,id=process.env.QIBAN_ACCEPTANCE_ID;
assert(exe&&fs.existsSync(exe));assert(/^dev\.qiban\.companion\.acceptance\.b2-[a-z0-9-]+$/.test(id||''));
const directory=path.join(process.env.LOCALAPPDATA,id); assert(!fs.existsSync(directory),'Refusing an existing profile');
const evidence=path.resolve('.cache',id+'-'+require('node:crypto').randomUUID());fs.mkdirSync(evidence,{recursive:true});
const port=Number(process.env.QIBAN_CDP_PORT||9446),delay=ms=>new Promise(r=>setTimeout(r,ms));
let child,browser,pet,panel;const requests=[];
// Minimal completing fixture: one delta, then stop — enough for one recorded turn.
const server=http.createServer((req,res)=>{let body='';req.on('data',chunk=>body+=chunk);req.on('end',()=>{
  assert(!req.headers.authorization);requests.push(JSON.parse(body));
  res.writeHead(200,{'Content-Type':'text/event-stream'});
  res.write('data: '+JSON.stringify({choices:[{delta:{content:'本地夹具回复，不计入模型质量'}}]})+'\n\n');
  res.end('data: {"choices":[{"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
});});
async function start(){
  browser=undefined;
  child=spawn(exe,[],{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(directory,'test-webview')}});
  for(let n=0;n<100;n++){try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);break;}catch{await delay(100);}}
  assert(browser);pet=panel=undefined;
  for(let n=0;n<100;n++){const pages=browser.contexts().flatMap(c=>c.pages());pet=pages.find(p=>p.url().includes('tauri')&&!p.url().includes('view=panel'));panel=pages.find(p=>p.url().includes('view=panel'));if(pet&&panel)break;await delay(100);}
  assert(pet&&panel);await pet.getByRole('button',{name:'和栖栖互动'}).waitFor();
  assert(fs.existsSync(path.join(directory,'instance.lock')),'Wrong build profile; refusing mutations');
  const db=new DatabaseSync(path.join(directory,'chat-history.db'),{readOnly:true});assert.equal(db.prepare('PRAGMA user_version').get().user_version,4);db.close();
}
async function stop(){if(browser)await browser.close();browser=undefined;if(child){const process=child;child=undefined;const exited=new Promise(r=>process.once('exit',r));process.kill();await exited;}await delay(400);}
const invoke=(page,command,args)=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
const denyOnPet=async request=>{try{await invoke(pet,'chat_usage_impact',{request});return null;}catch(error){return error;}};
(async()=>{try{
  let occupied=false;try{await fetch(`http://127.0.0.1:${port}/json/version`);occupied=true;}catch{} assert(!occupied);
  await new Promise(r=>server.listen(0,'127.0.0.1',r));await start();
  await invoke(pet,'pet_action',{action:'open_memory'});
  const memory=()=>panel.getByRole('region',{name:'我们的记忆'});
  await memory().getByRole('button',{name:'保存记忆',exact:true}).waitFor();
  // Empty consult: read-only, zero scopes, nothing written.
  assert.deepEqual(await invoke(panel,'chat_usage_impact',{request:{appIds:[],personalIds:[]}}),{scopes:[],affectedTurnsTotal:0});
  // The pet window is not granted the command; the invoke must be denied.
  const deniedEmpty=await denyOnPet({appIds:[],personalIds:[]});
  assert(deniedEmpty,'pet window must not reach chat_usage_impact');
  // One synthetic memory; before any turn the dialog consults a zero report.
  await memory().getByLabel('记忆内容').fill('合成偏好：先说结论');
  await memory().getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(memory().locator('article')).toContainText('合成偏好：先说结论');
  const snapshot=await invoke(panel,'memory_list');assert.equal(snapshot.items.length,1);
  const itemId=snapshot.items[0].id;
  await memory().locator('article').getByRole('button',{name:'更正',exact:true}).click();
  await memory().getByRole('button',{name:'检查更正影响'}).click();
  await expect(memory().getByRole('alertdialog')).toContainText('预计聊天记录保持不变');
  await memory().getByRole('button',{name:'返回，不修改'}).click();
  assert.equal((await invoke(panel,'chat_usage_impact',{request:{appIds:[itemId],personalIds:[]}})).affectedTurnsTotal,0);
  // Localhost model + selection, then one real recorded turn that uses the entry.
  await invoke(panel,'model_settings_save',{config:{baseUrl:`http://127.0.0.1:${server.address().port}`,model:'b2-fixture',useApiKey:false,maxOutputTokens:128}});
  let preview=await invoke(pet,'chat_context_preview');
  await invoke(panel,'memory_policy_set',{request:{expectedScope:preview.scope,expectedEpoch:preview.contextEpoch,expectedRevision:preview.policy.revision,enabled:true,selectedIds:[itemId],restartConversation:false}});
  preview=await invoke(pet,'chat_context_preview');assert.equal(preview.policy.enabled,true);
  const turn=await pet.evaluate(async({preview})=>{const native=window.__TAURI_INTERNALS__,requestId=crypto.randomUUID();
    const callback=native.transformCallback(()=>{});
    // Protocol v3: expectedPersonal is required at parse time. null = the
    // personal-memory service is offline in this acceptance run.
    try{return {ok:true,result:await native.invoke('chat_generate',{request:{requestId,prompt:'本机测试',expectedScope:preview.scope,expectedContextEpoch:preview.contextEpoch,expectedPersonal:null},onDelta:`__CHANNEL__:${callback}`})};}
    catch(error){return {ok:false,error};}},{preview});
  assert(turn.ok,'fixture turn must complete');
  assert.equal((await invoke(pet,'chat_history')).length,1);
  // The ledger attributes the turn; the consult reports the affected scope.
  const impact=await invoke(panel,'chat_usage_impact',{request:{appIds:[itemId],personalIds:[]}});
  assert.equal(impact.affectedTurnsTotal,1);assert.equal(impact.scopes.length,1);
  assert.equal(impact.scopes[0].affectedTurns,1);assert.equal(impact.scopes[0].scope.model,'b2-fixture');
  // The dialog renders the precomputed count from the real command.
  await memory().locator('article').getByRole('button',{name:'更正',exact:true}).click();
  await memory().getByLabel('记忆内容').fill('合成偏好：先说结论，再列证据');
  await memory().getByRole('button',{name:'检查更正影响'}).click();
  await expect(memory().getByRole('alertdialog')).toContainText('预计清除最近 1 轮');
  await panel.screenshot({path:path.join(evidence,'impact-dialog.png')});
  // Confirm through the real backend: the request must survive the host's
  // deny_unknown_fields deserialization, the receipt reports the prune, and
  // the recorded turn is gone.
  await memory().getByRole('button',{name:'确认并开始新对话'}).click();
  await expect(memory().locator('article')).toContainText('先说结论，再列证据');
  await expect(memory().getByRole('status')).toContainText('已清除使用过该记忆的最近 1 轮对话');
  assert.equal((await invoke(pet,'chat_history')).length,0);
  const deniedIds=await denyOnPet({appIds:[itemId],personalIds:[]});
  assert(deniedIds,'pet window must not reach chat_usage_impact');
  await stop();assert.equal(requests.length,1);
  const result={passed:true,identifier:id,localhostRequests:requests.length,impact,
    petDenials:{empty:String(deniedEmpty&&deniedEmpty.message||deniedEmpty),ids:String(deniedIds&&deniedIds.message||deniedIds)},
    cases:['empty consult and pet ACL denial','zero dialog before usage','ledger attribution after one turn','precomputed dialog count','confirmed update through the real backend','pet ACL denial with ids']};
  fs.writeFileSync(path.join(evidence,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{await stop();server.closeAllConnections();server.close();}})().catch(error=>{console.error(error);process.exitCode=1;});
