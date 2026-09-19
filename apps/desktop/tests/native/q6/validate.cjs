const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const suite=JSON.parse(fs.readFileSync(path.join(__dirname,'../../fixtures/memory-q6-v1.json'),'utf8'));
assert.equal(suite.version,'q6-v1');assert.equal(suite.synthetic,true);assert.equal(suite.cases.length,50);
assert.equal(new Set(suite.cases.map(c=>c.id)).size,50);
for(const group of ['preference','experience','correction','scope','negative'])assert.equal(suite.cases.filter(c=>c.group===group).length,10);
for(const c of suite.cases){
  assert(c.prompt&&c.title&&c.review.required&&c.review.source);assert.equal(c.review.status,'unreviewed');
  const created=new Set(),captures=new Set();
  for(const step of c.steps){assert(['create','policy','update','delete','restart','scope','capture'].includes(step.op));
    if(step.snapshot)assert(captures.has(step.snapshot));if(step.op==='capture')captures.add(step.name);
    if(step.op==='create'&&!step.expectError){assert(!created.has(step.key));created.add(step.key);}
    if(['update','delete'].includes(step.op))assert(created.has(step.key));
    if(step.op==='policy')for(const key of step.keys)assert(created.has(key));
  }
  assert(c.expected.references.length<=5);assert(c.expected.references.reduce((n,r)=>n+[...r.body].length,0)<=800);
  assert.equal(new Set(c.expected.references.map(r=>r.key)).size,c.expected.references.length);
  for(const r of c.expected.references){assert(created.has(r.key));assert(['preference','experience'].includes(r.kind));assert(r.revision>=1);assert(!c.expected.forbiddenBodies.includes(r.body));}
  assert(Number.isInteger(c.expected.memoryCount)&&c.expected.memoryCount>=0);
}
console.log('Q6 v1: 50 unique synthetic cases, five groups, source/rubric and explicit expectations validated. No model quality score assigned.');
