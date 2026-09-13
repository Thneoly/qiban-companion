// Documentation checks only; this does not test or approve the proposed product.
const fs = require('fs');
const path = require('path');
const assert = require('assert/strict');
const root = path.resolve(__dirname, '../..');
const files = ['docs/planning/agile-stage-gate-plan.md','docs/planning/delivery-backlog.md','docs/architecture/technical-feasibility.md','docs/quality/verification-protocol.md','docs/quality/design-review.md','docs/product/feasibility-report.md','docs/product/commercial-decision.md'];
const docs = Object.fromEntries(files.map(f=>[f,fs.readFileSync(path.join(root,f),'utf8')]));
for (const [f,s] of Object.entries(docs)) {
  assert(!s.includes('\uFFFD'), `${f}: invalid encoding`);
  let fenced=false, width=null;
  for (const line of s.split(/\r?\n/)) {
    if (/^```/.test(line)) { fenced=!fenced; width=null; continue; }
    if (fenced) continue;
    if (line.startsWith('|')) {
      const count=(line.match(/(?<!\\)\|/g)||[]).length;
      if(width===null)width=count;
      assert.equal(count,width,`${f}: inconsistent table: ${line}`);
    } else width=null;
  }
  assert(!fenced,`${f}: unmatched fence`);
  for(const match of s.matchAll(/\]\(([^)]+\.md)(?:#[^)]*)?\)/g)) {
    const target=match[1];
    if(/^https?:/.test(target))continue;
    assert(fs.existsSync(path.resolve(root,path.dirname(f),target)),`${f}: missing link ${target}`);
  }
}
const b=docs['docs/planning/delivery-backlog.md'];
const estimates={};
for (const line of b.split('\n').filter(l=>/^\| T\d{2}[ab]?／/.test(l))) {
  const cells=line.split('|').map(s=>s.trim());
  const id=cells[1].split('／')[0];
  assert(!estimates[id],`duplicate ${id}`);
  const effort=cells.find(c=>/^(A\+B|A|B)／/.test(c));
  assert(effort,`missing effort ${id}`);
  const m=effort.match(/^(A\+B|A|B)／(\d+)～(\d+)/);
  assert(m,`invalid effort ${id}`);
  estimates[id]={role:m[1],low:+m[2],high:+m[3]};
}
assert.equal(Object.keys(estimates).length,48); // 47 IDs, with T18 split into a/b.
for(let i=1;i<=47;i++) {
  if(i===18) {assert(estimates.T18a && estimates.T18b);continue;}
  assert(estimates['T'+String(i).padStart(2,'0')],`missing task ${i}`);
}
const totals={A:[0,0],B:[0,0]};
for(const e of Object.values(estimates))for(const r of ['A','B'])if(e.role.includes(r)) {
  const d=e.role==='A+B'?2:1;totals[r][0]+=e.low/d;totals[r][1]+=e.high/d;
}
assert.deepEqual(totals,{A:[62,110],B:[42,79]});
const scheduled={};const roleSum={A:0,B:0};let rounds=0;
for(const l of b.split('\n').filter(l=>/^\| S\d+／/.test(l))) {
  rounds++;
  const c=l.split('|').map(s=>s.trim());
  for(const [role,col,totalCol] of [['A',2,3],['B',4,5]]) {
    let sum=0;
    for(const m of c[col].matchAll(/(T\d{2}[ab]?) ([\d.]+)/g)) {
      const id=m[1],n=+m[2];assert(estimates[id],`unknown scheduled ${id}`);
      assert(estimates[id].role.includes(role),`wrong owner ${id}`);
      sum+=n;scheduled[id]=(scheduled[id]||0)+n;
    }
    assert.equal(sum,+c[totalCol],`${c[1]} ${role}: sum`);
    assert(sum<=7,`${c[1]} ${role}: over capacity`);roleSum[role]+=sum;
  }
}
assert.equal(rounds,14);
for(const [id,e] of Object.entries(estimates))assert.equal(scheduled[id],(e.low+e.high)/2,`${id}: midpoint allocation`);
assert.deepEqual(roleSum,{A:86,B:60.5});
const p=docs['docs/planning/agile-stage-gate-plan.md'];
assert(!p.includes('或已落实且可履约的纠正方案'));
assert(p.includes('T41～T47') && p.includes('Hold 扩批'));
console.log(JSON.stringify({documents:files.length,workPackages:Object.keys(estimates).length,rounds,effortRange:totals,midpointAllocation:roleSum,status:'Documentation structure, links and effort/capacity checks passed. No product tests executed.'},null,2));
