import { useEffect, useRef, useState, type FormEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Channel, invoke } from '@tauri-apps/api/core';
import { nativeDesktop } from '../../lib/surface';
import { decodeChatConfig, decodeChatDelta, decodeChatResult, decodeChatHistory, decodeContextEpoch, type ChatTurn, decodeContextPreview, sameScope, type ContextPreview, type MemoryUsage } from '@companion/contracts';
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
  const [preview,setPreview] = useState<ContextPreview|null>(null);
  const previewRef = useRef<ContextPreview|null>(null);
  const [memoryUsage,setMemoryUsage] = useState<MemoryUsage|null>(null);
  const contextEpoch = useRef<number | null>(null);
  const historySerial = useRef(0);
  function invalidateMemory(epoch: number) {
    if (contextEpoch.current !== null && epoch <= contextEpoch.current) return false;
    contextEpoch.current = epoch; historySerial.current++;
    setPreview(null); previewRef.current=null; setMemoryUsage(null);
    const id = current.current; current.current = null;
    if (id) void invoke('chat_cancel', { requestId: id }).catch(() => {});
    setConfigured(false); setBusy(false); setHistory([]); setReply(''); setDraft(''); setLastPrompt(''); setRecorded(false); setPhase('idle');
    setStatus('记忆或会话已变化，旧回复已停止；正在重新读取本机记录。');
    return true;
  }
  async function checkEpoch() {
    const value = decodeContextPreview(await invoke('chat_context_preview'));
    if (!alive.current) return false;
    if (contextEpoch.current === null) return false;
    if (previewRef.current && sameScope(value.scope,previewRef.current.scope) && value.contextEpoch===contextEpoch.current) return true;
    await reconcileMemory(value.contextEpoch);
    return false;
  }
  /** Fire-and-forget preview refresh for non-epoch failures (e.g. a personal
   * admission rejection leaves the epoch unchanged, so checkEpoch would keep
   * showing the stale online preview forever). Only adopts a same-scope read. */
  async function refreshPreview() {
    try {
      const value = decodeContextPreview(await invoke('chat_context_preview'));
      if (alive.current && previewRef.current && sameScope(value.scope, previewRef.current.scope)) {
        previewRef.current = value; setPreview(value);
      }
    } catch { /* transient; the next bracketed read is authoritative */ }
  }
  async function loadHistory() {
    const serial = ++historySerial.current;
    const before = decodeContextPreview(await invoke('chat_context_preview'));
    const turns = decodeChatHistory(await invoke('chat_history'));
    const after = decodeContextPreview(await invoke('chat_context_preview'));
    if (before.contextEpoch !== after.contextEpoch || !sameScope(before.scope,after.scope)) throw Error('会话已变化，请收起后重开');
    if (!alive.current || serial !== historySerial.current) return null;
    contextEpoch.current = after.contextEpoch; previewRef.current=after; setPreview(after);
    return turns;
  }
  async function reconcileMemory(epoch: number) {
    if (!invalidateMemory(epoch)) return true;
    try {
      const [value, turns] = await Promise.all([invoke('chat_config'), loadHistory()]);
      if (alive.current && turns) {
        const config = decodeChatConfig(value);
        setConfigured(config.configured); setModel(config.model); setOutputBudget(config.maxOutputTokens);
        setHistory(turns); setReply(turns.at(-1)?.assistant ?? '');
        setStatus('记忆或会话已变化，已刷新本机记录；请重新输入后发送。');
      }
    } catch { if (alive.current) { setConfigured(false); setStatus('无法核对当前会话，请收起后重开；暂不能发送。'); } }
    return false;
  }
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
    if (nativeDesktop) void Promise.all([invoke('chat_config'), loadHistory()]).then(([configValue, turns]) => {
      if (disposed || !turns) return;
      const config = decodeChatConfig(configValue);
      setConfigured(config.configured); setModel(config.model); setOutputBudget(config.maxOutputTokens); setHistory(turns);
      setReply(turns.at(-1)?.assistant ?? '');
      setStatus(config.configured ? (turns.length ? '已恢复本机记录 · 发送时携带当前模型前文' : '完整问答自动保存在本机 · 尚无当前模型记录') : '尚未配置密钥，请打开模型设置');
    }).catch(error => { if (!disposed) setStatus(typeof error === 'string' ? error : '读取模型或本机记录失败，请检查数据目录后重启'); });
    let unlisten: (() => void) | undefined;
    if (nativeDesktop) void listen<{ contextEpoch: number }>('memory-changed', event => {
      try { void reconcileMemory(decodeContextEpoch(event.payload.contextEpoch)); }
      catch { setConfigured(false); setStatus('记忆通知不兼容，请重开客户端。'); }
    }).then(remove => { if (disposed) remove(); else unlisten = remove; }).catch(() => { if (!disposed) setStatus('窗口通知连接失败，将在发送前重新核对记录。'); });
    const focus = () => { if (nativeDesktop) void checkEpoch().catch(() => { if (!disposed) { setConfigured(false); setStatus('无法核对当前会话，请收起后重开。'); } }); };
    window.addEventListener('focus', focus);
    return () => {
      disposed = true; alive.current = false; historySerial.current++; unlisten?.(); window.removeEventListener('focus', focus);
      const id = current.current; current.current = null;
      if (id) void invoke('chat_cancel', { requestId: id }).catch(() => {});
      onPhase('idle');
    };
  }, [onPhase]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !configured || !draft.trim() || !previewRef.current) return;
    const shownPreview=previewRef.current;
    const id = crypto.randomUUID();
    const prompt = draft.trim();
    current.current = id;
    setBusy(true); setReply(''); setPhase('waiting'); setStatus('正在生成…');
    setLastPrompt(prompt); setRecorded(false); setMemoryUsage(null);
    const channel = new Channel<unknown>();
    let received = '';
    channel.onmessage = value => {
      if (!alive.current || current.current !== id) return;
      try {
        const delta = decodeChatDelta(value);
        if (delta.requestId === id && delta.memoryUsage) setMemoryUsage(delta.memoryUsage);
        if (delta.requestId === id && delta.text) {
          received += delta.text;
          setReply(received); setPhase('streaming');
        }
      } catch {
        current.current = null; setBusy(false); setPhase('error');
        setStatus('回复协议不兼容；本段未加入前文，可编辑后重发');
        void invoke('chat_cancel', { requestId: id }).catch(() => {});
      }
    };
    try {
      if (!(await checkEpoch()) || current.current !== id) return;
      const expectedPersonal = shownPreview.personal.status === 'online'
        ? shownPreview.personal.items.map(({ id: memoryId, seq }) => ({ id: memoryId, seq }))
        : null;
      const result = decodeChatResult(await invoke('chat_generate', { request: { requestId: id, prompt, expectedScope: shownPreview.scope, expectedContextEpoch: shownPreview.contextEpoch, expectedPersonal }, onDelta: channel }));
      if (!(await checkEpoch())) return;
      if (alive.current && current.current === id && result.requestId === id) {
        setMemoryUsage(result.memoryUsage);
        setPhase('complete');
        setStatus('回复结束 · ' + (result.usage?.total_tokens == null ? '用量未返回' : result.usage.total_tokens + ' tokens'));
        if (!result.historySaved) {
          setDraft('');
          setStatus('回复已完成，但未保存记录（存储不可用、内容超限或模型已切换）；本段不会在重开后恢复，也不会作为前文。');
          return;
        }
        try {
          const turns = await loadHistory();
          if (turns && alive.current && current.current === id) {
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
        void refreshPreview();
      }
    } finally {
      if (current.current === id) { if (alive.current) setBusy(false); current.current = null; }
    }
  }
  async function stop() {
    const id = current.current;
    if (!id) return;
    current.current = null; setBusy(false); setPhase('stopped'); setStatus('正在停止…');
    try {
      await invoke('chat_cancel', { requestId: id });
      if (alive.current && !current.current) setStatus('已停止接收回复 · 已产生的用量可能计费');
    } catch { if (alive.current && !current.current) setStatus('停止请求失败，已屏蔽旧回复'); }
  }
  async function clear() {
    if (busy) return;
    setBusy(true);
    try {
      await invoke('chat_clear');
      if (alive.current) {
        setMemoryUsage(null); setHistory([]); setReply(''); setDraft(''); setLastPrompt(''); setRecorded(false); setPhase('idle');
        setStatus('本机全部模型的对话记录已删除，重开不会恢复；服务商留存不受此操作影响');
      }
    } catch (error) { if (alive.current) setStatus(typeof error === 'string' ? error : '清空会话失败'); }
    finally { if (alive.current) setBusy(false); }
  }
  return <div className={reading ? 'chat-bubble chat-bubble-reading' : 'chat-bubble'}>
    <div className="chat-state-line"><span className={`chat-phase chat-phase-${phase}`}>{phaseNames[phase]}</span><span>{generating ? `已等待 ${seconds} 秒` : (phase === 'error' || phase === 'stopped') ? '本段未作为完整前文' : '本机对话'}</span></div>
    {reading ? <ConversationReader history={history} prompt={lastPrompt} reply={reply} pending={!recorded} waiting={generating} label={phase === 'complete' ? '已完成 · 未载入记录' : phaseNames[phase] + ' · 未加入前文'}/>
      : <div className="chat-output" aria-label="栖栖的回复" aria-live="polite">{reply || '想聊点什么？'}</div>}
    {preview && <details className="chat-memory-preview"><summary>下次发送的记忆 · 应用{preview.items.length}条 · 个人{!preview.personal.policy.enabled ? '未启用' : preview.personal.status === 'online' ? preview.personal.items.length + '条' : '服务未连接'}</summary><div>
      <p>{preview.scope.baseUrl} · {preview.scope.model}</p>
      {preview.items.length ? preview.items.map(item=><p key={item.id}>{item.body}<br/><small>{item.sourceLabel} · 经历日期：{item.eventDate ?? '未指定'} · 确认：{new Date(item.confirmedAt).toLocaleString()}</small></p>) : <p>当前模型未选用应用记忆。</p>}
      <p className="chat-memory-family">个人记忆（独立服务）</p>
      {!preview.personal.policy.enabled
        ? <p>个人记忆注入未启用；可在任务面板的“个人记忆”里为当前模型勾选。</p>
        : preview.personal.status === 'offline'
          ? <p>个人记忆服务未连接，本次发送不含个人记忆；可在任务面板管理选择。</p>
          : preview.personal.items.length
            ? preview.personal.items.map(item=><p key={item.id}>{item.title}：{item.content}<br/><small>#{item.id} · {item.project ?? '全局'} · 更新于 {item.updatedAt}</small></p>)
            : <p>当前模型未选用个人记忆。</p>}
      {preview.personal.status === 'online' && preview.personal.inactiveSelectedIds.length > 0 && <p>已选的 {preview.personal.inactiveSelectedIds.length} 条个人记忆已失效（被取代或过期），本次不携带。</p>}
      <small>正文：应用{preview.bodyChars}字 · 个人{preview.personal.bodyChars}字；字符数不是tokens。</small>
    </div></details>}
    {memoryUsage && <details className="chat-memory-preview chat-memory-receipt"><summary>本轮已提交 · 应用{memoryUsage.memories.length}条 · 个人{memoryUsage.personal.status === 'offline' ? '未含（服务未连接）' : memoryUsage.personal.memories.length + '条'}</summary><div><p>{memoryUsage.scope.baseUrl} · {memoryUsage.scope.model}</p>{memoryUsage.memories.map(item=><p key={item.id}>{preview?.items.find(m=>m.id===item.id&&m.revision===item.revision)?.body ?? '条目已变化，请刷新预览'}<br/>第{item.revision}版</p>)}{memoryUsage.personal.status === 'sent' && memoryUsage.personal.memories.map(item=><p key={item.id}>{preview?.personal.items.find(m=>m.id===item.id&&m.seq===item.seq)?.title ?? '条目已变化或服务暂不可查，请刷新预览'}<br/>seq {item.seq}</p>)}<small>附加字符：应用{memoryUsage.contextChars} · 个人{memoryUsage.personal.status === 'sent' ? memoryUsage.personal.contextChars : 0}；提交不代表模型已引用。记录仅在当前窗口保留。</small></div></details>}
    <form onSubmit={submit}>
      <label className="sr-only" htmlFor="chat-draft">和栖栖说句话</label>
      <input id="chat-draft" maxLength={2000} value={draft} onChange={e => setDraft(e.target.value)} placeholder="和我说说…" disabled={busy}/>
      {busy ? <button type="button" className="pet-save" onClick={event => { event.preventDefault(); void stop(); }} disabled={!current.current}>停止</button> : <button className="pet-save" disabled={!configured || !preview || !draft.trim()}>发送</button>}
    </form>
    <p className="chat-status" role="status">{status}</p>
    <div className="chat-session-controls">
      <button type="button" aria-expanded={reading} onClick={() => setReading(v => !v)}>{reading ? '收回气泡' : '展开阅读'}</button>
      {reading ? <span className="chat-history-count">最近 {history.length} 轮</span> : <details><summary>最近 {history.length} 轮</summary><div className="chat-history" aria-label="本机对话记录">
        {history.length ? history.map((turn, index) => <div key={index}><p><strong>我：</strong>{turn.user}</p><p><strong>栖栖：</strong>{turn.assistant}</p></div>) : <p>还没有完整对话。</p>}
      </div></details>}
      <button type="button" title="删除本机全部模型的对话记录" onClick={() => void clear()} disabled={!nativeDesktop || busy}>清空对话</button>
    </div>
    <small className="chat-model">{model || '未连接'} · 上限{outputBudget} tokens<br/>本机明文保存 · 重开恢复 · 全部模型共6轮/1.2万字<br/>按地址和模型隔离 · 清空删除全部记录</small>
  </div>;
}
