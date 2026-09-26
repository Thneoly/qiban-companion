import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

const exec = promisify(execFile);
const script = fileURLToPath(new URL('./login.ps1', import.meta.url));
const quote = value => `'${value.replaceAll("'", "''")}'`;

async function fixture(mode) {
  const calls = [];
  const challengeId = randomUUID();
  const profile = { accountId: randomUUID(), companionId: randomUUID() };
  const token = `qbs_${'x'.repeat(43)}`;
  const server = createServer(async (req, res) => {
    let body = '';
    for await (const chunk of req) body += chunk;
    calls.push({ path: req.url, body, authorization: req.headers.authorization });
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/healthz') {
      if (mode === 'redirect') {
        res.writeHead(302, { Location: '/unexpected-redirect' }); res.end(); return;
      }
      res.end('{"status":"ok"}'); return;
    }
    if (req.url === '/v1/auth/request-code') {
      if (mode === 'limited') { res.writeHead(429); res.end('{"error":{"code":"rate_limited"}}'); return; }
      res.end(JSON.stringify({ challengeId })); return;
    }
    if (req.url === '/v1/auth/verify-code') {
      // Simulate a committed login whose first response is lost on the wire.
      if (calls.filter(call => call.path === req.url).length === 1) { req.socket.destroy(); return; }
      res.end(JSON.stringify({ accessToken: token })); return;
    }
    if (req.url === '/v1/me') { res.end(JSON.stringify(profile)); return; }
    res.writeHead(404); res.end('{}');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    // Only the test replaces console input; the production client makes real HTTP requests.
    const command = `. ${quote(script)}; function Read-Host { param($Prompt,[switch]$AsSecureString) if ($AsSecureString) { return ConvertTo-SecureString '12345678' -AsPlainText -Force }; return 'fixture@example.invalid' }; try { Start-CoordinatorLogin ${server.address().port} | ConvertTo-Json -Compress } catch { Write-Host $_.Exception.Message; exit 1 }`;
    let result;
    try { result = { ...(await exec('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', command], { timeout: 25000, windowsHide: true })), code: 0 }; }
    catch (error) { result = { stdout: error.stdout ?? '', stderr: error.stderr ?? '', code: error.code }; }
    assert(!result.stdout.includes(token) && !result.stderr.includes(token), 'session token leaked');
    assert(!result.stdout.includes('12345678') && !result.stderr.includes('12345678'), 'code leaked');
    return { calls, result, profile, token, challengeId };
  } finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
}

test('HTTP login recovers lost response with same nonce and prints only profile IDs', async () => {
  const { calls, result, profile, token, challengeId } = await fixture('recover');
  assert.equal(result.code, 0, result.stderr || result.stdout);
  assert.equal(calls.filter(call => call.path === '/v1/auth/request-code').length, 1);
  const verify = calls.filter(call => call.path === '/v1/auth/verify-code');
  assert.equal(verify.length, 2);
  assert.equal(verify[0].body, verify[1].body);
  const payload = JSON.parse(verify[0].body);
  assert.equal(payload.challengeId, challengeId);
  assert.match(payload.nonce, /^[A-Za-z0-9_-]{43}$/);
  assert.equal(calls.at(-1).authorization, `Bearer ${token}`);
  assert(result.stdout.includes(profile.accountId) && result.stdout.includes(profile.companionId));
});
test('HTTP 429 is sanitized and the mail request is not retried', async () => {
  const { calls, result } = await fixture('limited');
  assert.equal(result.code, 1);
  assert.match(result.stdout, /rate limited/);
  assert.deepEqual(calls.map(call => call.path), ['/healthz', '/v1/auth/request-code']);
});
test('HTTP redirect is refused before requesting any email', async () => {
  const { calls, result } = await fixture('redirect');
  assert.equal(result.code, 1);
  assert.deepEqual(calls.map(call => call.path), ['/healthz']);
});
