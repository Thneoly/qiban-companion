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
  if (entry.isFile() && (/\.(?:html|cjs)$/i.test(entry.name) || (/\.md$/i.test(entry.name) && !['README.md','AGENTS.md'].includes(entry.name)))) {
    failures.push(`Root file should be categorized: ${entry.name}`);
  }
}
for(const directory of ['apps/desktop/tests/browser','packages/contracts','crates/companion-core','crates/companion-storage','docs/product','docs/planning','docs/architecture','docs/quality','docs/research','docs/status','prototypes/companion-validation','scripts/verification']) {
  assert(fs.statSync(path.join(root,directory)).isDirectory(), `Missing directory ${directory}`);
}
assert.equal(failures.length,0,failures.join('\n'));
console.log(`Verified ${files.length} Markdown files, ${links} local links and repository layout.`);
