// Run against a fresh, isolated acceptance build; never uses a real model or credentials.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { DatabaseSync } = require('node:sqlite');
const { chromium } = require('@playwright/test');
const exe = process.env.QIBAN_ACCEPTANCE_EXE;
const id = process.env.QIBAN_ACCEPTANCE_ID;
assert(exe && fs.existsSync(exe), 'Set QIBAN_ACCEPTANCE_EXE to the isolated executable');
assert(/^dev\.qiban\.companion\.acceptance\.[a-z0-9-]+$/.test(id || ''), 'Require a dedicated acceptance identifier');
const directory = path.join(process.env.LOCALAPPDATA, id);
assert(!fs.existsSync(directory), 'Refusing to modify any existing profile; build with a fresh identifier');
fs.mkdirSync(directory);
const database = path.join(directory, 'chat-history.db');
const fixture = new DatabaseSync(database);
fixture.exec("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL); PRAGMA user_version=1;");
fixture.prepare('INSERT INTO chat_turns VALUES(7,?,?,?,?)').run('http://127.0.0.1:1', 'migration-fixture', 'synthetic question', 'synthetic answer');
fixture.close();
const port = Number(process.env.QIBAN_CDP_PORT || 9441);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const children = new Set();
let browser;
function launch() {
  const child = spawn(exe, [], { windowsHide: true, stdio: 'ignore', env: { ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    WEBVIEW2_USER_DATA_FOLDER: path.join(directory, 'acceptance-webview'),
  } });
  children.add(child); child.once('exit', () => children.delete(child));
  return child;
}
async function terminated(child) {
  for (let n = 0; n < 100; n++) { if (child.exitCode !== null || child.signalCode !== null) return; await delay(100); }
  throw Error('Second process failed to exit within 10s');
}
async function connect() {
  browser = undefined;
  for (let n = 0; n < 100; n++) {
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); break; } catch { await delay(100); }
  }
  assert(browser, 'No native WebView debug endpoint');
  let pet, panel;
  for (let n = 0; n < 100; n++) {
    const pages = browser.contexts().flatMap(context => context.pages());
    pet = pages.find(p => p.url().includes('tauri') && !p.url().includes('view=panel'));
    panel = pages.find(p => p.url().includes('view=panel'));
    if (pet && panel) break;
    await delay(100);
  }
  assert(pet && panel); await pet.getByRole('button', { name: '和栖栖互动' }).waitFor();
  return { pet, panel };
}
function invoke(page, command, args) {
  return page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
}
(async () => {
  try {
    // Never attach to an unrelated WebView already listening on the selected port.
    let portInUse = false;
    try { await fetch(`http://127.0.0.1:${port}/json/version`); portInUse = true; } catch { /* expected */ }
    assert(!portInUse, 'Choose an unused QIBAN_CDP_PORT');
    const first = launch();
    let { pet, panel } = await connect();
    const migrated = new DatabaseSync(database, { readOnly: true });
    const version = migrated.prepare('PRAGMA user_version').get().user_version;
    migrated.close();
    assert.equal(version, 4, 'Executable did not open the expected fresh profile; refusing settings changes');
    await invoke(panel, 'model_settings_save', { config: { baseUrl: 'http://127.0.0.1:1', model: 'migration-fixture', useApiKey: false, maxOutputTokens: 1024 } });
    assert.deepEqual(await invoke(pet, 'chat_history'), [{ user: 'synthetic question', assistant: 'synthetic answer' }]);
    await invoke(pet, 'pet_action', { action: 'hide' });
    const isVisible = () => execFileSync('powershell.exe', ['-NoProfile', '-File',
      path.join(__dirname, 'window-visible.ps1'), '-TargetProcessId', String(first.pid), '-WindowTitle', '栖栖 · 桌面伴侣'],
      { windowsHide: true, encoding: 'utf8', timeout: 10000 }).trim() === 'True';
    assert.equal(await isVisible(), false);
    await Promise.all(Array.from({ length: 4 }, () => terminated(launch())));
    assert.equal(first.exitCode, null);
    for (let n = 0; n < 50 && !(await isVisible()); n++) await delay(100);
    assert.equal(await isVisible(), true, 'Repeated launch must restore the existing pet');
    assert.equal(children.size, 1);
    await browser.close(); browser = undefined;
    first.kill(); await terminated(first); await delay(500);
    // A forced process exit must release the lease, even though the lock file remains.
    const restarted = launch(); ({ pet } = await connect());
    assert.deepEqual(await invoke(pet, 'chat_history'), [{ user: 'synthetic question', assistant: 'synthetic answer' }]);
    await browser.close(); browser = undefined; restarted.kill(); await terminated(restarted);
    const db = new DatabaseSync(database, { readOnly: true });
    assert.equal(db.prepare('PRAGMA user_version').get().user_version, 4);
    assert.equal(db.prepare('SELECT count(*) AS n FROM memory_policy').get().n, 0);
    assert.equal(db.prepare('SELECT count(*) AS n FROM memories').get().n, 0);
    assert.equal(db.prepare('SELECT context_epoch FROM memory_meta').get().context_epoch, 0);
    assert.equal(db.prepare('SELECT id FROM chat_turns').get().id, 7);
    db.close();
    console.log(JSON.stringify({ passed: true, identifier: id, cases: ['v1 migration and native chat restore', 'four duplicates exit and restore hidden pet', 'forced exit releases lease', 'restart preserves chat and empty memory defaults'] }));
  } finally {
    if (browser) await browser.close();
    for (const child of children) { child.kill(); await terminated(child); }
    // Retain synthetic evidence; no recursive removal and no real profile modifications.
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
