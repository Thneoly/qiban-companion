import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  decodeContextPreview, decodeMemoryReceipt,
  decodePersonalMemoryDetail, decodePersonalMemoryList, decodePersonalMemoryOverview,
  personalMemoryErrorMessage, personalPolicyErrorMessage, personalMemoryKinds,
  type ContextPreview, type PersonalMemoryDetail, type PersonalMemoryList, type PersonalMemoryOverview, type PersonalMemoryRecord,
} from '@companion/contracts';
import { nativeDesktop } from '../../lib/surface';
import './personal-memory.css';

const SEARCH_LIMIT = 20;
const kindLabels: Record<string, string> = {
  fact: '事实', decision: '决策', preference: '偏好', project: '项目',
  person: '人物', insight: '洞察', context: '情境',
};
/** Expired rows are compared as plain UTC strings, exactly like the service. */
const utcStamp = () => new Date().toISOString().slice(0, 19).replace('T', ' ');
/** The service stores UTC text; show it in the user's local time. */
const formatStamp = (value: string) => {
  const parsed = new Date(`${value.replace(' ', 'T')}Z`);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
};
const stateOf = (memory: PersonalMemoryRecord): '' | 'superseded' | 'expired' => {
  if (memory.supersededBy !== null) return 'superseded';
  if (memory.validUntil !== null && memory.validUntil <= utcStamp()) return 'expired';
  return '';
};
const stateLabel = { superseded: '已被取代', expired: '已过期' } as const;

function MemoryStates({ memory }: { memory: PersonalMemoryRecord }) {
  const state = stateOf(memory);
  if (!state) return null;
  return <span className={`personal-memory-state personal-memory-state-${state}`}>{stateLabel[state]}</span>;
}

export function PersonalMemoryPanel() {
  const [overview, setOverview] = useState<PersonalMemoryOverview | null>(null);
  const [results, setResults] = useState<PersonalMemoryList | null>(null);
  const [detail, setDetail] = useState<PersonalMemoryDetail | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [query, setQuery] = useState('');
  const [project, setProject] = useState('');
  const [kind, setKind] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  // Injection policy family (per current model scope, from chat_context_preview).
  const [preview, setPreview] = useState<ContextPreview | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [ids, setIds] = useState<number[]>([]);
  const [injectionBusy, setInjectionBusy] = useState(false);
  const [injectionError, setInjectionError] = useState('');
  const [confirming, setConfirming] = useState(false);
  const injectionSerial = useRef(0);
  const injectionActing = useRef(false);
  // Separate serial spaces: a search must never invalidate a concurrent
  // overview refresh (and vice versa).
  const alive = useRef(true), overviewSerial = useRef(0), searchSerial = useRef(0), acting = useRef(false);

  const refreshOverview = useCallback(async () => {
    const id = ++overviewSerial.current;
    try {
      const next = decodePersonalMemoryOverview(await invoke('personal_memory_overview'));
      if (alive.current && overviewSerial.current === id) {
        setOverview(next);
        if (!next.online) { setResults(null); setDetail(null); setExpanded(null); }
        setError('');
      }
    } catch (e) {
      if (alive.current && overviewSerial.current === id) setError(personalMemoryErrorMessage(e));
    }
  }, []);

  const search = useCallback(async (terms: { query: string; project: string; kind: string }) => {
    if (!nativeDesktop || acting.current) return;
    acting.current = true; setBusy(true); setError('');
    const id = ++searchSerial.current;
    try {
      const next = decodePersonalMemoryList(await invoke('personal_memory_recall', {
        query: terms.query || null, project: terms.project || null,
        kind: terms.kind || null, limit: SEARCH_LIMIT,
      }));
      if (alive.current && searchSerial.current === id) setResults(next);
    } catch (e) {
      if (alive.current && searchSerial.current === id) {
        // The service may have gone down between overview and search; keep
        // the address it reported instead of hardcoding one.
        if (e && typeof e === 'object' && (e as { code?: unknown }).code === 'service_offline') {
          setOverview(previous => previous
            ? { ...previous, online: false, stats: null }
            : { online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' });
          setResults(null);
        } else setError(personalMemoryErrorMessage(e));
      }
    } finally {
      acting.current = false;
      if (alive.current) setBusy(false);
    }
  }, []);

  const loadDetail = useCallback(async (id: number) => {
    if (acting.current) return;
    acting.current = true; setBusy(true); setError('');
    try {
      const next = decodePersonalMemoryDetail(await invoke('personal_memory_detail', { id }));
      if (alive.current) setDetail(next);
    } catch (e) {
      if (alive.current) setError(personalMemoryErrorMessage(e));
    } finally {
      acting.current = false;
      if (alive.current) setBusy(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    if (nativeDesktop) void refreshOverview();
    const focus = () => { if (nativeDesktop && !acting.current) void refreshOverview(); };
    window.addEventListener('focus', focus);
    return () => {
      alive.current = false;
      overviewSerial.current++;
      searchSerial.current++;
      injectionSerial.current++;
      window.removeEventListener('focus', focus);
    };
  }, [refreshOverview]);

  const refreshInjection = useCallback(async () => {
    const id = ++injectionSerial.current;
    try {
      const next = decodeContextPreview(await invoke('chat_context_preview'));
      if (alive.current && injectionSerial.current === id) {
        setPreview(next);
        setEnabled(next.personal.policy.enabled);
        setIds(next.personal.policy.selectedIds);
        setInjectionError('');
      }
    } catch (e) {
      if (alive.current && injectionSerial.current === id) setInjectionError(personalMemoryErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    if (nativeDesktop) void refreshInjection();
    // The injection policy shares the global context epoch: any memory-changed
    // event or window focus can make the cached scope/epoch/revision stale,
    // and a stale save fails with "请刷新" — so refresh alongside overview.
    const focus = () => { if (nativeDesktop && !injectionActing.current) void refreshInjection(); };
    window.addEventListener('focus', focus);
    let unlisten: (() => void) | undefined;
    let disposed = false;
    if (nativeDesktop) void import('@tauri-apps/api/event').then(({ listen }) =>
      listen('memory-changed', () => { if (!injectionActing.current) void refreshInjection(); })
    ).then(remove => { if (disposed) remove?.(); else unlisten = remove; }).catch(() => {});
    return () => {
      disposed = true;
      injectionSerial.current++;
      unlisten?.();
      window.removeEventListener('focus', focus);
    };
  }, [refreshInjection]);

  function toggleId(id: number) {
    setIds(previous => previous.includes(id) ? previous.filter(x => x !== id) : [...previous, id]);
  }

  async function saveInjection(restartConversation: boolean) {
    if (injectionActing.current || !preview) return;
    injectionActing.current = true;
    setInjectionBusy(true); setInjectionError(''); setConfirming(false);
    try {
      decodeMemoryReceipt(await invoke('personal_memory_policy_set', { request: {
        expectedScope: preview.scope, expectedEpoch: preview.contextEpoch,
        expectedRevision: preview.personal.policy.revision,
        enabled, selectedIds: enabled ? ids : [], restartConversation,
      }}));
      await refreshInjection();
    } catch (e) {
      if (alive.current) setInjectionError(personalPolicyErrorMessage(e));
    } finally {
      injectionActing.current = false;
      if (alive.current) setInjectionBusy(false);
    }
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    void search({ query, project, kind });
  }

  function toggle(id: number) {
    if (expanded === id) { setExpanded(null); setDetail(null); return; }
    setExpanded(id); setDetail(null);
    void loadDetail(id);
  }

  const injection = preview?.personal;
  const resultsById = new Map((results?.memories ?? []).map(m => [m.id, m]));
  const chosen = enabled ? ids : [];
  const chars = chosen.reduce((sum, id) => sum + [...(resultsById.get(id)?.content ?? preview?.personal.items.find(m => m.id === id)?.content ?? '')].length, 0);
  const removing = !!preview && preview.personal.policy.selectedIds.some(id => !chosen.includes(id));
  const changed = !!preview && (enabled !== preview.personal.policy.enabled || ids.join(',') !== preview.personal.policy.selectedIds.join(','));

  const online = overview?.online === true;
  const offline = overview?.online === false;
  const stats = overview?.stats ?? null;
  return <section className="personal-memory-panel" aria-labelledby="personal-memories-title">
    <div className="personal-memory-heading"><div><span className="eyebrow">ACROSS EVERYTHING WE USE</span><h2 id="personal-memories-title">个人记忆</h2></div><span className="personal-memory-badge">只读 · 独立服务</span></div>
    <p>这里陈列独立个人记忆服务中的档案：七类记忆、取代链与有效窗口，来自 Claude Code 等入口的共同记忆库。本视图只读，不写入、不注入聊天，也不会自动启动服务。</p>
    {!nativeDesktop && <p className="personal-memory-notice">浏览器仅预览界面，不连接个人记忆服务。请在桌面版使用。</p>}
    {nativeDesktop && !overview && !error && <p role="status" className="personal-memory-notice">正在连接个人记忆服务…</p>}
    {nativeDesktop && !overview && error && <div className="personal-memory-offline" role="status">
      <strong>暂时无法读取个人记忆服务状态</strong>
      <p>{error}</p>
      <button disabled={busy} onClick={() => void refreshOverview()}>重新连接</button>
    </div>}
    {nativeDesktop && offline && <div className="personal-memory-offline" role="status">
      <strong>个人记忆服务未运行</strong>
      <p>服务地址：<code>{overview?.serviceUrl}</code>。可先在仓库运行 <code>npm run memory:serve</code>（或运行已部署的 <code>memory-service.exe serve</code>）再回来刷新。未连接时不会显示示例数据。</p>
      <button disabled={busy} onClick={() => void refreshOverview()}>重新连接</button>
    </div>}
    {nativeDesktop && online && <div className="personal-memory-toolbar">
      <span>{stats ? `共 ${stats.total} 条 · 活跃 ${stats.active} · 已取代 ${stats.superseded}` : '服务在线，统计暂不可用'}</span>
      {stats && Object.entries(stats.byProject).sort((a, b) => b[1] - a[1]).slice(0, 3)
        .map(([name, count]) => <span key={name} className="personal-memory-chip">{name === '(global)' ? '全局' : name} {count}</span>)}
      <button disabled={busy} onClick={() => { void refreshOverview(); if (results) void search({ query, project, kind }); }}>刷新</button>
    </div>}
    {error && <p role="alert" className="personal-memory-error">{error}</p>}
    {nativeDesktop && online && <form className="personal-memory-form" onSubmit={submit}>
      <label>关键词<input value={query} maxLength={200} placeholder="标题、内容或标签" onChange={e => setQuery(e.target.value)} /></label>
      <label>类型<select value={kind} onChange={e => setKind(e.target.value)}>
        <option value="">全部类型</option>
        {personalMemoryKinds.map(value => <option key={value} value={value}>{kindLabels[value]}</option>)}
      </select></label>
      <label>项目<input value={project} maxLength={64} placeholder="填写后同时包含全局记忆" onChange={e => setProject(e.target.value)} /></label>
      <button type="submit" disabled={busy}>检索</button>
      <button type="button" disabled={busy} onClick={() => { setQuery(''); setProject(''); setKind(''); void search({ query: '', project: '', kind: '' }); }}>重置</button>
      <span className="personal-memory-help">最多显示 {SEARCH_LIMIT} 条，按重要度与更新时间排序。</span>
    </form>}
    {nativeDesktop && online && results && results.count === 0 && <div className="personal-memory-empty"><span aria-hidden="true">✧</span><strong>没有匹配的记忆</strong><p>换个关键词，或清除筛选条件后再试。</p></div>}
    {nativeDesktop && online && results && results.count > 0 && <ul className="personal-memory-list" aria-label="个人记忆检索结果">
      {results.memories.map(memory => <li key={memory.id}>
        <article className="personal-memory-card">
          <header>
            <span className="personal-memory-kind">{kindLabels[memory.type] ?? memory.type}</span>
            <span className="personal-memory-importance">重要度 {memory.importance}/5</span>
          </header>
          <h3>{memory.title}</h3>
          <p className="personal-memory-content">{memory.content}</p>
          <footer>
            <span>{memory.project === null ? '全局' : memory.project}</span>
            {memory.tags.length > 0 && <span>{memory.tags.join(' · ')}</span>}
            <span>更新于 {formatStamp(memory.updatedAt)}</span>
          </footer>
          <button className="personal-memory-detail-toggle" aria-expanded={expanded === memory.id} disabled={busy} onClick={() => toggle(memory.id)}>
            {expanded === memory.id ? '收起详情' : '查看详情'}
          </button>
          {expanded === memory.id && (detail
            ? <div className="personal-memory-detail" aria-live="polite">
                <h4>取代链（最旧在前）</h4>
                <ol>
                  {detail.chain.map(item => <li key={item.id} aria-current={item.id === memory.id ? 'true' : undefined}>
                    <span className="personal-memory-chain-title">#{item.id} {item.title}</span>
                    {item.id === memory.id
                      ? <span className="personal-memory-state personal-memory-state-current">当前查看</span>
                      : <MemoryStates memory={item}/>}
                    {item.id !== memory.id && stateOf(item) === '' && <span className="personal-memory-state personal-memory-state-current">当前生效</span>}
                    {item.supersededBy !== null && <span className="personal-memory-next">→ 被 #{item.supersededBy} 取代</span>}
                  </li>)}
                </ol>
              </div>
            : <p role="status" className="personal-memory-notice">正在读取取代链…</p>)}
        </article>
      </li>)}
    </ul>}
    {nativeDesktop && injection && <div className="personal-memory-injection">
      <h3>让交流用上这些个人记忆</h3>
      <p className="personal-memory-help">为当前模型勾选发送时携带的个人记忆；按勾选顺序注入，与应用记忆分开计数（各 5 条 / 800 字）。发送前可在聊天气泡预览，发送后回执如实记录。</p>
      <p>当前服务：{preview!.scope.baseUrl} · 模型：{preview!.scope.model} · 已保存状态：{injection.policy.enabled ? `启用 · ${injection.policy.selectedIds.length}条` : '关闭'}</p>
      {!online && <p role="status" className="personal-memory-notice">个人记忆服务未连接：无法新增或保留勾选（需核对内容与预算）；可移除全部勾选或关闭注入，保存时会如实提示。</p>}
      <fieldset disabled={injectionBusy} className="personal-memory-choice-set">
        <label className="personal-memory-choice">
          <input type="checkbox" checked={enabled} onChange={e => { setEnabled(e.target.checked); if (!e.target.checked) setIds([]); }}/>
          <span>允许此模型使用所选个人记忆（发送时携带）</span>
        </label>
        {injection.policy.selectedIds.length > 0 && <div className="personal-memory-choice-list">
          {injection.policy.selectedIds.map(id => <label key={id} className="personal-memory-choice">
            <input type="checkbox" checked={ids.includes(id)} disabled={!enabled} onChange={() => toggleId(id)}/>
            <span>#{id}{resultsById.get(id) ? ` ${resultsById.get(id)!.title}` : online ? '' : '（离线中，仅显编号）'}</span>
          </label>)}
        </div>}
        {online && results && results.count > 0 && <div className="personal-memory-choice-list">
          {results.memories.filter(m => !injection.policy.selectedIds.includes(m.id)).map(m => <label key={m.id} className="personal-memory-choice">
            <input type="checkbox" checked={ids.includes(m.id)} disabled={!enabled} onChange={() => toggleId(m.id)}/>
            <span>#{m.id} {m.title}（{kindLabels[m.type] ?? m.type} · {m.project ?? '全局'}）</span>
          </label>)}
        </div>}
      </fieldset>
      <p className="personal-memory-help">{chosen.length} / 5 条 · {chars} / 800 字 · 按勾选顺序发送{online ? '' : ' · 离线时字数为已显示内容的下限'}</p>
      {injectionError && <p role="alert" className="personal-memory-error">{injectionError}</p>}
      {confirming && <div className="personal-memory-confirm" role="alertdialog" aria-labelledby="personal-injection-confirm-title" aria-describedby="personal-injection-confirm-description">
        <h4 id="personal-injection-confirm-title">确认收回个人记忆使用</h4>
        <p id="personal-injection-confirm-description">移除选择会停止正在生成的回复，并清空这台电脑上<strong>全部模型的聊天记录</strong>。个人记忆条目仍保留在服务中；已经发给服务商的内容不能撤回。</p>
        <button autoFocus disabled={injectionBusy} onClick={() => void saveInjection(true)}>确认收回并清空聊天</button>
        <button disabled={injectionBusy} onClick={() => setConfirming(false)}>返回，不修改</button>
      </div>}
      <button disabled={injectionBusy || !online || chosen.length > 5 || chars > 800 || !changed} onClick={() => { if (removing) setConfirming(true); else void saveInjection(false); }}>保存选择</button>
      {!online && <p className="personal-memory-help">服务未连接时保存不可用；如需临时停用注入，请启动服务后操作。</p>}
    </div>}
  </section>;
}
