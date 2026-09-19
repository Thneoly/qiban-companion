const assert=require('node:assert/strict');const {delay}=require('./harness.cjs');
async function deletion(h){
  const records=[];
  for(const id of ['DEL09','DEL10','DEL11','DEL16']){
    const record={id,layer:'native',status:'failed'};const start=h.requests.length;
    try{
      await h.reset(id);const item=await h.create(id+' 合成旧正文');await h.policy([item.id]);const old=await h.preview();
      const remove=()=>h.mutate({action:'delete',id:item.id,expectedRevision:item.revision,expectedEpoch:old.contextEpoch,restartConversation:true});
      if(id==='DEL09'||id==='DEL11'){
        const pending=h.send(old,'Q6_HOLD');
        for(let n=0;n<100&&!h.held;n++)await delay(50);assert(h.held,'Stream did not reach barrier');
        assert(JSON.stringify(h.requests.at(-1).payload).includes(item.body));
        if(id==='DEL09')await remove();
        else await h.mutate({action:'update',id:item.id,expectedRevision:item.revision,expectedEpoch:old.contextEpoch,draft:{body:'合成已更正值',kind:'preference',eventDate:null},restartConversation:true});
        h.release();assert.equal((await pending).ok,false);assert.deepEqual(await h.history(),[]);
      }else if(id==='DEL10'){
        await remove();assert.equal((await h.send(old,'stale')).ok,false);assert.equal(h.requests.length,start);
      }else{
        await h.create('后来新增，旧窗口不能删除');
        await assert.rejects(h.mutate({action:'delete_all',expectedEpoch:old.contextEpoch,restartConversation:true}),e=>e.code==='context_changed');
        await assert.rejects(h.mutate({action:'update',id:item.id,expectedRevision:1,expectedEpoch:old.contextEpoch,draft:{kind:'preference',body:'旧窗口覆盖',eventDate:null},restartConversation:true}),e=>e.code==='context_changed');
        assert.equal((await h.list()).items.length,2);assert.equal((await h.send(old,'stale')).ok,false);assert.equal(h.requests.length,start);
      }
      const current=await h.preview();assert.equal((await h.send(current,'新请求')).ok,true);
      if(id!=='DEL16')assert(!JSON.stringify(h.requests.at(-1).payload).includes(item.body));
      if(id==='DEL11')assert(JSON.stringify(h.requests.at(-1).payload).includes('合成已更正值'));
      await h.restart();assert.deepEqual((await h.preview()).items,current.items);
      Object.assign(record,{status:'passed',requests:h.requests.slice(start)});
    }catch(error){h.release();record.error=String(error?.stack||error);}
    records.push(record);h.write('deletion-native.json',{...h.meta,cases:records});
  }
  return records;
}
module.exports={deletion};
