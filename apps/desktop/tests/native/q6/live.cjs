// Model-quality replay only. State-transition and scope isolation are validated by runner.cjs.
// Requires an explicit invocation cap; credentials are read only by the native application.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {Harness}=require('./harness.cjs');
const cap=Number(process.env.QIBAN_Q6_LIVE_COUNT),base=process.env.QIBAN_Q6_BASE_URL,model=process.env.QIBAN_Q6_MODEL;
assert([5,50].includes(cap),'Set an explicitly agreed QIBAN_Q6_LIVE_COUNT=5 or 50; there is no default');
assert(base&&model,'Provide the intended non-secret API base and model');
const url=new URL(base);assert(url.protocol==='https:'&&!url.username&&!url.password&&!url.search&&!url.hash,'A credential-free HTTPS base is required');
const suite=JSON.parse(fs.readFileSync(path.join(__dirname,'../../fixtures/memory-q6-v1.json'),'utf8'));
const pilot=['P01','E02','C01','S01','N01'];
(async()=>{
  const h=new Harness(),records=suite.cases.map(c=>({id:c.id,status:'not_run',sourceCorrect:null,omission:null,misuse:null,response:null,notes:''}));let attempted=0;
  try{
    await h.setup();
    const selected=cap===5?suite.cases.filter(c=>pilot.includes(c.id)):suite.cases;
    for(const c of selected){
      const record=records.find(r=>r.id===c.id);
      try{
        await h.reset(c.id);await h.model(base,model,true,2048);
        const ids=[];for(const r of c.expected.references)ids.push((await h.create(r.body,r.kind,r.eventDate)).id);
        if(ids.length)await h.policy(ids);
        const preview=await h.preview();assert.equal(preview.items.length,c.expected.references.length);
        assert(attempted<cap);attempted++;
        const result=await h.send(preview,c.prompt);
        Object.assign(record,{status:result.ok?'unreviewed':'request_failed',preview,result,response:result.deltas.map(d=>d.text).join(''),rubric:c.review.required});
        if(!result.ok){process.exitCode=1;break;} // Keep failed and unattempted cases; do not retry or probe alternatives.
      }catch(error){process.exitCode=1;record.status='setup_failed';record.error=typeof error==='string'?error:String(error?.code||error);break;}
      finally{
        h.write('live-review.json',{...h.meta,mode:'model-quality-replay',suiteVersion:suite.version,model,baseUrl:base,maxOutputTokens:2048,invocationCap:cap,attempted,denominator:50,sourceCorrect:null,reviewer:null,cases:records,m4Gate:'hold'});
      }
    }
    console.log(JSON.stringify({runId:h.meta.runId,mode:'model-quality-replay',attempted,cap,unreviewed:records.filter(r=>r.status==='unreviewed').length,failed:records.filter(r=>r.status.endsWith('failed')).map(r=>r.id),qualityScore:null,m4Gate:'hold'}));
  }finally{await h.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
