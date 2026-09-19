// Drives only a fresh, explicitly named acceptance build; never a production profile.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),http=require('node:http'),crypto=require('node:crypto');
const {spawn,execFileSync}=require('node:child_process');
const {DatabaseSync}=require('node:sqlite');const {chromium}=require('@playwright/test');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
class Harness {
  constructor(){
    this.exe=process.env.QIBAN_ACCEPTANCE_EXE;this.id=process.env.QIBAN_ACCEPTANCE_ID;
    assert(this.exe&&fs.existsSync(this.exe));assert(/^dev\.qiban\.companion\.acceptance\.q6-[a-z0-9-]+$/.test(this.id||''));
    this.directory=path.join(process.env.LOCALAPPDATA,this.id);assert(!fs.existsSync(this.directory),'Refusing an existing profile');
    this.evidence=path.resolve('.cache',this.id+'-'+crypto.randomUUID());fs.mkdirSync(this.evidence,{recursive:true});
    this.port=Number(process.env.QIBAN_CDP_PORT||9444);this.requests=[];
    this.meta={runId:path.basename(this.evidence),startedAt:new Date().toISOString(),commit:execFileSync('git',['-c','safe.directory='+process.cwd().replaceAll('\\','/'),'rev-parse','HEAD'],{encoding:'utf8'}).trim(),binarySha256:crypto.createHash('sha256').update(fs.readFileSync(this.exe)).digest('hex'),identifier:this.id,schemaVersion:2,protocolVersion:2,reviewer:null};
    this.meta.sourceDirty=!!execFileSync('git',['-c','safe.directory='+process.cwd().replaceAll('\\','/'),'status','--porcelain'],{encoding:'utf8'}).trim();
    this.meta.harnessSha256=crypto.createHash('sha256').update(fs.readdirSync(__dirname).filter(n=>n.endsWith('.cjs')).sort().map(n=>n+'\n'+fs.readFileSync(path.join(__dirname,n),'utf8')).join('\n')).digest('hex');
  }
  async setup(){
    let occupied=false;try{await fetch(`http://127.0.0.1:${this.port}/json/version`);occupied=true;}catch{}assert(!occupied,'Debug port is already owned');
    this.server=http.createServer((req,res)=>{let body='';req.on('data',chunk=>body+=chunk);req.on('end',()=>{
      assert(!req.headers.authorization);const payload=JSON.parse(body);this.requests.push({url:req.url,payload});
      res.writeHead(200,{'Content-Type':'text/event-stream'});res.write('data: '+JSON.stringify({choices:[{delta:{content:'本地夹具回复，不计入模型质量'}}]})+'\n\n');
      if(payload.messages.at(-1).content==='Q6_HOLD'){this.held=res;return;}
      res.end('data: {"choices":[{"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
    });});
    await new Promise(r=>this.server.listen(0,'127.0.0.1',r));this.base=`http://127.0.0.1:${this.server.address().port}`;await this.start();
  }
  async start(){
    this.child=spawn(this.exe,[],{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${this.port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(this.directory,'test-webview')}});
    for(let n=0;n<100;n++){try{this.browser=await chromium.connectOverCDP(`http://127.0.0.1:${this.port}`);break;}catch{await delay(100);}}
    assert(this.browser,'Acceptance browser failed to start');this.pet=this.panel=undefined;
    for(let n=0;n<100;n++){const pages=this.browser.contexts().flatMap(c=>c.pages());this.pet=pages.find(p=>p.url().includes('tauri')&&!p.url().includes('view=panel'));this.panel=pages.find(p=>p.url().includes('view=panel'));if(this.pet&&this.panel)break;await delay(100);}
    assert(this.pet&&this.panel);await this.pet.getByRole('button',{name:'和栖栖互动'}).waitFor();
    assert(fs.existsSync(path.join(this.directory,'instance.lock')),'Wrong build profile');
    const db=new DatabaseSync(path.join(this.directory,'chat-history.db'),{readOnly:true});assert.equal(db.prepare('PRAGMA user_version').get().user_version,2);db.close();
    assert.equal((await this.invoke(this.pet,'get_runtime_info')).protocolVersion,2);
  }
  async stop(){if(this.browser)await this.browser.close();this.browser=undefined;if(this.child){const child=this.child;this.child=undefined;const exited=new Promise(r=>child.once('exit',r));child.kill();await exited;}await delay(250);}
  async close(){await this.stop();this.server?.closeAllConnections();this.server?.close();}
  async restart(){await this.stop();await this.start();}
  async invoke(page,command,args){
    // Preserve structured IPC error codes across the Playwright boundary.
    const result=await page.evaluate(async({command,args})=>{try{return {ok:true,value:await window.__TAURI_INTERNALS__.invoke(command,args)};}catch(error){return {ok:false,error};}},{command,args});
    if(!result.ok)throw result.error;return result.value;
  }
  preview(){return this.invoke(this.pet,'chat_context_preview');}
  list(){return this.invoke(this.panel,'memory_list');}
  mutate(request){return this.invoke(this.panel,'memory_mutate',{request});}
  async model(baseUrl,model,useApiKey=false,maxOutputTokens=128){await this.invoke(this.panel,'model_settings_save',{config:{baseUrl,model,useApiKey,maxOutputTokens}});}
  async policy(ids,extra={}){const p=await this.preview();return this.invoke(this.panel,'memory_policy_set',{request:{expectedScope:p.scope,expectedEpoch:p.contextEpoch,expectedRevision:p.policy.revision,enabled:!!ids.length,selectedIds:ids,restartConversation:true,...extra}});}
  async reset(label){const p=await this.preview();await this.mutate({action:'delete_all',expectedEpoch:p.contextEpoch,restartConversation:true});await this.model(this.base,label+'-a');}
  async create(body,kind='preference',eventDate=null){const before=await this.list();await this.mutate({action:'create',expectedEpoch:before.contextEpoch,draft:{body,kind,eventDate}});return (await this.list()).items.find(m=>!before.items.some(old=>old.id===m.id));}
  async send(p,prompt){return this.pet.evaluate(async({p,prompt})=>{
    const native=window.__TAURI_INTERNALS__,deltas=[];const requestId=crypto.randomUUID();const callback=native.transformCallback(raw=>{if(raw.message)deltas.push(raw.message);});
    try{return {ok:true,requestId,result:await native.invoke('chat_generate',{request:{requestId,prompt,expectedScope:p.scope,expectedContextEpoch:p.contextEpoch},onDelta:`__CHANNEL__:${callback}`}),deltas};}
    catch(error){return {ok:false,requestId,error,deltas};}
  },{p,prompt});}
  async history(){return this.invoke(this.pet,'chat_history');}
  release(){this.held?.end('data: {"choices":[{"delta":{"content":"迟到旧回复"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');this.held=undefined;}
  write(name,data){fs.writeFileSync(path.join(this.evidence,name),JSON.stringify(data,null,2));}
}
module.exports={Harness,delay};
