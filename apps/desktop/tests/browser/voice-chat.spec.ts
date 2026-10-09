import { test, expect, type Page } from '@playwright/test';
import { pcmWave } from '../../src/features/voice/audio';

const wave = Buffer.from(pcmWave(new Float32Array(1600), 16000));

/** Pet-surface chat mock plus the three voice-turn commands, a fake
 * getUserMedia/MediaRecorder pair that yields the fixture WAV, and a
 * scripted HTMLMediaElement.play. Per-test knobs live on window. */
async function mockVoiceChat(page: Page) {
  await page.addInitScript((wavArray: number[]) => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    let next = 0; const callbacks = new Map<number, (value: unknown) => void>();
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    w.wavBytes = wavArray;
    w.savedVoice = { voiceBaseUrl: 'https://voice.test/v1', useVoiceKey: false, asrModel: 'asr-x', ttsModel: 'tts-x', voice: 'v1' };
    w.replyDeltas = ['你好呀。', '今天天气不错。'];
    w.transcribeCalls = []; w.speakCalls = []; w.turnCancels = []; w.chatCancels = [];
    w.generateRequests = []; w.playLog = []; w.gumCalls = 0; w.chatHistory = [];
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (cb: (value: unknown) => void) => { callbacks.set(++next, cb); return next; },
      unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') return ++next;
        if (cmd === 'chat_context_preview') return {
          scope: { baseUrl: 'https://fixture.test', model: 'glm-test-fixture' }, contextEpoch: 1,
          policy: { enabled: false, revision: 0, selectedIds: [] }, items: [], bodyChars: 0, contextChars: 0,
          personal: { status: 'offline', policy: { enabled: false, revision: 0, selectedIds: [] }, items: [], inactiveSelectedIds: [], bodyChars: 0, contextChars: 0 },
        };
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'personal_memory_overview') return { online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' };
        if (cmd === 'chat_config') return { configured: true, model: 'glm-test-fixture', maxOutputTokens: 1024 };
        if (cmd === 'chat_history') return w.chatHistory;
        if (cmd === 'chat_clear') { w.chatHistory = []; return; }
        if (cmd === 'chat_cancel') { w.chatCancels.push(args.requestId); w.onChatCancel?.(); return; }
        if (cmd === 'chat_generate') {
          w.generateRequests.push(structuredClone(args.request));
          const id = args.request.requestId;
          const deltas = w.replyDeltas ?? [];
          if (w.hangSilent) return new Promise((_, reject) => { w.onChatCancel = () => reject('fixture cancelled'); });
          for (const text of deltas) {
            args.onDelta.onmessage({ requestId: id, text, memoryUsage: null });
            if (w.deltaGap) await new Promise(resolve => setTimeout(resolve, w.deltaGap));
          }
          if (w.hangGenerate) return new Promise((_, reject) => { w.onChatCancel = () => reject('fixture cancelled'); });
          w.chatHistory.push({ user: args.request.prompt, assistant: deltas.join('') });
          return { requestId: id, elapsedMs: 3, historySaved: true, usage: { total_tokens: 2 },
            memoryUsage: { scope: args.request.expectedScope, contextEpoch: 1, memories: [], bodyChars: 0, contextChars: 0,
              personal: { status: 'offline', memories: [], bodyChars: 0, contextChars: 0 } } };
        }
        if (cmd === 'voice_settings_get') {
          const base = w.savedVoice || { voiceBaseUrl: '', useVoiceKey: true, asrModel: '', ttsModel: '', voice: '' };
          return { ...base, hasVoiceKey: Boolean(w.savedVoice) };
        }
        if (cmd === 'voice_transcribe') {
          const { wav, ...rest } = args.request;
          w.transcribeCalls.push({ ...rest, wavLength: wav.length });
          return { requestId: args.request.requestId, transcript: w.transcript ?? '今天过得怎么样', recognitionMs: 9 };
        }
        if (cmd === 'voice_speak') {
          const request = { requestId: args.request.requestId, turnId: args.request.turnId, expectedBaseUrl: args.request.expectedBaseUrl, seq: args.request.seq, text: args.request.text, first: args.request.first };
          w.speakCalls.push(request);
          args.onProgress?.onmessage({ requestId: request.requestId, trimmed: request.first ? null : (w.trimFlag ?? true) });
          if ((w.speakFailSeqs ?? []).includes(request.seq)) return Promise.reject('fixture synthesis failed');
          if (w.speakDelays?.[request.seq] === 'hang') return new Promise(resolve => { (w.hangSpeakResolvers ??= []).push(resolve); });
          // The wav's first data byte (index 44) carries the seq so tests can
          // map a played blob URL back to its sentence; speakRaw toggles the
          // real-machine return shape (ArrayBuffer) vs the JSON fallback.
          const bytes = new Uint8Array(w.wavBytes.length);
          bytes.set(w.wavBytes);
          bytes[44] = request.seq % 256;
          return Promise.resolve(w.speakRaw === 'arraybuffer' ? bytes.buffer : bytes);
        }
        if (cmd === 'voice_turn_cancel') { w.turnCancels.push(args.turnId); return; }
        return 1;
      },
    } });
    const context = new AudioContext();
    navigator.mediaDevices.getUserMedia = () => {
      w.gumCalls++;
      const stream = context.createMediaStreamDestination().stream;
      w.lastStream = stream;
      return Promise.resolve(stream);
    };
    window.MediaRecorder = class {
      stream: MediaStream; state = 'inactive'; mimeType = 'audio/wav';
      ondataavailable: ((event: { data: Blob }) => void) | null = null;
      onstop: (() => void) | null = null;
      constructor(stream: MediaStream) { this.stream = stream; }
      start() { this.state = 'recording'; }
      stop() {
        this.state = 'inactive';
        this.ondataavailable?.({ data: new Blob([new Uint8Array(w.wavBytes)], { type: 'audio/wav' }) });
        this.stream.getTracks().forEach(track => track.stop());
        this.onstop?.();
      }
    } as unknown as typeof MediaRecorder;
    w.releaseSpeak = () => { const list = w.hangSpeakResolvers ?? []; w.hangSpeakResolvers = []; for (const resolve of list) resolve(new Uint8Array(w.wavBytes)); };
    w.releasePlays = () => { const list = w.hangPlayResolvers ?? []; w.hangPlayResolvers = []; for (const release of list) release(); };
    // Map every created blob URL to its bytes so play order can be tied to
    // sentence seqs instead of opaque URLs.
    w.urlBytes = new Map();
    const realCreateObjectURL = URL.createObjectURL.bind(URL);
    URL.createObjectURL = (blob: Blob) => {
      const url = realCreateObjectURL(blob);
      void blob.arrayBuffer().then(value => { w.urlBytes.set(url, new Uint8Array(value)); });
      return url;
    };
    HTMLMediaElement.prototype.play = function () {
      const el = this;
      w.playLog.push(el.src);
      if ((w.hangPlayCount ?? 0) > 0) {
        w.hangPlayCount--;
        return new Promise(resolve => { (w.hangPlayResolvers ??= []).push(() => { resolve(); setTimeout(() => el.dispatchEvent(new Event('ended')), 0); }); });
      }
      setTimeout(() => el.dispatchEvent(new Event('ended')), 30);
      return Promise.resolve();
    };
  }, Array.from(wave));
}

const mic = (page: Page) => page.locator('button.pet-voice');
async function openChat(page: Page) {
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByRole('button', { name: '聊一聊', exact: true }).click();
}

test('unconfigured or unreadable voice settings keep the mic disabled', async ({ page }) => {
  await mockVoiceChat(page);
  await page.addInitScript(() => { (window as any).savedVoice = null; });
  await page.goto('/');
  await openChat(page);
  await expect(mic(page)).toBeDisabled();
  await expect(mic(page)).toHaveText('语音未配置');
  await expect(mic(page)).toHaveAttribute('title', /语音未配置或缺密钥/);
  // A failing settings read fails closed too: reopen with a rejecting invoke.
  await page.evaluate(() => {
    const inner = (window as any).__TAURI_INTERNALS__;
    const real = inner.invoke;
    inner.invoke = (cmd: string, args: any) => cmd === 'voice_settings_get' ? Promise.reject('fixture read failed') : real(cmd, args);
  });
  await page.getByRole('button', { name: '收起气泡', exact: true }).click();
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByRole('button', { name: '聊一聊', exact: true }).click();
  await expect(mic(page)).toBeDisabled();
  await expect(mic(page)).toHaveAttribute('title', /语音配置读取失败/);
});

test('recording uploads once, blur cancels an open recording, and 10 seconds auto-stop', async ({ page }) => {
  await page.clock.install();
  await mockVoiceChat(page);
  await page.goto('/');
  await openChat(page);
  await expect(mic(page)).toHaveText('语音说话');
  // One manual stop → exactly one upload carrying the whole WAV payload.
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  expect(await page.evaluate(() => (window as any).gumCalls)).toBe(1);
  await mic(page).click();
  await expect.poll(() => page.evaluate(() => (window as any).transcribeCalls.length)).toBe(1);
  const first = await page.evaluate(() => (window as any).transcribeCalls[0]);
  expect(first.wavLength).toBe(wave.length);
  expect(first.requestId).toMatch(/-t0$/);
  // The turn settles on its own: chat done, both clips played, mic idle again.
  await expect(mic(page)).toHaveText('语音说话');
  // Blur while recording collapses the pet dialog (Pet.tsx blur) and cancels
  // the take: no upload, tracks released, a reopened bubble starts fresh.
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await expect(page.getByRole('button', { name: '和栖栖互动' })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).lastStream.getTracks()[0].readyState)).toBe('ended');
  expect(await page.evaluate(() => (window as any).transcribeCalls.length)).toBe(1);
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByRole('button', { name: '聊一聊', exact: true }).click();
  await expect(mic(page)).toHaveText('语音说话');
  // The 10-second cap stops the recorder by itself.
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await page.clock.fastForward(10000);
  await expect.poll(() => page.evaluate(() => (window as any).transcribeCalls.length)).toBe(2);
});

test('the transcript auto-sends through the real chat chain with the expected scope', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(page.getByText('我听到：今天过得怎么样')).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).generateRequests.length)).toBe(1);
  const request = await page.evaluate(() => (window as any).generateRequests[0]);
  expect(request.prompt).toBe('今天过得怎么样');
  expect(request.expectedScope).toEqual({ baseUrl: 'https://fixture.test', model: 'glm-test-fixture' });
  expect(request.expectedContextEpoch).toBe(1);
  expect(request.expectedPersonal).toBeNull();
  const transcribe = await page.evaluate(() => (window as any).transcribeCalls[0]);
  expect(transcribe.expectedBaseUrl).toBe('https://voice.test/v1');
  const speak = await page.evaluate(() => (window as any).speakCalls[0]);
  expect(speak.turnId).toBe(transcribe.turnId);
  await expect(page.getByText('最近 1 轮', { exact: true })).toBeVisible();
  await expect(mic(page)).toHaveText('语音说话');
});

test('clips play in sentence order with one-ahead prefetch', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。', '第二句。', '第三句。'];
    w.hangPlayCount = 1;
    w.speakRaw = 'arraybuffer'; // exercise the real-machine return branch
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  // The first clip is mid-play and the second is already synthesizing.
  await expect.poll(() => page.evaluate(() => (window as any).playLog.length === 1 && (window as any).speakCalls.length === 2)).toBe(true);
  await page.evaluate(() => (window as any).releasePlays());
  await expect.poll(() => page.evaluate(() => (window as any).playLog.length)).toBe(3);
  const calls = await page.evaluate(() => (window as any).speakCalls.map((c: any) => ({ seq: c.seq, first: c.first })));
  expect(calls).toEqual([{ seq: 0, first: true }, { seq: 1, first: false }, { seq: 2, first: false }]);
  // The played blob URLs map back to sentence order, not just count.
  const played = await page.evaluate(() => (window as any).playLog.map((src: string) => (window as any).urlBytes.get(src)?.[44]));
  expect(played).toEqual([0, 1, 2]);
  await expect(mic(page)).toHaveText('语音说话');
});

test('interrupting between sentences leaves no second drive loop behind', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  // Turn A: one clip plays fast, then the stream hangs with the queue empty -
  // the drive loop parks waiting for a next sentence that never comes.
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。'];
    w.hangGenerate = true;
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('停止朗读');
  await page.waitForTimeout(120); // let the clip settle so the loop parks
  await mic(page).click(); // 停止朗读 while parked, not mid-fetch
  await expect(mic(page)).toHaveText('语音说话');
  // Turn C: two sentences drip in while the first play hangs. A stale loop
  // from turn A would clobber the driving flag and start a second concurrent
  // play; only one clip may ever sound at a time. Turn A's own play is the
  // baseline, so every count below is plays of turn C only.
  const baseline = await page.evaluate(() => (window as any).playLog.length);
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['C一句。', 'C二句。'];
    w.hangGenerate = false;
    w.hangPlayCount = 1;
    w.deltaGap = 60;
  });
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect.poll(() => page.evaluate((from: number) => (window as any).playLog.length - from, baseline)).toBe(1);
  await page.waitForTimeout(250);
  expect(await page.evaluate((from: number) => (window as any).playLog.length - from, baseline)).toBe(1);
  const order = await page.evaluate((from: number) => (window as any).playLog.slice(from).map((src: string) => (window as any).urlBytes.get(src)?.[44]), baseline);
  expect(order).toEqual([0]);
  await page.evaluate(() => (window as any).releasePlays());
  await expect.poll(() => page.evaluate((from: number) => (window as any).playLog.length - from, baseline)).toBe(2);
  const settled = await page.evaluate((from: number) => (window as any).playLog.slice(from).map((src: string) => (window as any).urlBytes.get(src)?.[44]), baseline);
  expect(settled).toEqual([0, 1]);
  await expect(mic(page)).toHaveText('语音说话');
});

test('interrupting speech drops late clips, cancels both legs, and allows a fresh turn', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。'];
    w.hangGenerate = true;
    w.speakDelays = { 0: 'hang' };
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('停止朗读');
  await expect.poll(() => page.evaluate(() => (window as any).speakCalls.length)).toBe(1);
  await mic(page).click();
  await expect(mic(page)).toHaveText('语音说话');
  const legs = await page.evaluate(() => ({
    turnCancels: (window as any).turnCancels,
    chatCancels: (window as any).chatCancels.length,
    turn: (window as any).transcribeCalls[0].turnId,
  }));
  expect(legs.turnCancels).toEqual([legs.turn]);
  expect(legs.chatCancels).toBe(1);
  // The clip that was still synthesizing lands late: it must never play.
  await page.evaluate(() => (window as any).releaseSpeak());
  await page.waitForTimeout(60);
  expect(await page.evaluate(() => (window as any).playLog.length)).toBe(0);
  // A fresh voice turn on the same surface works end to end.
  await page.evaluate(() => {
    const w = window as any;
    w.hangGenerate = false;
    w.speakDelays = {};
    w.replyDeltas = ['新的回答。'];
  });
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect.poll(() => page.evaluate(() => (window as any).playLog.length)).toBe(1);
  await expect(mic(page)).toHaveText('语音说话');
});

test('a speaking turn survives pet-window blur and collapse resumes once idle', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  // One clip hangs mid-synthesis so the turn is solidly in speaking state.
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。'];
    w.hangGenerate = true;
    w.speakDelays = { 0: 'hang' };
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('停止朗读');
  // Blur while speaking: the bubble stays, the turn keeps running — no cancel
  // of either leg, the stop control remains reachable.
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await expect(mic(page)).toBeVisible();
  await expect(mic(page)).toHaveText('停止朗读');
  expect(await page.evaluate(() => ({ turn: (window as any).turnCancels.length, chat: (window as any).chatCancels.length }))).toEqual({ turn: 0, chat: 0 });
  // Settle the turn (chat leg resolves, hung synthesis lands and plays), then
  // a fresh blur collapses the dialog again — the exemption is per-turn only.
  await page.evaluate(() => (window as any).onChatCancel?.());
  await page.evaluate(() => (window as any).releaseSpeak());
  await expect(mic(page)).toHaveText('语音说话');
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await expect(page.getByLabel('栖栖的交互气泡')).toBeHidden();
  await expect(page.getByRole('button', { name: '和栖栖互动' })).toHaveAttribute('aria-expanded', 'false');
});

test('the first clip keeps its tone, later clips report trimming, and the untrimmed fallback engages', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。第二句。第三句。'];
    w.trimFlag = false;
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('语音说话');
  const turn1 = await page.evaluate(() => (window as any).speakCalls.map((c: any) => ({ seq: c.seq, first: c.first })));
  expect(turn1).toEqual([{ seq: 0, first: true }, { seq: 1, first: false }, { seq: 2, first: false }]);
  // Both later clips came back untrimmed → the next turn speaks whole text once.
  await page.evaluate(() => { (window as any).replyDeltas = ['这一整段回复没有任何句号却要一次说完']; });
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('语音说话');
  const calls = await page.evaluate(() => (window as any).speakCalls);
  expect(calls.length).toBe(4);
  expect(calls[3].first).toBe(true);
  expect(calls[3].text).toBe('这一整段回复没有任何句号却要一次说完');
});

test('a failed clip is skipped and never poisons the next turn', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。', '第二句。', '第三句。'];
    w.speakFailSeqs = [1];
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('语音说话');
  const turn1 = await page.evaluate(() => ({
    speaks: (window as any).speakCalls.map((c: any) => c.seq),
    plays: (window as any).playLog.length,
  }));
  expect(turn1.speaks).toEqual([0, 1, 2]);
  expect(turn1.plays).toBe(2);
  // The next turn synthesizes and plays normally.
  await page.evaluate(() => {
    const w = window as any;
    w.speakFailSeqs = [];
    w.replyDeltas = ['新轮次。'];
  });
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(mic(page)).toHaveText('语音说话');
  await expect.poll(() => page.evaluate(() => (window as any).playLog.length)).toBe(3);
});

test('the transcript subtitle appears before any reply text streams', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  await page.evaluate(() => { (window as any).hangSilent = true; });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect(page.getByText('我听到：今天过得怎么样')).toBeVisible();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('想聊点什么？');
  await expect(page.locator('.chat-speaking')).toHaveText('栖栖正在说');
  // Settle the hanging generation so the turn ends cleanly.
  await page.evaluate(() => (window as any).onChatCancel?.());
  await expect(mic(page)).toHaveText('语音说话');
});

test('the svg mouth follows playback amplitude in coarse steps', async ({ page }) => {
  await mockVoiceChat(page);
  await page.goto('/');
  // Headless autoplay policy may suspend the audio context and starve the
  // analyser; the mouth chain itself is what needs testing, so the context
  // gate is forced open and the analyser data is synthesised on demand.
  await page.evaluate(() => {
    const w = window as any;
    const real = window.AudioContext;
    class Forced extends real {
      get state() { return 'running'; }
      async resume() { try { await super.resume(); } catch { /* forced */ } }
    }
    window.AudioContext = Forced;
    const read = AnalyserNode.prototype.getByteTimeDomainData;
    AnalyserNode.prototype.getByteTimeDomainData = function (array: Uint8Array) {
      if (w.mouthDrive) { for (let i = 0; i < array.length; i++) array[i] = i % 2 ? 230 : 26; return; }
      return read.call(this, array);
    };
  });
  await page.evaluate(() => {
    const w = window as any;
    w.replyDeltas = ['第一句。'];
    w.hangPlayCount = 1;
  });
  await openChat(page);
  await mic(page).click();
  await expect(mic(page)).toHaveText('结束录音');
  await mic(page).click();
  await expect.poll(() => page.evaluate(() => (window as any).playLog.length)).toBe(1);
  const mouth = page.locator('.pet-character svg .avatar-mouth');
  await expect(mouth).toHaveAttribute('data-level', '0');
  // A loud square wave opens the mouth fully (RMS≈0.8, smoothed over 150ms);
  // silence closes it again and restores the resting mouth line.
  await page.evaluate(() => { (window as any).mouthDrive = true; });
  await expect.poll(() => mouth.getAttribute('data-level')).toBe('4');
  await page.evaluate(() => { (window as any).mouthDrive = false; });
  await expect.poll(() => mouth.getAttribute('data-level')).toBe('0');
  await expect(page.locator('.pet-character svg .resting-mouth')).toBeVisible();
  await page.evaluate(() => (window as any).releasePlays());
  await expect(mic(page)).toHaveText('语音说话');
});
