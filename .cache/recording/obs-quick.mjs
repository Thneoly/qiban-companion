import WebSocket from 'ws';
import crypto from 'node:crypto';
const PASSWORD = 'rec-local-2026';
function sha256b64(s) { return crypto.createHash('sha256').update(s, 'utf8').digest('base64'); }
const ws = new WebSocket('ws://127.0.0.1:4455');
const pending = new Map();
let n = 1, identified = false;
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
  } else if (m.op === 2) { identified = true; }
  else if (m.op === 7) {
    const p = pending.get(m.d.requestId);
    if (p) { pending.delete(m.d.requestId); (m.d.requestStatus.ok ?? m.d.requestStatus.result) ? p.resolve(m.d.responseData ?? {}) : p.reject(new Error(m.d.requestType + ' failed ' + m.d.requestStatus.code + ' ' + (m.d.requestStatus.comment ?? ''))); }
  }
});
ws.on('error', e => { console.error('ERR ' + e.message); process.exit(1); });
const MON = Number(process.argv[2] ?? '1');
const METHOD = Number(process.argv[3] ?? '2');
setTimeout(async () => {
  try {
    await req('SetInputSettings', { inputName: 'Screen', inputSettings: { method: METHOD, monitor: MON, show_cursor: true } });
    console.log(`SET monitor=${MON} method=${METHOD} ok`);
    await new Promise(r => setTimeout(r, 1500));
    const scenes = await req('GetSceneList');
    const items = await req('GetSceneItemList', { sceneName: scenes.currentProgramSceneName });
    const t = items.sceneItems.find(i => i.sourceName === 'Screen')?.sceneItemTransform;
    console.log('SIZE ' + (t?.sourceWidth ?? '?') + 'x' + (t?.sourceHeight ?? '?'));
    process.exit(0);
  } catch (e) { console.error('FAIL ' + e.message); process.exit(1); }
}, 2500);
