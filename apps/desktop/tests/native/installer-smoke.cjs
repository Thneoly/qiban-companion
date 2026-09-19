// Windows-only lifecycle smoke, using a fresh acceptance identity and synthetic data.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const http = require('node:http');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require('@playwright/test');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const config = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
assert.equal(process.platform, 'win32');
assert(/^dev\.qiban\.companion\.acceptance\.installer-[a-z0-9-]+$/.test(config.identifier));
assert(/^Qiban Installer Acceptance [a-zA-Z0-9-]+$/.test(config.productName));
const installer = path.resolve(process.argv[3]);
assert(path.basename(installer).startsWith(config.productName + '_'));
const profile = path.join(process.env.LOCALAPPDATA, config.identifier);
assert(!fs.existsSync(profile), 'Use a fresh acceptance identity; never erase a profile');
const root = path.resolve('.cache', 'installer-' + crypto.randomUUID());
const installDir = path.join(root, 'application');
assert(installDir.startsWith(path.resolve('.cache') + path.sep));
assert(!/\s/.test(installDir), 'NSIS /D test path must not contain spaces');
const executable = path.join(installDir, 'companion-desktop.exe');
fs.mkdirSync(root, { recursive: true });
const evidence = { runId: path.basename(root), startedAt: new Date().toISOString(),
  commit: execFileSync('git', ['-c', 'safe.directory=' + process.cwd().replaceAll('\\', '/'), 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  sourceDirty: !!execFileSync('git', ['-c', 'safe.directory=' + process.cwd().replaceAll('\\', '/'), 'status', '--porcelain'], { encoding: 'utf8' }).trim(),
  installerSha256: hash(installer), config, installDir, profile, checks: [], status: 'running', cleanDevice: false };
function save() { fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(evidence, null, 2)); }
function checked(name) { evidence.checks.push(name); save(); console.log('PASS ' + name); }
function shell(code) {
  return execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', code],
    { encoding: 'utf8', windowsHide: true, env: { ...process.env, QIBAN_TEST_PRODUCT: config.productName } }).trim();
}
function registration() {
  const text = shell("$p='HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\'+$env:QIBAN_TEST_PRODUCT; if(Test-Path -LiteralPath $p){Get-ItemProperty -LiteralPath $p | Select-Object DisplayName,DisplayVersion,InstallLocation | ConvertTo-Json -Compress}");
  return text ? JSON.parse(text) : null;
}
function run(file, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(file, args, { windowsHide: true, stdio: 'ignore' });
    const timer = setTimeout(() => { child.kill(); reject(Error('Installer timed out')); }, 60000);
    child.once('error', error => { clearTimeout(timer); reject(error); });
    child.once('exit', code => { clearTimeout(timer); code === 0 ? resolve() : reject(Error('Installer exit ' + code)); });
  });
}
let browser, child, pet, panel, server, requests = 0;
const port = 9455;
async function invoke(page, command, args) {
  return page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
}
async function start() {
  child = spawn(executable, [], { windowsHide: true, stdio: 'ignore', env: { ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    WEBVIEW2_USER_DATA_FOLDER: path.join(profile, 'smoke-webview') } });
  for (let i = 0; i < 100; i++) {
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); break; } catch { await delay(100); }
  }
  assert(browser, 'Installed app did not start');
  for (let i = 0; i < 100; i++) {
    const pages = browser.contexts().flatMap(context => context.pages());
    pet = pages.find(page => page.url().includes('tauri') && !page.url().includes('view=panel'));
    panel = pages.find(page => page.url().includes('view=panel'));
    if (pet && panel) break;
    await delay(100);
  }
  assert(pet && panel); assert(fs.existsSync(path.join(profile, 'instance.lock')));
  await pet.getByRole('button', { name: '和栖栖互动' }).waitFor();
}
async function stop() {
  if (browser) { await browser.close(); browser = null; }
  if (child && child.exitCode === null) {
    const own = child;
    await new Promise(resolve => { own.once('exit', resolve); own.kill(); });
  }
  child = null; await delay(300);
}
async function install() {
  await run(installer, ['/S', '/D=' + installDir]);
  assert(fs.existsSync(executable)); assert.equal(registration()?.DisplayName, config.productName);
  assert.equal(path.resolve(registration().InstallLocation.replace(/^"|"$/g, '')), installDir);
}
async function uninstall() {
  // _?= keeps the uninstaller in the validated private install directory so we can await it.
  await run(path.join(installDir, 'uninstall.exe'), ['/S', '_?=' + installDir]);
  assert(!fs.existsSync(executable)); assert.equal(registration(), null);
  assert(fs.existsSync(path.join(profile, 'chat-history.db')), 'Default uninstall must preserve data');
}
(async () => {
  try {
    assert.equal(registration(), null, 'Acceptance identity already registered');
    assert(!shell("Get-Process | Where-Object ProcessName -eq 'companion-desktop' | Select-Object -ExpandProperty Id"), 'Exit all companion instances before installer testing');
    let occupied = false;
    try { await fetch(`http://127.0.0.1:${port}/json/version`); occupied = true; } catch {}
    assert(!occupied, 'Debug port already owned');
    await install(); checked('install-current-user-and-register');
    evidence.binarySha256 = hash(executable);
    server = http.createServer((req, res) => {
      assert(!req.headers.authorization); requests++;
      req.resume(); req.on('end', () => { res.writeHead(200, { 'Content-Type': 'text/event-stream' });
        res.end('data: {"choices":[{"delta":{"content":"安装验收合成回答"}}]}\n\ndata: {"choices":[{"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n'); });
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    await start();
    assert.equal(await invoke(pet, 'guide_status'), false);
    await pet.getByRole('button', { name: '和栖栖互动' }).click();
    await pet.screenshot({ path: path.join(root, 'first-use.png') });
    await invoke(pet, 'guide_complete');
    await invoke(panel, 'create_task', { title: '安装验收合成待办' });
    await invoke(panel, 'model_settings_save', { config: { baseUrl: `http://127.0.0.1:${server.address().port}`, model: 'installer-fixture', useApiKey: false, maxOutputTokens: 128 } });
    await invoke(panel, 'memory_mutate', { request: { action: 'create', expectedEpoch: (await invoke(pet, 'chat_context_preview')).contextEpoch, draft: { kind: 'preference', body: '安装验收合成偏好', eventDate: null } } });
    const preview = await invoke(pet, 'chat_context_preview');
    await pet.evaluate(async p => {
      const n = window.__TAURI_INTERNALS__, callback = n.transformCallback(() => {});
      await n.invoke('chat_generate', { request: { requestId: crypto.randomUUID(), prompt: '安装验收', expectedScope: p.scope, expectedContextEpoch: p.contextEpoch }, onDelta: `__CHANNEL__:${callback}` });
    }, preview);
    assert.equal(requests, 1); checked('installed-ipc-save-and-local-chat');
    async function restored() {
      assert.equal(await invoke(pet, 'guide_status'), true);
      assert.equal((await invoke(panel, 'list_tasks')).length, 1);
      assert.equal((await invoke(panel, 'memory_list')).items[0].body, '安装验收合成偏好');
      assert.equal((await invoke(pet, 'chat_history')).length, 1);
      assert.equal((await invoke(panel, 'model_settings_get')).model, 'installer-fixture');
      await pet.getByRole('button', { name: '和栖栖互动' }).click();
      assert.equal(await pet.getByRole('button', { name: 'Live2D 实验' }).count(), 0);
    }
    await stop(); await start(); await restored(); checked('restart-restores-records-without-research-entry');
    await stop(); await install(); await start(); await restored(); checked('same-version-reinstall-preserves-records');
    await stop();
    const savedHash = hash(path.join(profile, 'chat-history.db'));
    await uninstall(); assert.equal(hash(path.join(profile, 'chat-history.db')), savedHash); checked('default-uninstall-retains-data');
    await install(); await start(); await restored(); checked('reinstall-after-uninstall-restores-records');
    await invoke(pet, 'chat_clear');
    await stop(); await start(); assert.deepEqual(await invoke(pet, 'chat_history'), []);
    assert.equal((await invoke(panel, 'memory_list')).items.length, 1); checked('clear-chat-persists-and-retains-memory');
    await stop(); await uninstall(); checked('final-uninstall-removes-app-and-registration');
    evidence.status = 'passed'; evidence.localhostRequests = requests;
  } catch (error) {
    evidence.status = 'failed'; evidence.error = String(error?.stack || error); process.exitCode = 1;
  } finally {
    await stop(); server?.closeAllConnections(); server?.close(); save(); console.log(JSON.stringify(evidence));
  }
})();
