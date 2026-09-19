const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const fixtures=path.resolve(__dirname,'../../fixtures/memory-q6-v1.json');
const suite=JSON.parse(fs.readFileSync(fixtures,'utf8'));
async function facts(h){
  assert.equal(suite.cases.length,50);assert.equal(new Set(suite.cases.map(c=>c.id)).size,50);
  const records=[];
  for(const c of suite.cases){
    const record={id:c.id,title:c.title,status:'failed',modelQuality:'not_run',review:c.review};
    try{
      await h.reset(c.id);const keys={},captures={};let name='A';
      for(const step of c.steps){
        const before=await h.list(),p=await h.preview();
        const checkpoint=step.snapshot?captures[step.snapshot]:before;
        const operation=async()=>{
          if(step.op==='capture'){captures[step.name]=before;return;}
          if(step.op==='scope'){name=step.name;await h.model(h.base+(name==='C'?'/alternate':''),c.id+(name==='B'?'-b':'-a'));return;}
          if(step.op==='restart'){await h.restart();return;}
          if(step.op==='create'){await h.mutate({action:'create',expectedEpoch:checkpoint.contextEpoch,draft:step.draft});keys[step.key]=(await h.list()).items.find(m=>!before.items.some(old=>old.id===m.id)).id;return;}
          if(step.op==='policy'){await h.policy(step.keys.map(key=>keys[key]),{enabled:step.enabled??!!step.keys.length});return;}
          const item=before.items.find(m=>m.id===keys[step.key]);assert(item);
          if(step.op==='update'){await h.mutate({action:'update',id:item.id,expectedRevision:step.revision??item.revision,expectedEpoch:checkpoint.contextEpoch,draft:step.draft,restartConversation:true});return;}
          if(step.op==='delete'){await h.mutate({action:'delete',id:item.id,expectedRevision:item.revision,expectedEpoch:p.contextEpoch,restartConversation:true});return;}
          throw Error('Unknown fixture operation: '+step.op);
        };
        if(step.expectError){await assert.rejects(operation(),e=>e.code===step.expectError);assert.deepEqual(await h.list(),before,'Rejected operation changed state');}
        else await operation();
      }
      const p=await h.preview(),expected=c.expected.references;
      assert.equal(p.items.length,expected.length);assert.equal(p.policy.enabled,!!expected.length);
      for(let i=0;i<expected.length;i++){
        const wanted=expected[i],actual=p.items[i];assert.equal(actual.id,keys[wanted.key]);
        for(const key of ['body','kind','eventDate','revision'])assert.equal(actual[key],wanted[key],c.id+' '+key);
        assert.equal(actual.sourceKind,'user_manual');assert.equal(actual.sourceLabel,'用户在记忆面板填写');
        assert(actual.createdAt>0&&actual.confirmedAt>=actual.createdAt&&actual.updatedAt===actual.confirmedAt);
      }
      assert.equal((await h.list()).items.length,c.expected.memoryCount);
      const beforeRequests=h.requests.length,result=await h.send(p,c.prompt);assert.equal(result.ok,true);assert.equal(h.requests.length,beforeRequests+1);
      const request=h.requests.at(-1),payload=request.payload;
      const blocks=payload.messages.filter(m=>m.role==='user'&&m.content.startsWith('{')&&m.content.includes('user_confirmed_reference'));
      assert.equal(blocks.length,expected.length?1:0);
      if(expected.length){const block=JSON.parse(blocks[0].content);assert.deepEqual(block.items,p.items);assert(!payload.messages[0].content.includes(expected[0].body));}
      for(const body of c.expected.forbiddenBodies)assert(!JSON.stringify(payload).includes(body),'Forbidden old/private body was sent');
      assert.deepEqual(result.result.memoryUsage.memories,p.items.map(m=>({id:m.id,revision:m.revision})));
      assert.equal(result.result.memoryUsage.contextEpoch,p.contextEpoch);assert.deepEqual(result.result.memoryUsage.scope,p.scope);
      assert.equal((await h.list()).items.length,c.expected.memoryCount,'Chat auto-created a memory');
      Object.assign(record,{status:'passed',preview:p,request,result});
    }catch(error){record.error=String(error?.stack||error);}
    records.push(record);h.write('facts.json',{...h.meta,suiteVersion:suite.version,fixtureSha256:crypto.createHash('sha256').update(fs.readFileSync(fixtures)).digest('hex'),mode:'local-transport',cases:records});
  }
  return records;
}
module.exports={facts};
