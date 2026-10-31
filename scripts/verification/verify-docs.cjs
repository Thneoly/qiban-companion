// Check repository document links and the agreed source layout, independent of caller cwd.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '../..');
function markdownFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const file = path.join(directory, entry.name);
    return entry.isDirectory() ? markdownFiles(file) : entry.name.endsWith('.md') ? [file] : [];
  });
}
const files = ['README.md', 'AGENTS.md'].map(file => path.join(root,file)).concat(markdownFiles(path.join(root,'docs')));
const failures = [];
let links = 0;
for (const file of files) {
  const body = fs.readFileSync(file,'utf8').replace(/```[\s\S]*?```/g,'');
  for (const match of body.matchAll(/\]\(([^\s)]+)\)/g)) {
    const target = match[1];
    if (/^(?:https?:|mailto:|#)/i.test(target)) continue;
    if (/^[a-z]:[\\/]/i.test(target) || path.isAbsolute(target)) {
      failures.push(`${path.relative(root,file)}: absolute local link ${target}`);
      continue;
    }
    const resolved = path.resolve(path.dirname(file),decodeURIComponent(target.split('#')[0]));
    links++;
    if (!fs.existsSync(resolved)) failures.push(`${path.relative(root,file)}: missing ${target}`);
  }
}
for (const entry of fs.readdirSync(root,{withFileTypes:true})) {
  if (entry.isFile() && (/\.(?:html|cjs)$/i.test(entry.name) || (/\.md$/i.test(entry.name) && !['README.md','AGENTS.md','CHANGELOG.md'].includes(entry.name)))) {
    failures.push(`Root file should be categorized: ${entry.name}`);
  }
}
// The doc-center index must stay structurally sound. Two checks, both born
// from a real incident: a pasted fragment (a stray "D" and a leaked commit
// message) once landed between the table rows of docs/README.md, splitting
// the 状态 section out of the rendered table — and link checking stayed
// green because the damage contained no link syntax.
const indexPath = path.join(root,'docs/README.md');
const indexBody = fs.readFileSync(indexPath,'utf8');
// 1) No non-table line may sit BETWEEN table rows (prose before/after the
//    table is fine; interposed lines break the render).
const lines = indexBody.split(/\r?\n/).map(line=>line.trim());
const nonEmpty = lines.map((text,i)=>({text,i})).filter(l=>l.text!=='');
for (let n=0;n<nonEmpty.length;n++) {
  const prev=nonEmpty[n-1]?.text??'',next=nonEmpty[n+1]?.text??'',cur=nonEmpty[n].text;
  if(!cur.startsWith('|')&&prev.startsWith('|')&&next.startsWith('|'))
    failures.push(`docs/README.md line ${nonEmpty[n].i+1}: non-table line inside the index table`);
}
// 2) Every docs/ markdown file must be reachable from the index (the table
//    above only guards files someone remembered to list).
for (const file of markdownFiles(path.join(root,'docs'))) {
  const rel=path.relative(path.join(root,'docs'),file).split(path.sep).join('/');
  if(rel==='README.md')continue;
  if(!indexBody.includes(`](${rel}`)) failures.push(`docs/README.md: index is missing docs/${rel}`);
}
for(const directory of ['apps/desktop/tests/browser','packages/contracts','crates/companion-core','crates/companion-storage','docs/product','docs/planning','docs/architecture','docs/quality','docs/research','docs/status','prototypes/companion-validation','scripts/verification']) {
  assert(fs.statSync(path.join(root,directory)).isDirectory(), `Missing directory ${directory}`);
}
assert.equal(failures.length,0,failures.join('\n'));
console.log(`Verified ${files.length} Markdown files, ${links} local links and repository layout.`);
