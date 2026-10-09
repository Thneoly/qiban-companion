/**
 * T11 voice turn skin: record → transcribe → send through the real chat chain
 * (history, memory, epoch guards all inherited) → split the stream into
 * sentences → speak them in order with one-ahead prefetch. Speech is a
 * presentation layer only: history stores text, never audio.
 */
import { useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { decodeVoiceSettings, decodeVoiceSpeakProgress, decodeVoiceTranscribe } from '@companion/contracts';
import { nativeDesktop } from '../../lib/surface';
import { normalizeRecording } from './audio';
import { feedSentence, flushSentences } from './sentences';

export type MicState = 'off' | 'idle' | 'recording' | 'transcribing' | 'speaking';
/** The chat leg reports stream deltas and settlement through this context. */
export interface VoiceTurnContext {
  onDelta(text: string): void;
  onDone(): void;
}
/** voice_speak returns raw WAV bytes via ipc::Response (ArrayBuffer on the JS
 * side); the array form is the documented JSON fallback seam. */
function toWav(value: unknown): Uint8Array<ArrayBuffer> {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  // Copy any foreign-backed view so the Blob part is always ArrayBuffer-backed.
  if (value instanceof Uint8Array) return new Uint8Array(value);
  if (Array.isArray(value)) return Uint8Array.from(value as number[]);
  throw Error('语音音频格式不兼容');
}
const MAX_CLIPS = 64; // Rust rejects seq > 64; longer replies stop early
const SINGLE_SHOT_CHARS = 500;

export function useVoiceTurn(options: {
  onTranscript(text: string, turn: VoiceTurnContext): boolean | Promise<boolean>;
  onStopChat(): void;
  /** Amplitude (0..1) of the clip currently playing, smoothed and gated. The
   * renderers read it every frame to drive the mouth; playback itself never
   * depends on it. Optional so surfaces without a mouth can adopt the turn. */
  mouth?: { current: number };
}) {
  const [micState, setMicState] = useState<MicState>('off');
  const [guidance, setGuidance] = useState('正在检查语音配置…');
  const [notice, setNotice] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<string | null>(null);
  const micRef = useRef(micState); micRef.current = micState;
  const onTranscript = useRef(options.onTranscript); onTranscript.current = options.onTranscript;
  const onStopChat = useRef(options.onStopChat); onStopChat.current = options.onStopChat;
  const mouthTarget = useRef(options.mouth); mouthTarget.current = options.mouth;

  const generation = useRef(0);
  const baseUrl = useRef('');
  const singleShot = useRef(false);
  const turnId = useRef<string | null>(null);
  const buffer = useRef('');
  const fullText = useRef('');
  const queue = useRef<{ seq: number; text: string }[]>([]);
  const cursor = useRef(0);
  const done = useRef(false);
  const fetches = useRef(new Map<number, Promise<Uint8Array<ArrayBuffer>>>());
  const stats = useRef({ later: 0, untrimmed: 0 });
  const driving = useRef(false);
  const wake = useRef<(() => void) | null>(null);
  const stopPlayback = useRef<(() => void) | null>(null);
  const recorder = useRef<MediaRecorder | null>(null);
  const stream = useRef<MediaStream | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const url = useRef<string | null>(null);
  const recording = useRef(false);
  const audioCtx = useRef<AudioContext | null>(null);
  const analyser = useRef<AnalyserNode | null>(null);
  const mouthSource = useRef<MediaElementAudioSourceNode | null>(null);
  const mouthFrame = useRef(0);

  function release() {
    stopMouth();
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    if (recorder.current?.state === 'recording') recorder.current.stop();
    stream.current?.getTracks().forEach(track => track.stop()); stream.current = null;
    if (audio.current) {
      audio.current.pause(); audio.current.onended = null; audio.current.onerror = null;
      audio.current.removeAttribute('src'); audio.current.load(); audio.current = null;
    }
    stopPlayback.current?.(); stopPlayback.current = null;
    if (url.current) { URL.revokeObjectURL(url.current); url.current = null; }
  }
  /** Stop everything of the current turn: epoch bump drops late clips, the
   * audio element settles, and the backend cancels in-flight requests. The
   * drive loop is also woken: a loop parked waiting for the next sentence
   * would otherwise survive into a later turn and clobber the new loop's
   * driving flag when it finally exits. */
  function interrupt() {
    generation.current++;
    const turn = turnId.current; turnId.current = null;
    buffer.current = ''; fullText.current = '';
    queue.current = []; cursor.current = 0; done.current = false;
    fetches.current.clear(); driving.current = false;
    wake.current?.(); wake.current = null;
    release();
    recording.current = false;
    if (turn) void invoke('voice_turn_cancel', { turnId: turn }).catch(() => {});
  }

  function enqueue(sentences: string[]) {
    for (const text of sentences) {
      if (queue.current.length >= MAX_CLIPS) break;
      queue.current.push({ seq: queue.current.length, text });
    }
    // A sentence arriving mid-playback must start synthesizing immediately:
    // the drive loop only looks ahead when it picks its next clip.
    const turn = turnId.current;
    const upcoming = turn ? queue.current[cursor.current] : undefined;
    if (turn && upcoming) void clipBytes(turn, upcoming).catch(() => {});
    wake.current?.(); wake.current = null;
    driveIfIdle();
  }
  function clipBytes(turn: string, clip: { seq: number; text: string }): Promise<Uint8Array<ArrayBuffer>> {
    const existing = fetches.current.get(clip.seq);
    if (existing) return existing;
    const requestId = `${turn}-s${clip.seq}`;
    const first = clip.seq === 0;
    const epoch = generation.current;
    const progress = new Channel<unknown>();
    progress.onmessage = value => {
      if (generation.current !== epoch) return;
      try {
        const flag = decodeVoiceSpeakProgress(value);
        if (flag.requestId === requestId && flag.trimmed === false) stats.current.untrimmed++;
      } catch { /* progress is advisory; the audio itself is authoritative */ }
    };
    if (!first) stats.current.later++;
    const promise = invoke('voice_speak', {
      request: { requestId, turnId: turn, expectedBaseUrl: baseUrl.current, seq: clip.seq, text: clip.text, first },
      onProgress: progress,
    }).then(toWav);
    fetches.current.set(clip.seq, promise);
    return promise;
  }
  /** Route the playing element through an analyser so the mouth follows the
   * real audio amplitude. Routing happens only while the context actually
   * runs — a suspended context would swallow the element's sound, which must
   * never happen; in that case the clip plays normally and the mouth rests. */
  function startMouth(player: HTMLAudioElement, alive: () => boolean) {
    const target = mouthTarget.current;
    if (!target) return;
    const epoch = generation.current;
    void (async () => {
      try {
        let ctx = audioCtx.current;
        if (!ctx) ctx = audioCtx.current = new AudioContext();
        if (ctx.state !== 'running') await ctx.resume();
        if (ctx.state !== 'running' || generation.current !== epoch || !alive()) return;
        let node = analyser.current;
        if (!node) {
          node = ctx.createAnalyser();
          node.fftSize = 256;
          node.connect(ctx.destination);
          analyser.current = node;
        }
        const source = ctx.createMediaElementSource(player);
        source.connect(node);
        mouthSource.current?.disconnect();
        mouthSource.current = source;
        const samples = new Uint8Array(node.fftSize);
        let smoothed = 0;
        let previous = performance.now();
        const tick = () => {
          if (!alive() || mouthSource.current !== source) return;
          node!.getByteTimeDomainData(samples);
          let sum = 0;
          for (let index = 0; index < samples.length; index++) {
            const value = (samples[index]! - 128) / 128;
            sum += value * value;
          }
          const rms = Math.sqrt(sum / samples.length);
          const level = rms < 0.02 ? 0 : Math.min(1, rms * 4);
          const now = performance.now();
          smoothed += (level - smoothed) * (1 - Math.exp(-(now - previous) / 150));
          previous = now;
          target.current = smoothed;
          mouthFrame.current = requestAnimationFrame(tick);
        };
        mouthFrame.current = requestAnimationFrame(tick);
      } catch { /* the mouth is cosmetic; playback itself must not fail */ }
    })();
  }
  function stopMouth() {
    cancelAnimationFrame(mouthFrame.current);
    mouthSource.current?.disconnect();
    mouthSource.current = null;
    if (mouthTarget.current) mouthTarget.current.current = 0;
  }
  function playClip(wav: Uint8Array<ArrayBuffer>): Promise<boolean> {
    return new Promise(resolve => {
      const blobUrl = URL.createObjectURL(new Blob([wav], { type: 'audio/wav' }));
      url.current = blobUrl;
      const player = new Audio(blobUrl); audio.current = player;
      let settled = false;
      const settle = (failed: boolean) => {
        settled = true;
        stopMouth();
        player.onended = null; player.onerror = null;
        stopPlayback.current = null;
        if (url.current === blobUrl) { URL.revokeObjectURL(blobUrl); url.current = null; }
        audio.current = null;
        resolve(failed);
      };
      player.onended = () => settle(false);
      player.onerror = () => settle(true);
      stopPlayback.current = () => settle(false);
      void player.play().catch(() => settle(true));
      startMouth(player, () => !settled);
    });
  }
  async function drive(epoch: number, turn: string) {
    while (generation.current === epoch) {
      const clip = queue.current[cursor.current];
      if (!clip) {
        if (done.current) break;
        await new Promise<void>(resolve => { wake.current = resolve; });
        continue;
      }
      cursor.current++;
      try {
        const wav = await clipBytes(turn, clip);
        if (generation.current !== epoch) break;
        // Prefetch exactly one clip ahead: the next sentence's synthesis
        // hides behind this clip's playback.
        const ahead = queue.current[cursor.current];
        if (ahead) void clipBytes(turn, ahead).catch(() => {});
        await playClip(wav);
      } catch { /* a failed clip is skipped; the next one must still speak */ }
      if (generation.current !== epoch) break;
    }
    if (generation.current !== epoch) { driving.current = false; return; }
    // Natural end of the turn: decide whether the next turn needs the
    // single-shot fallback (trim missed on at least half of the later clips).
    // Only evidence moves the flag — later===0 (one-sentence turns, trim never
    // attempted) keeps the previous decision, and a clean later clip proves
    // the provider layout is trimmable again.
    if (stats.current.later >= 2 && stats.current.untrimmed * 2 >= stats.current.later) singleShot.current = true;
    else if (stats.current.later >= 1 && stats.current.untrimmed === 0) singleShot.current = false;
    driving.current = false;
    turnId.current = null;
    if (micRef.current === 'speaking') setMicState('idle');
  }
  function driveIfIdle() {
    const turn = turnId.current;
    if (!turn || driving.current) return;
    driving.current = true;
    void drive(generation.current, turn);
  }

  function feed(text: string) {
    if (!turnId.current) return;
    fullText.current += text;
    if (singleShot.current) return; // one whole-text synthesis at onDone
    const step = feedSentence(buffer.current, text);
    buffer.current = step.buffer;
    enqueue(step.sentences);
  }
  function finish() {
    if (!turnId.current) return;
    done.current = true;
    if (singleShot.current) {
      const text = fullText.current.trim();
      if (text) {
        const chunks: string[] = [];
        const chars = Array.from(text);
        for (let index = 0; index < chars.length; index += SINGLE_SHOT_CHARS) {
          chunks.push(chars.slice(index, index + SINGLE_SHOT_CHARS).join(''));
        }
        enqueue(chunks);
      }
    } else {
      enqueue(flushSentences(buffer.current));
      buffer.current = '';
    }
    driveIfIdle();
  }

  async function upload(blob: Blob, id: number) {
    try {
      const wav = await normalizeRecording(blob);
      if (generation.current !== id) return;
      const turn = crypto.randomUUID();
      turnId.current = turn;
      buffer.current = ''; fullText.current = ''; queue.current = []; cursor.current = 0; done.current = false;
      fetches.current.clear(); stats.current = { later: 0, untrimmed: 0 };
      const data = decodeVoiceTranscribe(await invoke('voice_transcribe', {
        request: { requestId: `${turn}-t0`, turnId: turn, expectedBaseUrl: baseUrl.current, wav: Array.from(wav) },
      }));
      if (generation.current !== id || turnId.current !== turn) return;
      setTranscript(data.transcript); setNotice(null);
      setMicState('speaking');
      // The chat leg's callbacks are bound to this turn: a late settle of a
      // previous turn's chat request must not mark this turn done.
      const accepted = await onTranscript.current(data.transcript, {
        onDelta: text => { if (turnId.current === turn) feed(text); },
        onDone: () => { if (turnId.current === turn) finish(); },
      });
      if (!accepted && generation.current === id && turnId.current === turn) {
        turnId.current = null;
        setMicState('idle');
        setNotice('文字回复正在进行，请等它结束后再说话。');
      }
    } catch (error) {
      if (generation.current !== id) return;
      turnId.current = null;
      setMicState('idle');
      setNotice(typeof error === 'string' ? error : error instanceof Error ? error.message : '语音识别失败，请重试。');
    }
  }

  async function startRecording() {
    if (micRef.current === 'off') return;
    // Recording and speaking are mutually exclusive: stop the old turn first.
    if (micRef.current !== 'idle') { onStopChat.current(); interrupt(); setMicState('idle'); }
    const id = ++generation.current;
    release(); setTranscript(null); setNotice(null);
    recording.current = true;
    try {
      const acquired = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true }, video: false,
      });
      if (generation.current !== id) { acquired.getTracks().forEach(track => track.stop()); return; }
      stream.current = acquired;
      const capture = new MediaRecorder(acquired); recorder.current = capture;
      const parts: Blob[] = [];
      capture.ondataavailable = event => { if (event.data.size) parts.push(event.data); };
      capture.onstop = () => {
        acquired.getTracks().forEach(track => track.stop());
        if (timer.current) { clearTimeout(timer.current); timer.current = null; }
        if (generation.current !== id) return;
        recording.current = false;
        setMicState('transcribing');
        void upload(new Blob(parts, { type: capture.mimeType }), id);
      };
      capture.onerror = () => {
        if (generation.current === id) { interrupt(); setMicState('idle'); setNotice('录音设备发生错误，请重试。'); }
      };
      capture.start(); setMicState('recording');
      timer.current = setTimeout(() => { if (capture.state === 'recording') capture.stop(); }, 10000);
    } catch {
      if (generation.current === id) {
        interrupt();
        setMicState('idle'); setNotice('无法使用麦克风：请检查系统权限或设备后重试。');
      }
    }
  }
  function finishRecording() { if (recorder.current?.state === 'recording') recorder.current.stop(); }
  function stopSpeaking() {
    onStopChat.current();
    interrupt();
    setMicState('idle');
    setNotice('已停止朗读；在途调用可能计费。');
  }

  useEffect(() => {
    let disposed = false;
    if (!nativeDesktop) { setGuidance('浏览器预览不调用语音；请运行桌面版。'); return; }
    void invoke('voice_settings_get').then(value => {
      if (disposed) return;
      const saved = decodeVoiceSettings(value);
      baseUrl.current = saved.voiceBaseUrl;
      if (!saved.voiceBaseUrl || (saved.useVoiceKey && !saved.hasVoiceKey)) {
        setMicState('off');
        setGuidance('语音未配置或缺密钥：请在任务面板的“语音实验”里保存语音配置并设置密钥，再重开气泡。');
      } else setMicState('idle');
    }).catch(() => {
      if (!disposed) { setMicState('off'); setGuidance('语音配置读取失败，已停用麦克风；请收起后重开气泡重试。'); }
    });
    // Blur cancels an open recording (a half-finished take is useless). Live
    // speech is NOT interrupted by blur: the pet surface keeps the bubble (and
    // this hook) mounted while a turn is transcribing or speaking, so the
    // companion finishes its sentence. Escape, the × button, quiet/hide and a
    // blur outside an active turn still unmount the bubble, whose cleanup
    // interrupts everything.
    const blur = () => {
      if (!recording.current) return;
      generation.current++; release(); recording.current = false;
      setMicState('idle'); setNotice('窗口失焦，本次录音已取消。');
    };
    window.addEventListener('blur', blur);
    return () => { disposed = true; window.removeEventListener('blur', blur); interrupt(); void audioCtx.current?.close().catch(() => {}); };
  }, []);

  return {
    micState, guidance, notice, transcript,
    speaking: micState === 'speaking',
    startRecording: () => void startRecording(),
    finishRecording, stopSpeaking,
  };
}
