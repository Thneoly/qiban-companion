import { MemoryPolicyPanel } from './MemoryPolicyPanel';
import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { decodeMemorySnapshot, decodeMemoryReceipt, decodeMemoryExport, memoryErrorMessage, type MemoryDraft, type MemoryRecord, type MemorySnapshot } from '@companion/contracts';
import { nativeDesktop } from '../../lib/surface';
import { useUsageImpact } from '../../lib/usage-impact';
import './memory.css';

type Pending = { action: 'update' | 'delete' | 'delete_all'; expectedEpoch: number; id?: string; expectedRevision?: number; draft?: MemoryDraft; impactIds: string[] };
const empty: MemoryDraft = { kind: 'preference', body: '', eventDate: null };
const formatTime = (value: number) => new Date(value).toLocaleString();
export function MemoryPanel() {
  const [snapshot, setSnapshot] = useState<MemorySnapshot | null>(null);
  const [draft, setDraft] = useState<MemoryDraft>(empty);
  const [editing, setEditing] = useState<MemoryRecord | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState('');
  const [error, setError] = useState('');
  const alive = useRef(true), serial = useRef(0), acting = useRef(false);
  const refresh = useCallback(async () => {
    const id = ++serial.current;
    try {
      const next = decodeMemorySnapshot(await invoke('memory_list'));
      if (alive.current && serial.current === id) { setSnapshot(next); setError(''); }
      return true;
    } catch (e) {
      if (alive.current && serial.current === id) { setSnapshot(null); setError(memoryErrorMessage(e)); }
      return false;
    }
  }, []);
  useEffect(() => {
    alive.current = true;
    const removers: (() => void)[] = [];
    if (nativeDesktop) {
      void refresh();
      for (const [name, callback] of [
        ['memory-changed', () => { void refresh(); }],
        ['memory-open', () => { document.getElementById('memories-title')?.scrollIntoView({ block: 'start' }); void refresh(); }],
      ] as const) void listen(name, callback).then(remove => { if (alive.current) removers.push(remove); else remove(); }).catch(() => setError('窗口通知连接失败，请使用刷新按钮核对记录。'));
    }
    const focus = () => { if (nativeDesktop && !acting.current) void refresh(); };
    window.addEventListener('focus', focus);
    return () => { alive.current = false; serial.current++; removers.forEach(remove => remove()); window.removeEventListener('focus', focus); };
  }, [refresh]);
  async function mutate(request: object, success: string) {
    if (acting.current) return;
    acting.current = true; setBusy(true); setError(''); setNote('');
    try {
      const receipt = decodeMemoryReceipt(await invoke('memory_mutate', { request }));
      if (!alive.current) return;
      setPending(null); setEditing(null); setDraft(empty); setSnapshot(null);
      const message = success + (receipt.clearedTurns > 0 ? ` 已清除使用过该记忆的最近 ${receipt.clearedTurns} 轮对话，其余保留。` : ' 已有聊天记录保留。');
      setNote(message + (receipt.notificationsDelivered ? '' : ' 本机已提交，另一窗口待刷新。'));
      if (!(await refresh())) setNote(message + ' 列表暂未刷新，请重试读取；不必重复提交。');
    } catch (e) { if (alive.current) { setError(memoryErrorMessage(e)); setPending(null); } }
    finally { acting.current = false; if (alive.current) setBusy(false); }
  }
  function save(event: FormEvent) {
    event.preventDefault();
    if (!snapshot || busy) return;
    if (!draft.body.trim() || [...draft.body.trim()].length > 200) { setError('正文需要包含1～200个字符，表情按Unicode字符计数。'); return; }
    if (editing) setPending({ action: 'update', id: editing.id, expectedRevision: editing.revision, expectedEpoch: snapshot.contextEpoch, draft: { ...draft }, impactIds: [editing.id] });
    else void mutate({ action: 'create', draft, expectedEpoch: snapshot.contextEpoch }, '记忆已保存在本机，新增条目尚未被模型选用。');
  }
  async function exportMemories() {
    if (acting.current) return;
    acting.current = true; setBusy(true); setError(''); setNote('');
    try {
      const result = decodeMemoryExport(await invoke('memory_export'));
      if (alive.current) setNote(result.status === 'cancelled' ? '已取消导出，没有生成文件。' : `已导出${result.count}条有效记忆。导出文件不会随之后的删除自动撤回。`);
    } catch (e) { if (alive.current) setError(memoryErrorMessage(e)); }
    finally { acting.current = false; if (alive.current) setBusy(false); }
  }
  const count = [...draft.body].length;
  // Consultative precount while the dialog is open. Update/delete/delete_all
  // have no still-selecting exemption, so the number is a plain estimate; the
  // static wording stays while it loads or fails (receipt is authoritative).
  const impact = useUsageImpact(pending !== null, pending?.impactIds ?? [], []);
  const impactDescription = !pending ? '' : !impact
    ? `这会停止正在生成的回复，并删除使用过${pending.action === 'delete_all' ? '这些' : '这条'}记忆的对话：从各模型对话中第一次使用它的一轮起全部清除，之前的对话保留；没有使用记录的模型对话不变。`
    : impact.affectedTurnsTotal > 0
      ? `这会停止正在生成的回复，并删除使用过${pending.action === 'delete_all' ? '这些' : '这条'}记忆的对话：从各模型对话中第一次使用它的一轮起，预计清除最近 ${impact.affectedTurnsTotal} 轮，之前的对话保留；没有使用记录的模型对话不变。实际以提交后的回执为准。`
      : `这会停止正在生成的回复。目前没有本机对话使用过${pending.action === 'delete_all' ? '这些' : '这条'}记忆，预计聊天记录保持不变；实际以提交后的回执为准。`;
  return <section className="memory-panel" aria-labelledby="memories-title">
    <div className="memory-heading"><div><span className="eyebrow">LITTLE THINGS WE KEEP</span><h2 id="memories-title">我们的记忆</h2></div><span className="memory-badge">本机记忆 · 模型使用需单独授权</span></div>
    <p>把希望留下的小事亲自写在这里。保存聊天不会自动生成记忆；是否随对话发送，由下方每个模型的使用设置决定。</p>
    {!nativeDesktop && <p className="memory-notice">浏览器仅预览界面，不保存或导出记忆。请在桌面版使用。</p>}
    <div className="memory-toolbar"><span>{snapshot ? `${snapshot.items.length} / 30 条` : nativeDesktop ? '记录尚未载入' : '桌面功能预览'}</span><button disabled={busy || !nativeDesktop} onClick={() => void refresh()}>刷新记忆</button><button disabled={busy || !snapshot} onClick={() => void exportMemories()}>导出 JSON</button><button className="memory-danger" disabled={busy || !snapshot?.items.length} onClick={() => snapshot && setPending({ action: 'delete_all', expectedEpoch: snapshot.contextEpoch, impactIds: snapshot.items.map(item => item.id) })}>删除全部记忆</button></div>
    {error && <p role="alert" className="memory-error">{error}</p>}
    {note && <p role="status" className="memory-notice">{note}</p>}
    {pending && <div className="memory-confirm" role="alertdialog" aria-labelledby="memory-confirm-title" aria-describedby="memory-confirm-description">
      <h3 id="memory-confirm-title">{pending.action === 'update' ? '确认更正记忆' : pending.action === 'delete_all' ? '确认删除全部记忆' : '确认删除这条记忆'}</h3>
      <p id="memory-confirm-description">{impactDescription}{pending.action === 'delete_all' ? '全部记忆正文将删除。' : pending.action === 'delete' ? '此条记忆正文将删除。' : '原正文将被替换。'}已发给服务商的内容和已导出的文件不能撤回。</p>
      <button autoFocus disabled={busy} onClick={() => pending && void mutate({ ...pending, restartConversation: true }, pending.action === 'update' ? '记忆已更正。' : '记忆已删除。')}>确认并开始新对话</button><button disabled={busy} onClick={() => setPending(null)}>返回，不修改</button>
    </div>}
    <div className="memory-layout">
      <form onSubmit={save} className="memory-form">
        <h3>{editing ? '更正这条记忆' : '留下一件小事'}</h3>
        <fieldset disabled={busy || !snapshot || !!pending}>
          <label>记忆类型<select value={draft.kind} onChange={e => setDraft({ ...draft, kind: e.target.value as MemoryDraft['kind'] })}><option value="preference">我的偏好</option><option value="experience">共同经历</option></select></label>
          <label>记忆内容<textarea rows={5} maxLength={800} value={draft.body} onChange={e => setDraft({ ...draft, body: e.target.value })} placeholder="例如：讨论方案时，先给结论，再讲理由。" aria-describedby="memory-body-count"/></label>
          <small id="memory-body-count" className={count > 200 ? 'memory-error' : ''}>{count} / 200 字符</small>
          <label>经历日期（可选）<input type="date" value={draft.eventDate ?? ''} onChange={e => setDraft({ ...draft, eventDate: e.target.value || null })}/></label>
          <p className="memory-help">来源固定为“用户在记忆面板填写”。新增会停止当前生成，保留已有聊天；更正须确认开始新对话。</p>
          <button type="submit" disabled={!editing && (snapshot?.items.length ?? 0) >= 30}>{editing ? '检查更正影响' : '保存记忆'}</button>
          {editing && <button type="button" onClick={() => { setEditing(null); setDraft(empty); }}>取消编辑</button>}
        </fieldset>
      </form>
      <div className="memory-list" aria-label="本机记忆列表">
        {snapshot?.items.length === 0 && <div className="memory-empty"><span>✧</span><h3>还没有留下记忆</h3><p>你亲自保存的偏好和经历，会出现在这里。</p></div>}
        {snapshot?.items.map(item => <article key={item.id} className="memory-card"><span className="memory-kind">{item.kind === 'preference' ? '我的偏好' : '共同经历'}</span><p className="memory-body">{item.body}</p><small>{item.sourceLabel}<br/>经历日期：{item.eventDate ?? '未指定日期'}<br/>创建：{formatTime(item.createdAt)}<br/>确认／更正：{formatTime(item.confirmedAt)}</small><div><button disabled={busy || !!pending} onClick={() => { setEditing(item); setDraft({ kind: item.kind, body: item.body, eventDate: item.eventDate }); setError(''); }}>更正</button><button disabled={busy || !!pending} onClick={() => snapshot && setPending({ action: 'delete', id: item.id, expectedRevision: item.revision, expectedEpoch: snapshot.contextEpoch, impactIds: [item.id] })}>删除</button></div></article>)}
      </div>
    </div>
    <MemoryPolicyPanel items={snapshot?.items ?? []} epoch={snapshot?.contextEpoch ?? -1} locked={busy || !snapshot || !!pending}/>
    <small className="memory-help">本机明文保存。导出仅包含有效记忆，不包含聊天或密钥；导出文件需要你自行保管。本版不支持导入。</small>
  </section>;
}
