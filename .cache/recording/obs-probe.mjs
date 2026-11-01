import WebSocket from 'ws';
import crypto from 'node:crypto';
const PASSWORD = 'rec-local-2026';
function sha256b64(s) { return crypto.createHash('sha256').update(s, 'utf8').digest('base64'); }
const ws = new WebSocket('ws://127.0.0.1:4455');
ws.on('open', () => console.log('OPEN'));
ws.on('error', e => { console.error('ERR ' + e.message); process.exit(1); });
ws.on('close', (c, r) => { console.error('CLOSED ' + c + ' ' + r.toString()); });
ws.on('message', raw => {
  const m = JSON.parse(raw.toString());
  if (m.op === 0) {
    console.log('HELLO ' + JSON.stringify(m.d));
    const d = { rpcVersion: 1, eventSubscriptions: 0 };
    if (m.d.authentication) {
      const secret = sha256b64(PASSWORD + m.d.authentication.salt);
      d.authentication = sha256b64(secret + m.d.authentication.challenge);
    }
    ws.send(JSON.stringify({ op: 1, d }));
  } else if (m.op === 2) {
    console.log('IDENTIFIED');
    ws.send(JSON.stringify({ op: 6, d: { requestType: 'GetRecordStatus', requestId: 'r1' } }));
  } else if (m.op === 7) {
    console.log('RESP ' + JSON.stringify(m.d));
    process.exit(0);
  } else {
    console.log('OP' + m.op + ' ' + JSON.stringify(m.d).slice(0, 200));
  }
});
setTimeout(() => { console.error('TIMEOUT'); process.exit(3); }, 15000);
