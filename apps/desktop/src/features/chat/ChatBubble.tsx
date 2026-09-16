import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { nativeDesktop } from '../../lib/surface';
import { decodeChatConfig, decodeChatDelta, decodeChatResult, decodeChatHistory, type ChatTurn } from '@companion/contracts';
import { ConversationReader } from './ConversationReader';
import type { ConversationPhase as Phase } from '../companion/presentation';

const phaseNames: Record<Phase, string> = { idle: '准备好了', waiting: '等待回复', streaming: '正在回复', complete: '已完成', stopped: '已停止', error: '未完成' };
export function ChatBubble({ onPhase, onReading }: { onPhase: (phase: Phase) => void; onReading: (active: boolean) => void }) {
  const [configured, setConfigured] = useState(false);
  const [model, setModel] = useState('');
  const [outputBudget, setOutputBudget] = useState(1024);
  const [draft, setDraft] = useState('');
  const [reply, setReply] = useState('');
  const [history, setHistory] = useState<ChatTurn[]>([]);
  const [status, setStatus] = useState(nativeDesktop ? '正在检查模型配置…' : '浏览器预览不调用模型，请运行桌面版');
  const [busy, setBusy] = useState(false);
  const [reading, setReading] = useState(false);
  const [phase, setPhase] = useState<Phase>('idle');
  const [seconds, setSeconds] = useState(0);
  const [lastPrompt, setLastPrompt] = useState('');
  const [recorded, setRecorded] = useState(false);
  const current = useRef<string | null>(null);
  const alive = useRef(true);
  const generating = phase === 'waiting' || phase === 'streaming';
  useEffect(() => {
    if (!generating) return;
    const start = Date.now(); setSeconds(0);
    const timer = setInterval(() => setSeconds(Math.floor((Date.now() - start) / 1000)), 1000);
    return () => clearInterval(timer);
  }, [generating]);
  useEffect(() => { onReading(reading); return () => onReading(false); }, [reading, onReading]);
  useEffect(() => { onPhase(phase); }, [phase, onPhase]);
  useEffect(() => {
    alive.current = true;
    let disposed = false;
    if (nativeDesktop) void Promise.all([invoke('chat_config'), invoke('chat_history')]).then(([configValue, historyValue]) => {
      if (disposed) return;
      const config = decodeChatConfig(configValue);
      const turns = decodeChatHistory(historyValue);
      setConfigured(config.configured); setModel(config.model); setOutputBudget(config.maxOutputTokens); setHistory(turns);
      setReply(turns.at(-1)?.assistant ?? '');
      setStatus(config.configured ? '临时会话 · 发送时携带最近前文' : '尚未配置密钥，请打开模型设置');
    }).catch(() => { if (!disposed) setStatus('读取模型或会话配置失败'); });
    return () => {
      disposed = true; alive.current = false;
      const id = current.current; current.current = null;
      if (id) void invoke('chat_cancel', { requestId: id }).catch(() => {});
      onPhase('idle');
    };
  }, [onPhase]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !configured || !draft.trim()) return;
    const id = crypto.randomUUID();
    const prompt = draft.trim();
    current.current = id;
    setBusy(true); setReply(''); setPhase('waiting'); setStatus('正在生成…');
    setLastPrompt(prompt); setRecorded(false);
    const channel = new Channel<unknown>();
    let received = '';
    channel.onmessage = value => {
      if (!alive.current || current.current !== id) return;
      try {
        const delta = decodeChatDelta(value);
        if (delta.requestId === id && delta.text) {
          received += delta.text;
          setReply(received); setPhase('streaming');
        }
      } catch {
        current.current = null; setPhase('error');
        setStatus('回复协议不兼容；本段未加入前文，可编辑后重发');
        void invoke('chat_cancel', { requestId: id }).catch(() => {});
      }
    };
    try {
      const result = decodeChatResult(await invoke('chat_generate', { request: { requestId: id, prompt }, onDelta: channel }));
      if (alive.current && current.current === id && result.requestId === id) {
        setPhase('complete');
        setStatus('回复结束 · ' + (result.usage?.total_tokens == null ? '用量未返回' : result.usage.total_tokens + ' tokens'));
        try {
          const turns = decodeChatHistory(await invoke('chat_history'));
          if (alive.current && current.current === id) {
            setHistory(turns); setDraft('');
            setRecorded(turns.at(-1)?.user === prompt && turns.at(-1)?.assistant === received);
          }
        } catch {
          if (alive.current && current.current === id) setStatus('回复已完成，但记录读取失败；收起后重开可重试读取');
        }
      }
    } catch (error) {
      if (alive.current && current.current === id) {
        setPhase('error');
        setStatus((typeof error === 'string' ? error : '请求失败') + ' · 未加入前文，可编辑后重发');
      }
    } finally {
      if (alive.current) setBusy(false);
      if (current.current === id) current.current = null;
    }
  }
  async function stop() {
    const id = current.current;
    if (!id) return;
    current.current = null; setPhase('stopped'); setStatus('正在停止…');
    try {
      await invoke('chat_cancel', { requestId: id });
      if (alive.current) setStatus('已停止接收回复 · 已产生的用量可能计费');
    } catch { if (alive.current) setStatus('停止请求失败，已屏蔽旧回复'); }
  }
  async function clear() {
    if (busy) return;
    setBusy(true);
    try {
      await invoke('chat_clear');
      if (alive.current) {
        setHistory([]); setReply(''); setDraft(''); setLastPrompt(''); setRecorded(false); setPhase('idle');
        setStatus('本机会话已清空，下一条从新对话开始');
      }
    } catch (error) { if (alive.current) setStatus(typeof error === 'string' ? error : '清空会话失败'); }
    finally { if (alive.current) setBusy(false); }
  }
  return <div className={reading ? 'chat-bubble chat-bubble-reading' : 'chat-bubble'}>
    <div className="chat-state-line"><span className={`chat-phase chat-phase-${phase}`}>{phaseNames[phase]}</span><span>{generating ? `已等待 ${seconds} 秒` : (phase === 'error' || phase === 'stopped') ? '本段未作为完整前文' : '临时会话'}</span></div>
    {reading ? <ConversationReader history={history} prompt={lastPrompt} reply={reply} pending={!recorded} waiting={generating} label={phase === 'complete' ? '已完成 · 未载入记录' : phaseNames[phase] + ' · 未加入前文'}/>
      : <div className="chat-output" aria-label="栖栖的回复" aria-live="polite">{reply || '想聊点什么？'}</div>}
    <form onSubmit={submit}>
      <label className="sr-only" htmlFor="chat-draft">和栖栖说句话</label>
      <input id="chat-draft" maxLength={2000} value={draft} onChange={e => setDraft(e.target.value)} placeholder="和我说说…" disabled={busy}/>
      {busy ? <button type="button" className="pet-save" onClick={event => { event.preventDefault(); void stop(); }} disabled={!current.current}>停止</button> : <button className="pet-save" disabled={!configured || !draft.trim()}>发送</button>}
    </form>
    <p className="chat-status" role="status">{status}</p>
    <div className="chat-session-controls">
      <button type="button" aria-expanded={reading} onClick={() => setReading(v => !v)}>{reading ? '收回气泡' : '展开阅读'}</button>
      {reading ? <span className="chat-history-count">最近 {history.length} 轮</span> : <details><summary>最近 {history.length} 轮</summary><div className="chat-history" aria-label="本次对话记录">
        {history.length ? history.map((turn, index) => <div key={index}><p><strong>我：</strong>{turn.user}</p><p><strong>栖栖：</strong>{turn.assistant}</p></div>) : <p>还没有完整对话。</p>}
      </div></details>}
      <button type="button" onClick={() => void clear()} disabled={!nativeDesktop || busy || (!history.length && !reply)}>清空对话</button>
    </div>
    <small className="chat-model">{model || '未连接'} · 上限{outputBudget} tokens<br/>最近6轮/1.2万字 · 收起停生成 · 重启清空</small>
  </div>;
}
