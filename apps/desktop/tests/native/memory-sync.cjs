// Windows acceptance for the cross-device memory sync engine's local half:
// real UI mutations must enqueue/fold in the outbox (schema v5) in the same
// transaction, and the tombstone invariant must hold after delete-all. The
// HTTP convergence half (pull/push against a live coordinator, phone peer)
// is exercised by the Rust integration tests in src/account/tests.rs — the
// third loop itself needs no coordinator to prove its storage contract.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {spawn}=require('node:child_process');
const {DatabaseSync}=require('node:sqlite'); const {chromium,expect}=require('@playwright/test');
const exe=process.env.QIBAN_ACCEPTANCE_EXE,id=process.env.QIBAN_ACCEPTANCE_ID;
assert(exe&&fs.existsSync(exe));assert(/^dev\.qiban\.companion\.acceptance\.m2-[a-z0-9-]+$/.test(id||''));
const directory=path.join(process.env.LOCALAPPDATA,id); assert(!fs.existsSync(directory),'Refusing an existing profile');
const evidence=path.resolve('.cache',id+'-'+require('node:crypto').randomUUID());fs.mkdirSync(evidence,{recursive:true});
const port=Number(process.env.QIBAN_CDP_PORT||9443),delay=ms=>new Promise(r=>setTimeout(r,ms));
let child,browser,pet,panel;
const read=(sql,...args)=>{const db=new DatabaseSync(path.join(directory,'chat-history.db'),{readOnly:true});try{return db.prepare(sql).all(...args);}finally{db.close();}};
async function start(){
  browser=undefined;
  child=spawn(exe,[],{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(directory,'test-webview')}});
  for(let n=0;n<100;n++){try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);break;}catch{await delay(100);}}
  assert(browser);pet=panel=undefined;
  for(let n=0;n<100;n++){const pages=browser.contexts().flatMap(c=>c.pages());pet=pages.find(p=>p.url().includes('tauri')&&!p.url().includes('view=panel'));panel=pages.find(p=>p.url().includes('view=panel'));if(pet&&panel)break;await delay(100);}
  assert(pet&&panel);await pet.getByRole('button',{name:'和栖栖互动'}).waitFor();
  assert(fs.existsSync(path.join(directory,'instance.lock')),'Wrong build profile; refusing mutations');
  assert.equal(read('PRAGMA user_version')[0].user_version,7);
  await invoke(pet,'pet_action',{action:'open_memory'});
  await panel.getByRole('region',{name:'我们的记忆'}).getByRole('button',{name:'保存记忆',exact:true}).waitFor();
}
async function stop(){if(browser)await browser.close();browser=undefined;if(child){const process=child;child=undefined;const exited=new Promise(r=>process.once('exit',r));process.kill();await exited;}await delay(400);}
const invoke=(page,command,args)=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
(async()=>{try{
  let occupied=false;try{await fetch(`http://127.0.0.1:${port}/json/version`);occupied=true;}catch{} assert(!occupied);
  await start();
  const memory=()=>panel.getByRole('region',{name:'我们的记忆'});
  const outbox=()=>read('SELECT request_id,op,memory_id,kind,body FROM memory_outbox ORDER BY seq');

  // Case 1 — a real UI create lands in the outbox in the same transaction
  // and survives a restart (durable, not an in-memory queue).
  await memory().getByLabel('记忆类型').selectOption('preference');
  await memory().getByLabel('记忆内容').fill('合成偏好：跨端删除同步验收');
  await memory().getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(memory().locator('article')).toContainText('跨端删除同步验收');
  let rows=outbox();assert.equal(rows.length,1);
  assert.equal(rows[0].op,'create');assert.equal(rows[0].body,'合成偏好：跨端删除同步验收');
  const firstRequestId=rows[0].request_id;
  const memoryId=rows[0].memory_id;
  await stop();await start();
  rows=outbox();assert.equal(rows.length,1);assert.equal(rows[0].request_id,firstRequestId,'outbox row is durable');

  // Case 2 — a real UI correction folds onto the create: still exactly one
  // row per memory id, newest wording, and a fresh request id (an in-flight
  // push of the old payload must never ack the folded change).
  await memory().locator('article').getByRole('button',{name:'更正',exact:true}).click();
  await memory().getByLabel('记忆内容').fill('合成偏好：已更正的跨端内容');
  await memory().getByRole('button',{name:'检查更正影响'}).click();
  await memory().getByRole('button',{name:'确认并开始新对话'}).click();
  await expect(memory().locator('article')).toContainText('已更正的跨端内容');
  rows=outbox();assert.equal(rows.length,1);
  assert.equal(rows[0].op,'create');assert.equal(rows[0].body,'合成偏好：已更正的跨端内容');
  assert.notEqual(rows[0].request_id,firstRequestId,'folding rewrites the request id');

  // Case 3 — delete-all after never-pushed creates: the fold keeps a DELETE
  // row per entry (an in-flight create push may exist), and the tombstone
  // invariant holds in the real database.
  await memory().getByLabel('记忆内容').fill('第二条合成记忆');
  await memory().getByRole('button',{name:'保存记忆',exact:true}).click();
  await expect(memory().locator('article').nth(0)).toBeVisible();
  assert.equal(outbox().length,2,'one pending row per memory id');
  await memory().getByRole('button',{name:'删除全部记忆'}).click();
  await memory().getByRole('button',{name:'确认并开始新对话'}).click();
  await expect(memory()).toContainText('还没有留下记忆');
  rows=outbox();assert.equal(rows.length,2);
  assert(rows.every(r=>r.op==='delete'&&r.body===null&&r.kind===null),'folded deletes keep content-free rows');
  const tombstones=read("SELECT count(*) AS n FROM memories WHERE deleted_at IS NOT NULL AND body IS NULL AND source_label IS NULL AND event_date IS NULL")[0].n;
  assert.equal(tombstones,2);
  assert.equal(read('SELECT count(*) AS n FROM memories WHERE deleted_at IS NULL')[0].n,0);
  await panel.locator('.memory-panel').screenshot({path:path.join(evidence,'memory-sync.png')});
  await stop();

  const result={passed:true,identifier:id,cases:['ui create enqueues durably','ui correction folds one row with a fresh request id','delete-all enqueues per-entry deletes and keeps tombstones content-free'],note:'HTTP convergence (pull/push/phone peer) covered by the Rust integration tests against a live coordinator fixture'};
  fs.writeFileSync(path.join(evidence,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{await stop();}})().catch(error=>{console.error(error);process.exitCode=1;});
