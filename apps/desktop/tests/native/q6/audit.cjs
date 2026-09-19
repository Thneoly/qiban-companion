// Runs the 16 component deletion cases and joins them with the four native cases.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {spawnSync}=require('node:child_process');
const directory=path.resolve(process.argv[2]||'');assert(path.basename(directory).startsWith('dev.qiban.companion.acceptance.q6-'));
const native=JSON.parse(fs.readFileSync(path.join(directory,'deletion-native.json'),'utf8'));
const facts=JSON.parse(fs.readFileSync(path.join(directory,'facts.json'),'utf8'));
const fixture=fs.readFileSync(path.join(__dirname,'../../fixtures/memory-q6-v1.json'));
assert.equal(facts.fixtureSha256,crypto.createHash('sha256').update(fixture).digest('hex'),'Case set changed since native run');
assert(!fs.existsSync(path.join(directory,'audit.json')),'Never overwrite a previous audit');
const run=spawnSync('cargo',['test','--workspace','--locked','q6_del','--','--test-threads=1'],{encoding:'utf8',maxBuffer:8*1024*1024,windowsHide:true});
fs.writeFileSync(path.join(directory,'cargo-q6.log'),(run.stdout||'')+'\n'+(run.stderr||''));
const components=Array.from((run.stdout||'').matchAll(/^test (\S*::q6_del(\d{2})_\S+) \.\.\. ok\r?$/gm)).map(m=>({id:'DEL'+m[2],layer:'rust-component',test:m[1],status:'passed'}));
const cases=[...components,...native.cases].sort((a,b)=>a.id.localeCompare(b.id));
const wanted=Array.from({length:20},(_,i)=>'DEL'+String(i+1).padStart(2,'0'));
const complete=run.status===0&&cases.length===20&&new Set(cases.map(c=>c.id)).size===20&&wanted.every(id=>cases.some(c=>c.id===id&&c.status==='passed'));
const audit={runId:native.runId,commit:native.commit,binarySha256:native.binarySha256,fixtureSha256:facts.fixtureSha256,createdAt:new Date().toISOString(),localTransport:{total:50,passed:facts.cases.filter(c=>c.status==='passed').length},deletion:{expected:20,cases,complete},modelQuality:{status:'not_run',denominator:50,correct:null,reviewer:null},independentExperience:{status:'not_run',reviewer:null},m4Gate:'hold'};
fs.writeFileSync(path.join(directory,'audit.json'),JSON.stringify(audit,null,2));console.log(JSON.stringify({runId:audit.runId,transport:audit.localTransport,deletionComplete:complete,deletionCases:cases.length,m4Gate:'hold',modelQuality:'not_run'}));
if(!complete||audit.localTransport.passed!==50)process.exitCode=1;
