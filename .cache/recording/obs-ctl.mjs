// obs-ctl.mjs - one-shot OBS control via obs-websocket v5.
// Usage: node obs-ctl.mjs <status|start|stop|scene|pause>
import WebSocket from 'ws';
import crypto from 'node:crypto';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

const PASSWORD = 'rec-local-2026';
const URL = 'ws://127.0.0.1:4455';
const cmd = process.argv[2] ?? 'status';

const ws = new WebSocket(URL);
let nextId = 1;
const pending = new Map();
let identified = false;

function sha256b64(s) { return crypto.createHash('sha256').update(s, 'utf8').digest('base64'); }

function request(requestType, requestData) {
  return new Promise((resolve, reject) => {
    const id = String(nextId++);
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ op: 6, d: { requestType, requestId: id, ...(requestData ? { requestData } : {}) } }));
    setTimeout(() => {
      if (pending.has(id)) { pending.delete(id); reject(new Error('timeout: ' + requestType)); }
    }, 20000);
  });
}

ws.on('message', raw => {
  const msg = JSON.parse(raw.toString());
  if (msg.op === 0) {
    const d = { rpcVersion: 1, eventSubscriptions: 0 };
    if (msg.d.authentication) {
      const secret = sha256b64(PASSWORD + msg.d.authentication.salt);
      d.authentication = sha256b64(secret + msg.d.authentication.challenge);
    }
    ws.send(JSON.stringify({ op: 1, d }));
  } else if (msg.op === 2) {
    identified = true;
    main();
  } else if (msg.op === 7) {
    const p = pending.get(msg.d.requestId);
    if (p) {
      pending.delete(msg.d.requestId);
      if (msg.d.requestStatus.ok ?? msg.d.requestStatus.result) p.resolve(msg.d.responseData ?? {});
      else p.reject(new Error(`${msg.d.requestType} failed: ${msg.d.requestStatus.code} ${msg.d.requestStatus.comment ?? ''}`));
    }
  }
});

ws.on('error', e => { console.error('WS_ERROR ' + e.message); process.exit(2); });

async function main() {
  try {
    if (cmd === 'status') {
      const rec = await request('GetRecordStatus');
      const scenes = await request('GetSceneList');
      console.log(JSON.stringify({
        recording: rec.outputActive, paused: rec.outputPaused,
        currentScene: scenes.currentProgramSceneName,
        scenes: scenes.scenes.map(s => s.sceneName)
      }));
    } else if (cmd === 'scene') {
      const scenes = await request('GetSceneList');
      console.log(JSON.stringify({ currentScene: scenes.currentProgramSceneName, scenes: scenes.scenes.map(s => s.sceneName) }));
    } else if (cmd === 'start') {
      await request('StartRecord');
      console.log('STARTED');
    } else if (cmd === 'stop') {
      const r = await request('StopRecord');
      console.log('STOPPED ' + (r.outputPath ?? 'no-path'));
    } else if (cmd === 'setup') {
      // idempotent scene setup on whatever scene collection OBS loaded
      const scenes = await request('GetSceneList');
      const sceneName = scenes.currentProgramSceneName;
      const names = scenes.scenes.map(s => s.sceneName);
      const inputs = await request('GetInputList');
      const existing = inputs.inputs.map(i => i.inputName);
      let added = false;
      if (!existing.includes('Screen')) {
        await request('CreateInput', {
          sceneName,
          inputName: 'Screen',
          inputKind: 'monitor_capture',
          inputSettings: { monitor: 0, method: 0, show_cursor: true },
          sceneItemTransform: { positionX: 0, positionY: 0, width: 1920, height: 1080, boundsType: 'OBS_BOUNDS_STRETCH' }
        });
        added = true;
      }
      await request('SetVideoSettings', {
        baseWidth: 1920, baseHeight: 1080, outputWidth: 1920, outputHeight: 1080,
        fpsType: 'common', fpsCommon: '30'
      });
      // derive from script location: worktree path changes between runs, never hardcode it
      const outDir = resolve(here, 'obs-out').replace(/\\/g, '/');
      await request('SetProfileParameter', { parameterCategory: 'SimpleOutput', parameterName: 'FilePath', parameterValue: outDir });
      await request('SetProfileParameter', { parameterCategory: 'SimpleOutput', parameterName: 'RecFormat2', parameterValue: 'mkv' });
      await request('SetProfileParameter', { parameterCategory: 'SimpleOutput', parameterName: 'VBitrate', parameterValue: '12000' });
      console.log(JSON.stringify({ scene: sceneName, scenes: names, addedScreen: added }));
    } else {
      throw new Error('unknown cmd ' + cmd);
    }
    ws.close();
    process.exit(0);
  } catch (e) {
    console.error('CTL_ERROR ' + e.message);
    process.exit(1);
  }
}

setTimeout(() => { if (!identified) { console.error('WS_ERROR identify timeout (is OBS running with websocket?)'); process.exit(3); } }, 15000);
