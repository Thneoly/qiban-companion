import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  decodePersonalMemoryDetail, decodePersonalMemoryList, decodePersonalMemoryOverview,
  personalMemoryErrorMessage, personalMemoryKinds,
  type PersonalMemoryDetail, type PersonalMemoryList, type PersonalMemoryOverview, type PersonalMemoryRecord,
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
  const alive = useRef(true), serial = useRef(0), acting = useRef(false);

  const refreshOverview = useCallback(async () => {
    const id = ++serial.current;
    try {
      const next = decodePersonalMemoryOverview(await invoke('personal_memory_overview'));
      if (alive.current && serial.current === id) {
        setOverview(next);
        if (!next.online) { setResults(null); setDetail(null); setExpanded(null); }
        setError('');
      }
    } catch (e) {
      if (alive.current && serial.current === id) setError(personalMemoryErrorMessage(e));
    }
  }, []);

  const search = useCallback(async (terms: { query: string; project: string; kind: string }) => {
    if (!nativeDesktop || acting.current) return;
    acting.current = true; setBusy(true); setError('');
    const id = ++serial.current;
    try {
      const next = decodePersonalMemoryList(await invoke('personal_memory_recall', {
        query: terms.query || null, project: terms.project || null,
        kind: terms.kind || null, limit: SEARCH_LIMIT,
      }));
      if (alive.current && serial.current === id) setResults(next);
    } catch (e) {
      if (alive.current && serial.current === id) {
        // The service may have gone down between overview and search.
        if (e && typeof e === 'object' && (e as { code?: unknown }).code === 'service_offline') {
          setOverview({ online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' });
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
    return () => { alive.current = false; serial.current++; window.removeEventListener('focus', focus); };
  }, [refreshOverview]);

  function submit(event: FormEvent) {
    event.preventDefault();
    void search({ query, project, kind });
  }

  function toggle(id: number) {
    if (expanded === id) { setExpanded(null); setDetail(null); return; }
    setExpanded(id); setDetail(null);
    void loadDetail(id);
  }

  const online = overview?.online === true;
  const offline = overview?.online === false;
  const stats = overview?.stats ?? null;
  return <section className="personal-memory-panel" aria-labelledby="personal-memories-title">
    <div className="personal-memory-heading"><div><span className="eyebrow">ACROSS EVERYTHING WE USE</span><h2 id="personal-memories-title">个人记忆</h2></div><span className="personal-memory-badge">只读 · 独立服务</span></div>
    <p>这里陈列独立个人记忆服务中的档案：七类记忆、取代链与有效窗口，来自 Claude Code 等入口的共同记忆库。本视图只读，不写入、不注入聊天，也不会自动启动服务。</p>
    {!nativeDesktop && <p className="personal-memory-notice">浏览器仅预览界面，不连接个人记忆服务。请在桌面版使用。</p>}
    {nativeDesktop && !overview && <p role="status" className="personal-memory-notice">正在连接个人记忆服务…</p>}
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
            <MemoryStates memory={memory}/>
          </header>
          <h3>{memory.title}</h3>
          <p className="personal-memory-content">{memory.content}</p>
          <footer>
            <span>{memory.project === null ? '全局' : memory.project}</span>
            {memory.tags.length > 0 && <span>{memory.tags.join(' · ')}</span>}
            <span>更新于 {memory.updatedAt}</span>
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
  </section>;
}
