// Windows acceptance for app-local reminders (TF-E1): user-created via the
// panel UI, fired as a pet-window bubble (in-app only — no OS notification is
// claimed or sent), cancelled while pending, and recovered across a restart.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {spawn}=require('node:child_process');
const {DatabaseSync}=require('node:sqlite'); const {chromium,expect}=require('@playwright/test');
const exe=process.env.QIBAN_ACCEPTANCE_EXE,id=process.env.QIBAN_ACCEPTANCE_ID;
assert(exe&&fs.existsSync(exe));assert(/^dev\.qiban\.companion\.acceptance\.m2-[a-z0-9-]+$/.test(id||''));
const directory=path.join(process.env.LOCALAPPDATA,id); assert(!fs.existsSync(directory),'Refusing an existing profile');
const evidence=path.resolve('.cache',id+'-'+require('node:crypto').randomUUID());fs.mkdirSync(evidence,{recursive:true});
const port=Number(process.env.QIBAN_CDP_PORT||9445),delay=ms=>new Promise(r=>setTimeout(r,ms));
let child,browser,pet,panel;
async function start(){
  browser=undefined;
  child=spawn(exe,[],{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(directory,'test-webview')}});
  for(let n=0;n<100;n++){try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);break;}catch{await delay(100);}}
  assert(browser);pet=panel=undefined;
  for(let n=0;n<100;n++){const pages=browser.contexts().flatMap(c=>c.pages());pet=pages.find(p=>p.url().includes('tauri')&&!p.url().includes('view=panel'));panel=pages.find(p=>p.url().includes('view=panel'));if(pet&&panel)break;await delay(100);}
  assert(pet&&panel);await pet.getByRole('button',{name:'和栖栖互动'}).waitFor();
  assert(fs.existsSync(path.join(directory,'instance.lock')),'Wrong build profile; refusing mutations');
  await invoke(pet,'pet_action',{action:'open_memory'});
  await panel.getByRole('region',{name:'应用内提醒'}).waitFor();
}
async function stop(){if(browser)await browser.close();browser=undefined;if(child){const process=child;child=undefined;const exited=new Promise(r=>process.once('exit',r));process.kill();await exited;}await delay(400);}
const contentInput=section=>section().locator('input').first();
const minutesInput=section=>section().locator('input').nth(1);
const invoke=(page,command,args)=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
(async()=>{try{
  let occupied=false;try{await fetch(`http://127.0.0.1:${port}/json/version`);occupied=true;}catch{} assert(!occupied);
  await start();
  const section=()=>panel.getByRole('region',{name:'应用内提醒'});
  // Case 1 — create via the real UI; the list shows it as 待提醒 with a cancel.
  await contentInput(section).fill('验收：该喝水了');
  await minutesInput(section).fill('1');
  await section().getByRole('button',{name:'设置提醒'}).click();
  const row=section().locator('li').filter({hasText:'验收：该喝水了'});
  await expect(row).toContainText('验收：该喝水了');
  await expect(row).toContainText('待提醒');
  await expect(row.getByRole('button',{name:'取消',exact:true})).toHaveCount(1);
  // Validation: a bad minute count is refused with an honest message.
  await contentInput(section).fill('坏时间');
  await minutesInput(section).fill('0');
  await section().getByRole('button',{name:'设置提醒'}).click();
  await expect(panel.getByRole('alert')).toContainText('1 分钟到 24 小时');
  // Case 2 — a second reminder to survive the restart as pending.
  await contentInput(section).fill('验收：重启后仍待提醒');
  await minutesInput(section).fill('30');
  await section().getByRole('button',{name:'设置提醒'}).click();
  await expect(section().locator('li').filter({hasText:'重启后仍待提醒'})).toContainText('待提醒');
  await panel.screenshot({path:path.join(evidence,'reminders-panel.png'),fullPage:true});
  // Case 3 — due reminder fires ONCE as a pet bubble (the firing loop runs
  // every 15s; a 1-minute reminder fires within ~75s) and settles to 已提醒.
  // The bubble is the user-visible firing evidence, so it is asserted for
  // real — a silent regression of the pet-window listener fails here, not
  // just in the database.
  await expect(pet.getByText(/⏰|验收：该喝水了/).first()).toBeVisible({timeout:90_000});
  await expect(row).toContainText('已提醒',{timeout:90_000});
  // The bubble retracts itself after ten seconds.
  await expect(pet.getByText('验收：该喝水了')).toHaveCount(0,{timeout:20_000});
  // Cancel the pending 30-minute one.
  const pending=section().locator('li').filter({hasText:'重启后仍待提醒'});
  await pending.getByRole('button',{name:'取消',exact:true}).click();
  await expect(pending).toContainText('已取消');
  // Case 4 — restart recovery: a fresh pending reminder survives as pending.
  await contentInput(section).fill('验收：跨重启');
  await minutesInput(section).fill('20');
  await section().getByRole('button',{name:'设置提醒'}).click();
  await expect(section().locator('li').filter({hasText:'跨重启'})).toContainText('待提醒');
  await stop();await start();
  await expect(panel.getByRole('region',{name:'应用内提醒'}).locator('li').filter({hasText:'跨重启'})).toContainText('待提醒');
  await stop();
  // Authoritative database state: exactly one fired, one cancelled, one
  // pending across the restart; no system notification can be claimed
  // because none exists in the codebase — the statuses speak for themselves.
  const db=new DatabaseSync(path.join(directory,'chat-history.db'),{readOnly:true});
  const statuses=db.prepare('SELECT status,COUNT(*) AS n FROM reminders GROUP BY status ORDER BY status').all();
  db.close();
  assert.deepStrictEqual(statuses.map(s=>[s.status,s.n]).sort(),[['cancelled',1],['fired',1],['pending',1]]);
  const result={passed:true,identifier:id,cases:['ui create with validation','due reminder fires and settles to fired','cancel while pending','restart recovery keeps pending']};
  fs.writeFileSync(path.join(evidence,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{await stop();}})().catch(error=>{console.error(error);process.exitCode=1;});
