import { useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { decodeModelSettings, decodeVoiceResult, decodeVoiceSettings, type VoiceResult } from '@companion/contracts';
import { nativeDesktop } from '../../lib/surface';
import { normalizeRecording } from './audio';

type Phase = 'idle' | 'permission' | 'recording' | 'preparing' | 'recognizing' | 'generating' | 'synthesizing' | 'playing' | 'complete' | 'stopped' | 'error';
const labels: Record<Phase, string> = { idle: '准备语音', permission: '等待麦克风授权', recording: '正在录音（最多10秒）', preparing: '正在准备音频', recognizing: '正在识别', generating: '正在生成回复', synthesizing: '正在合成语音', playing: '正在播放', complete: '播放已结束', stopped: '已停止', error: '未完成' };

export function VoiceLab() {
  const [expanded, setExpanded] = useState(false);
  return <section className="model-settings voice-lab" aria-label="语音实验">
    <h2>语音实验</h2>
    <p>验证“录音 → 识别 → 回答 → 播放”。实验入口，尚未接入桌宠语音与口型。</p>
    <button onClick={() => setExpanded(v => !v)} aria-expanded={expanded}>{expanded ? '关闭语音实验' : '打开语音实验'}</button>
    {expanded && <VoiceProbe/>}
  </section>;
}
function VoiceProbe() {
  const [asr, setAsr] = useState(''); const [tts, setTts] = useState(''); const [voice, setVoice] = useState('');
  const [service, setService] = useState('正在读取当前模型服务…');
  const [baseUrl, setBaseUrl] = useState('');
  const [voiceBase, setVoiceBase] = useState(''); const [useVoiceKey, setUseVoiceKey] = useState(true); const [keyBusy, setKeyBusy] = useState(false);
  const [phase, setPhase] = useState<Phase>('idle'); const [note, setNote] = useState('点击录音才会申请麦克风；也可选择测试WAV。');
  const [wav, setWav] = useState<Uint8Array | null>(null); const [result, setResult] = useState<VoiceResult | null>(null);
  const [playbackMs, setPlaybackMs] = useState<number | null>(null);
  const [source, setSource] = useState<'microphone' | 'file'>('file'); const [preparedName, setPreparedName] = useState<string | null>(null);
  const generation = useRef(0); const current = useRef<string | null>(null); const busyRef = useRef(false);
  const recorder = useRef<MediaRecorder | null>(null); const stream = useRef<MediaStream | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null); const audio = useRef<HTMLAudioElement | null>(null); const url = useRef<string | null>(null);
  const busy = !['idle', 'complete', 'error', 'stopped'].includes(phase);
  busyRef.current = busy;

  function release() {
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    if (recorder.current?.state === 'recording') recorder.current.stop();
    stream.current?.getTracks().forEach(t => t.stop()); stream.current = null;
    if (audio.current) { audio.current.pause(); audio.current.onplaying = null; audio.current.onended = null; audio.current.onerror = null; audio.current.removeAttribute('src'); audio.current.load(); audio.current = null; }
    if (url.current) { URL.revokeObjectURL(url.current); url.current = null; }
  }
  function cancel(update = true) {
    generation.current++;
    const id = current.current; current.current = null;
    release();
    if (id) void invoke('voice_cancel', { requestId: id }).catch(() => {});
    if (update) { setPhase('stopped'); setNote('已停止本地录音和播放，并请求取消网络；在途调用可能计费。'); }
  }
  useEffect(() => {
    let disposed = false;
    if (nativeDesktop) {
      void invoke('model_settings_get').then(value => { if (!disposed) { const config = decodeModelSettings(value); setBaseUrl(config.baseUrl); setService(`文字服务：${config.baseUrl} · ${config.model}`); } }).catch(() => { if (!disposed) setService('读取服务失败，请先检查模型设置'); });
      // Saved labs config pre-fills the form; a failed read keeps the empty
      // form usable rather than blocking the experiment.
      void invoke('voice_settings_get').then(value => { if (!disposed) { const saved = decodeVoiceSettings(value); if (saved.voiceBaseUrl) { setVoiceBase(saved.voiceBaseUrl); setUseVoiceKey(saved.useVoiceKey); setAsr(saved.asrModel); setTts(saved.ttsModel); setVoice(saved.voice); } } }).catch(() => {});
    } else setService('浏览器仅预览；语音服务在桌面版调用');
    const blur = () => { if (busyRef.current) cancel(); };
    window.addEventListener('blur', blur);
    return () => { disposed = true; cancel(false); window.removeEventListener('blur', blur); };
  }, []);

  async function prepare(blob: Blob, id: number, name: string | null = null) {
    try { const bytes = await normalizeRecording(blob); if (generation.current !== id) return; setWav(bytes); setPreparedName(name); setPhase('idle'); setNote(name ? `音频已准备（${name}）。点击“运行并播放”后才上传到当前服务。` : '音频已准备。点击“运行并播放”后才上传到当前服务。'); }
    catch { if (generation.current === id) { setPhase('error'); setNote('音频无法解码或超过30秒，请重新录制/选择PCM16 WAV。'); } }
  }
  function replay() {
    // The picker's value is cleared on purpose so re-picking the same file fires
    // change; that leaves "未选择文件" on screen, so the prepared name and a
    // replay path make the kept audio observable. First-play latency stays the
    // original submit-to-play number; replays never touch it.
    if (!url.current) return;
    const epoch = generation.current;
    const player = new Audio(url.current); audio.current = player;
    player.onplaying = () => { if (generation.current === epoch) setPhase('playing'); };
    player.onended = () => { if (generation.current === epoch) setPhase('complete'); };
    player.onerror = () => { if (generation.current === epoch) { release(); setPhase('error'); setNote('音频播放失败；合成成功不代表已经播放。'); } };
    setPhase('playing');
    void player.play().catch(() => {});
  }
  async function record() {
    const id = ++generation.current; release(); setWav(null); setResult(null); setPlaybackMs(null); setPhase('permission'); setSource('microphone'); setPreparedName(null);
    try {
      const acquired = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true }, video: false });
      if (generation.current !== id) { acquired.getTracks().forEach(t => t.stop()); return; }
      stream.current = acquired;
      const capture = new MediaRecorder(acquired); recorder.current = capture;
      const parts: Blob[] = [];
      capture.ondataavailable = event => { if (event.data.size) parts.push(event.data); };
      capture.onstop = () => {
        acquired.getTracks().forEach(t => t.stop());
        if (timer.current) { clearTimeout(timer.current); timer.current = null; }
        if (generation.current !== id) return;
        setPhase('preparing'); void prepare(new Blob(parts, { type: capture.mimeType }), id);
      };
      capture.onerror = () => { if (generation.current === id) { cancel(); setPhase('error'); setNote('录音设备发生错误，请重试。'); } };
      capture.start(); setPhase('recording');
      timer.current = setTimeout(() => { if (capture.state === 'recording') capture.stop(); }, 10000);
    } catch { if (generation.current === id) { release(); setPhase('error'); setNote('无法使用麦克风：请检查系统权限、设备或改用测试WAV。'); } }
  }
  async function run() {
    if (!wav || busy || !nativeDesktop) return;
    const epoch = ++generation.current; release(); const id = crypto.randomUUID(); current.current = id;
    setResult(null); setPlaybackMs(null); setPhase('recognizing'); setNote('音频发送到语音地址，识别文本发送到文字服务，回复送交语音合成；不写入聊天历史。');
    const started = performance.now();
    const channel = new Channel<unknown>();
    channel.onmessage = value => {
      if (generation.current !== epoch || !value || typeof value !== 'object') return;
      const v = value as { requestId?: unknown; stage?: unknown };
      if (v.requestId === id && (v.stage === 'recognizing' || v.stage === 'generating' || v.stage === 'synthesizing')) setPhase(v.stage);
    };
    try {
      const data = decodeVoiceResult(await invoke('voice_probe', { request: { requestId: id, expectedBaseUrl: baseUrl, voiceBaseUrl: voiceBase.trim(), useVoiceKey, asrModel: asr.trim(), ttsModel: tts.trim(), voice: voice.trim(), wav: Array.from(wav) }, onStage: channel }));
      if (generation.current !== epoch) return;
      if (data.requestId !== id) throw Error('响应请求不匹配');
      current.current = null; setResult(data);
      const blob = new Blob([Uint8Array.from(data.wav)], { type: 'audio/wav' });
      url.current = URL.createObjectURL(blob);
      const player = new Audio(url.current); audio.current = player;
      player.onplaying = () => { if (generation.current === epoch) { setPhase('playing'); setPlaybackMs(Math.round(performance.now() - started)); } };
      player.onended = () => { if (generation.current === epoch) { setPhase('complete'); setNote('本轮播放结束；可点“重播合成语音”再听一次。录音和回复仅保留在当前实验界面。'); } };
      player.onerror = () => { if (generation.current === epoch) { release(); setPhase('error'); setNote('音频播放失败；合成成功不代表已经播放。'); } };
      await player.play();
    } catch (error) { if (generation.current === epoch) { current.current = null; release(); setPhase('error'); setNote(typeof error === 'string' ? error : error instanceof Error ? error.message : '语音实验失败'); } }
  }
  return <div className="voice-probe">
    <p>{service}</p>
    <p>语音地址可与文字服务不同，密钥按地址隔离保存。识别和合成需兼容 /audio/transcriptions 与 /audio/speech；非密钥配置仅用于本轮实验。</p>
    <fieldset disabled={busy || keyBusy || !nativeDesktop}>
      <button type="button" onClick={() => { setVoiceBase('https://open.bigmodel.cn/api/paas/v4'); setUseVoiceKey(true); setAsr('glm-asr-2512'); setTts('glm-tts'); setVoice('tongtong'); }}>填入智谱语音示例</button>
      <label>语音 API 基地址<input value={voiceBase} maxLength={512} onChange={e => setVoiceBase(e.target.value)}/></label>
      <label><input type="checkbox" checked={useVoiceKey} onChange={e => setUseVoiceKey(e.target.checked)}/>语音服务需要 API Key</label>
      <button type="button" disabled={!voiceBase.trim() || !useVoiceKey} onClick={async () => { setKeyBusy(true); try { await invoke('voice_key_set', { baseUrl: voiceBase.trim() }); setNote('已为该语音地址保存系统密钥，文字服务设置未改动。'); } catch (e) { setNote(typeof e === 'string' ? e : '密钥设置未完成'); } finally { setKeyBusy(false); } }}>设置语音密钥</button>
      <button type="button" disabled={!voiceBase.trim() || !asr.trim() || !tts.trim() || !voice.trim()} onClick={async () => { try { await invoke('voice_settings_save', { config: { voiceBaseUrl: voiceBase.trim(), useVoiceKey, asrModel: asr.trim(), ttsModel: tts.trim(), voice: voice.trim() } }); setNote('语音配置已保存，下次打开语音实验自动填入；密钥仍单独保存在系统凭据管理器。'); } catch (e) { setNote(typeof e === 'string' ? e : '语音配置保存失败'); } }}>保存语音配置</button>
      <label>识别模型<input value={asr} maxLength={160} onChange={e => setAsr(e.target.value)}/></label>
      <label>合成模型<input value={tts} maxLength={160} onChange={e => setTts(e.target.value)}/></label>
      <label>音色编码<input value={voice} maxLength={160} onChange={e => setVoice(e.target.value)}/></label>
      <button type="button" onClick={() => void record()}>录制语音</button>
      <label>选择测试WAV<input type="file" accept="audio/wav,.wav" onChange={e => { const file = e.target.files?.[0]; if (!file) return; const id = ++generation.current; release(); setResult(null); setWav(null); setPlaybackMs(null); setSource('file'); setPhase('preparing'); void prepare(file, id, file.name); e.target.value = ''; }}/></label>
      <button type="button" disabled={!baseUrl || !voiceBase.trim() || !wav || !asr.trim() || !tts.trim() || !voice.trim()} onClick={() => void run()}>运行并播放</button>
    </fieldset>
    {phase === 'recording' && <button onClick={() => recorder.current?.stop()}>结束录音</button>}
    {busy && <button onClick={() => cancel()}>停止语音实验</button>}
    {phase === 'complete' && <button onClick={replay}>重播合成语音</button>}
    <p className="voice-phase" role="status">{labels[phase]}</p><p className="voice-note">{note}</p>
    {result && <><p>识别：{result.transcript}</p><p>栖栖：{result.reply}</p><table><tbody>
      <tr><th>音频来源</th><td>{source === 'microphone' ? '本次录音' : preparedName ? `测试文件（${preparedName}）` : '测试文件'}</td></tr>
      <tr><th>识别 / 生成 / 合成</th><td>{result.recognitionMs} / {result.generationMs} / {result.synthesisMs} ms</td></tr>
      <tr><th>提交至播放开始</th><td>{playbackMs === null ? '尚未开始播放' : `${playbackMs} ms`}</td></tr>
      <tr><th>输入 / 输出时长</th><td>{result.inputSeconds.toFixed(2)} / {result.outputSeconds.toFixed(2)} 秒</td></tr>
      <tr><th>文本用量</th><td>{result.modelTotalTokens === null ? '未返回' : `${result.modelTotalTokens} tokens`}</td></tr>
      <tr><th>语音费用</th><td>未知，需按服务商账单核对</td></tr>
    </tbody></table></>}
  </div>;
}
