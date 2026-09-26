// Only a fresh acceptance profile is allowed by Harness; never production user data.
const {Harness}=require('./q6/harness.cjs');
const {expect}=require('@playwright/test');
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {DatabaseSync}=require('node:sqlite');
(async()=>{
  const h=new Harness(),checks=[];
  try{
    await h.setup();
    await assert.rejects(()=>h.invoke(h.pet,'execution_list'));checks.push('pet-window-denied');
    await h.invoke(h.pet,'pet_action',{action:'open_panel'});
    await h.panel.getByRole('button',{name:'打开文档任务',exact:true}).click();
    const source=path.join(h.evidence,'source.txt');fs.writeFileSync(source,'这是第一行。\n这是第二行。\n第三行引用。');
    await h.panel.getByLabel('选择文档',{exact:true}).setInputFiles(source);
    await h.panel.getByRole('button',{name:'生成摘录预览',exact:true}).click();
    await expect(h.panel.getByLabel('摘录预览',{exact:true})).toContainText('这是第一行');
    const first=(await h.invoke(h.panel,'execution_list'))[0];assert.equal(first.status,'waiting_confirmation');
    const artifact=path.join(h.directory,'document-drafts',first.artifactName);assert(!fs.existsSync(artifact));
    await assert.rejects(()=>h.invoke(h.panel,'execution_confirm',{id:first.id,expectedRevision:9}));checks.push('confirmation-required-and-versioned');
    const db=new DatabaseSync(path.join(h.directory,'executions.db'),{readOnly:true});
    const requestId=db.prepare('SELECT request_id FROM execution_tasks WHERE id=?').get(first.id).request_id;db.close();
    const duplicate=await h.invoke(h.panel,'execution_prepare',{requestId,sourceName:'source.txt',text:fs.readFileSync(source,'utf8')});assert.equal(duplicate.task.id,first.id);
    await assert.rejects(()=>h.invoke(h.panel,'execution_prepare',{requestId,sourceName:'source.txt',text:'changed'}));checks.push('prepare-dedup-and-payload-binding');
    await h.panel.getByRole('button',{name:'确认保存草稿',exact:true}).click();
    await expect(h.panel.getByRole('button',{name:'读取已保存草稿',exact:true})).toBeVisible();
    await h.panel.getByRole('button',{name:'读取已保存草稿',exact:true}).click();
    await expect(h.panel.getByLabel('已保存草稿',{exact:true})).toHaveText(first.preview);
    assert.equal(fs.readFileSync(artifact,'utf8'),first.preview);
    await h.invoke(h.panel,'execution_confirm',{id:first.id,expectedRevision:0});
    assert.equal((await h.invoke(h.panel,'execution_detail',{id:first.id})).attempts.length,1);checks.push('real-artifact-and-confirm-dedup');
    await h.panel.locator('.document-execution').screenshot({path:path.join(h.evidence,'execution-native.png')});
    const prepare=()=>h.invoke(h.panel,'execution_prepare',{requestId:crypto.randomUUID(),sourceName:'checkpoint.txt',text:'受控恢复材料'});
    const cancelled=(await prepare()).task;
    await h.invoke(h.panel,'execution_cancel',{id:cancelled.id,expectedRevision:0});
    await assert.rejects(()=>h.invoke(h.panel,'execution_confirm',{id:cancelled.id,expectedRevision:0}));
    assert(!fs.existsSync(path.join(h.directory,'document-drafts',cancelled.artifactName)));checks.push('cancel-before-publish');
    const pending=[(await prepare()).task,(await prepare()).task,(await prepare()).task];
    await h.stop();
    // Inject checkpoints only into the now-stopped, fresh synthetic profile.
    const ledger=new DatabaseSync(path.join(h.directory,'executions.db'));
    ledger.exec('BEGIN IMMEDIATE');
    for(const task of pending){
      task.status='running';task.revision=1;task.note='受控中断点';
      const attempt={id:crypto.randomUUID(),actionId:task.actionId,owner:'interrupted-fixture',leaseUntil:Date.now()+30000,status:'running'};
      ledger.prepare('UPDATE execution_tasks SET body=? WHERE id=?').run(JSON.stringify(task),task.id);
      ledger.prepare('UPDATE execution_actions SET approved_revision=0 WHERE task_id=?').run(task.id);
      ledger.prepare('INSERT INTO execution_attempts VALUES(?,?,?)').run(attempt.id,task.id,JSON.stringify(attempt));
      ledger.prepare('INSERT INTO execution_events(task_id,revision,status,created_at) VALUES(?,?,?,?)').run(task.id,1,JSON.stringify('running'),task.updatedAt);
    }
    ledger.exec('COMMIT');ledger.close();
    fs.writeFileSync(path.join(h.directory,'document-drafts',pending[0].artifactName),pending[0].preview,{flag:'wx'});
    fs.writeFileSync(path.join(h.directory,'document-drafts',pending[2].artifactName),'不可覆盖的冲突内容',{flag:'wx'});
    await h.start();
    const rows=await h.invoke(h.panel,'execution_list');
    assert.equal(rows.find(t=>t.id===first.id).status,'completed');
    assert.equal(rows.find(t=>t.id===pending[0].id).status,'completed');
    assert.equal(rows.find(t=>t.id===pending[1].id).status,'failed');
    assert.equal(rows.find(t=>t.id===pending[2].id).status,'unknown');
    assert.equal(fs.readFileSync(path.join(h.directory,'document-drafts',pending[2].artifactName),'utf8'),'不可覆盖的冲突内容');
    assert.equal(fs.readdirSync(path.join(h.directory,'document-drafts')).length,3);
    checks.push('restart-completed-restored','post-publish-receipt-recovered','pre-publish-not-retried','conflicting-artifact-kept-unknown');
    assert.equal(h.requests.length,0);
    h.write('execution-result.json',{...h.meta,checks,passed:true,modelRequests:0});
    console.log(JSON.stringify({runId:h.meta.runId,checks,modelRequests:0}));
  } finally {await h.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
