// cdp-eval.mjs - evaluate JS in a WebView2 page via CDP, print JSON result.
// Usage: node cdp-eval.mjs <url-substring> <js-expression>
import WebSocket from 'ws';

const [, , sub, expr] = process.argv;
if (!sub || !expr) { console.error('usage: node cdp-eval.mjs <url-substring> <js-expression>'); process.exit(1); }

const list = await (await fetch('http://127.0.0.1:9222/json')).json();
// prefix '=' means exact URL match instead of substring
const page = sub.startsWith('=')
  ? list.find(t => t.type === 'page' && t.url === sub.slice(1))
  : list.find(t => t.type === 'page' && t.url.includes(sub));
if (!page) { console.error('NO_TARGET ' + sub); process.exit(2); }

const ws = new WebSocket(page.webSocketDebuggerUrl);
const waitOpen = new Promise(r => ws.on('open', r));
await waitOpen;

let nextId = 1;
const pending = new Map();
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
    setTimeout(() => { if (pending.has(id)) { pending.delete(id); reject(new Error('timeout ' + method)); } }, 45000);
  });
}
ws.on('message', raw => {
  const m = JSON.parse(raw.toString());
  if (m.id && pending.has(m.id)) {
    const p = pending.get(m.id);
    pending.delete(m.id);
    if (m.error) p.reject(new Error(m.error.message));
    else p.resolve(m.result);
  }
});

try {
  await call('Runtime.enable');
  const r = await call('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) {
    console.error('EVAL_ERROR ' + JSON.stringify(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text));
    process.exit(3);
  }
  console.log(JSON.stringify(r.result.value ?? null));
  ws.close();
  process.exit(0);
} catch (e) {
  console.error('CTL_ERROR ' + e.message);
  process.exit(1);
}
