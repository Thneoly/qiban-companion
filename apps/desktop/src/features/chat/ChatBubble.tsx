import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { nativeDesktop } from '../../lib/surface';
import { decodeChatConfig, decodeChatDelta, decodeChatResult, decodeChatHistory, type ChatTurn } from '@companion/contracts';

export function ChatBubble({ onThinking }: { onThinking: (active: boolean) => void }) {
  const [configured, setConfigured] = useState(false);
  const [model, setModel] = useState('');
  const [draft, setDraft] = useState('');
  const [reply, setReply] = useState('');
  const [history, setHistory] = useState<ChatTurn[]>([]);
  const [status, setStatus] = useState(nativeDesktop ? '正在检查模型配置…' : '浏览器预览不调用模型，请运行桌面版');
  const [busy, setBusy] = useState(false);
  const current = useRef<string | null>(null);
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    let disposed = false;
    if (nativeDesktop) void Promise.all([invoke('chat_config'), invoke('chat_history')]).then(([configValue, historyValue]) => {
      if (disposed) return;
      const config = decodeChatConfig(configValue);
      const turns = decodeChatHistory(historyValue);
      setConfigured(config.configured);
      setModel(config.model);
      setHistory(turns);
      setReply(turns.at(-1)?.assistant ?? '');
      setStatus(config.configured ? '临时会话 · 发送时携带最近前文' : '尚未配置密钥，请打开模型设置');
    }).catch(() => { if (!disposed) setStatus('读取模型或会话配置失败'); });
    return () => {
      disposed = true;
      alive.current = false;
      const id = current.current;
      current.current = null;
      if (id) void invoke('chat_cancel', { requestId: id }).catch(() => {});
      onThinking(false);
    };
  }, [onThinking]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !configured || !draft.trim()) return;
    const id = crypto.randomUUID();
    current.current = id;
    setBusy(true); setReply(''); setStatus('正在生成…'); onThinking(true);
    const channel = new Channel<unknown>();
    channel.onmessage = value => {
      if (!alive.current || current.current !== id) return;
      try {
        const delta = decodeChatDelta(value);
        if (delta.requestId === id) setReply(text => text + delta.text);
      } catch {
        current.current = null;
        setStatus('回复协议不兼容');
        void invoke('chat_cancel', { requestId: id }).catch(() => {});
      }
    };
    try {
      const result = decodeChatResult(await invoke('chat_generate', { request: { requestId: id, prompt: draft.trim() }, onDelta: channel }));
      if (alive.current && current.current === id && result.requestId === id) {
        setStatus('回复结束 · ' + (result.usage?.total_tokens == null ? '用量未返回' : result.usage.total_tokens + ' tokens'));
        const turns = decodeChatHistory(await invoke('chat_history'));
        if (alive.current && current.current === id) { setHistory(turns); setDraft(''); }
      }
    } catch (error) {
      if (alive.current && current.current === id) setStatus(typeof error === 'string' ? error : '请求或记录读取失败，请重试');
    } finally {
      if (alive.current) { setBusy(false); onThinking(false); }
      if (current.current === id) current.current = null;
    }
  }

  async function stop() {
    const id = current.current;
    if (!id) return;
    current.current = null; setStatus('正在停止…');
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
      if (alive.current) { setHistory([]); setReply(''); setDraft(''); setStatus('本机会话已清空，下一条从新对话开始'); }
    } catch (error) { if (alive.current) setStatus(typeof error === 'string' ? error : '清空会话失败'); }
    finally { if (alive.current) setBusy(false); }
  }

  return <div className="chat-bubble">
    <div className="chat-output" aria-label="栖栖的回复" aria-live="polite">{reply || '想聊点什么？'}</div>
    <form onSubmit={submit}>
      <label className="sr-only" htmlFor="chat-draft">和栖栖说句话</label>
      <input id="chat-draft" maxLength={2000} value={draft} onChange={e => setDraft(e.target.value)} placeholder="和我说说…" disabled={busy}/>
      {busy ? <button type="button" className="pet-save" onClick={event => { event.preventDefault(); void stop(); }} disabled={!current.current}>停止</button> : <button className="pet-save" disabled={!configured || !draft.trim()}>发送</button>}
    </form>
    <p className="chat-status" role="status">{status}</p>
    <div className="chat-session-controls">
      <details><summary>最近 {history.length} 轮</summary><div className="chat-history" aria-label="本次对话记录">
        {history.length ? history.map((turn, index) => <div key={index}><p><strong>我：</strong>{turn.user}</p><p><strong>栖栖：</strong>{turn.assistant}</p></div>) : <p>还没有完整对话。</p>}
      </div></details>
      <button type="button" onClick={() => void clear()} disabled={!nativeDesktop || busy || (!history.length && !reply)}>清空对话</button>
    </div>
    <small className="chat-model">{model || '未连接'} · 最多6轮/1.2万字 · 重启清空<br/>收起停止生成，已完成对话保留</small>
  </div>;
}