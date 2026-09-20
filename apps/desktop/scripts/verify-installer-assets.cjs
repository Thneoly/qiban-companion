// A packaging guard: local public/ research downloads must never enter an installer.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '../dist');
function inspect(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const file = path.join(directory, entry.name);
    assert(!entry.isSymbolicLink(), 'Installer assets cannot contain symlinks');
    if (entry.isDirectory()) return inspect(file);
    const relative = path.relative(root, file).replaceAll('\\', '/');
    assert(relative === 'index.html' || /^assets\/[\w-]+\.(js|css)$/.test(relative),
      `Unreviewed installer asset: ${relative}`);
    const text = fs.readFileSync(file, 'utf8');
    assert(!/live2d-local|Hiyori\.model3|live2dcubismcore|Live2D 实验/.test(text),
      `Research runtime or entry remains in ${relative}`);
    return [relative];
  });
}
assert(fs.existsSync(path.join(root, 'index.html')));
console.log(`Installer asset guard passed: ${inspect(root).length} files; no local research assets.`);
