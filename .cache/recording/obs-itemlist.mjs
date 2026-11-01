import WebSocket from 'ws';
import crypto from 'node:crypto';
const PASSWORD = 'rec-local-2026';
function sha256b64(s) { return crypto.createHash('sha256').update(s, 'utf8').digest('base64'); }
const ws = new WebSocket('ws://127.0.0.1:4455');
const pending = new Map();
let n = 1;
function req(requestType, requestData) {
  return new Promise((resolve, reject) => {
    const id = String(n++);
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ op: 6, d: { requestType, requestId: id, ...(requestData ? { requestData } : {}) } }));
    setTimeout(() => { if (pending.has(id)) { pending.delete(id); reject(new Error('timeout ' + requestType)); } }, 15000);
  });
}
ws.on('message', raw => {
  const m = JSON.parse(raw.toString());
  if (m.op === 0) {
    const d = { rpcVersion: 1, eventSubscriptions: 0 };
    if (m.d.authentication) {
      const secret = sha256b64(PASSWORD + m.d.authentication.salt);
      d.authentication = sha256b64(secret + m.d.authentication.challenge);
    }
    ws.send(JSON.stringify({ op: 1, d }));
  } else if (m.op === 7) {
    const p = pending.get(m.d.requestId);
    if (p) { pending.delete(m.d.requestId); (m.d.requestStatus.ok ?? m.d.requestStatus.result) ? p.resolve(m.d.responseData ?? {}) : p.reject(new Error(m.d.requestType + ' failed ' + m.d.requestStatus.code + ' ' + (m.d.requestStatus.comment ?? ''))); }
  }
});
ws.on('error', e => { console.error('ERR ' + e.message); process.exit(1); });
ws.on('open', () => {});
(async () => {
  await new Promise(r => ws.once('open', r));
  await new Promise(r => setTimeout(r, 300));
  // wait identified
  await new Promise(r => { const iv = setInterval(() => { if (pending.size === 0 && identified) { clearInterval(iv); r(); } }, 50); });
})().catch(() => {});
let identified = false;
ws.on('message', raw => { const m = JSON.parse(raw.toString()); if (m.op === 2) identified = true; });
setTimeout(async () => {
  const scenes = await req('GetSceneList');
  const sceneName = scenes.currentProgramSceneName;
  const items = await req('GetSceneItemList', { sceneName });
  console.log('SCENE=' + sceneName);
  for (const it of items.sceneItems) {
    console.log(JSON.stringify({ sourceName: it.sourceName, itemId: it.sceneItemId, visible: it.sceneItemVisible, transform: it.sceneItemTransform }));
  }
  const inputs = await req('GetInputList');
  for (const i of inputs.inputs) console.log('INPUT ' + i.inputName + ' kind=' + i.inputKind);
  const s = await req('GetInputSettings', { inputName: 'Screen' });
  console.log('SETTINGS ' + JSON.stringify(s));
  process.exit(0);
}, 2500);
