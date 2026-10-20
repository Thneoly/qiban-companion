import { listen } from '@tauri-apps/api/event';
import { nativeDesktop, petAction } from './lib/surface';
import { useCallback, useEffect, useState } from 'react';
import type { RuntimeInfo, Task } from '@companion/contracts';
import { client, errorMessage } from './lib/client';
import { MemoryPanel } from './features/memory/MemoryPanel';
import { ModelSettings } from './features/settings/ModelSettings';
import { VoiceLab } from './features/voice/VoiceLab';
import { Avatar } from './features/companion/Avatar';
import { ExecutionPanel } from './features/tasks/ExecutionPanel';
import { TaskPanel } from './features/tasks/TaskPanel';
import { AccountPanel } from './features/account/AccountPanel';
import { PersonalMemoryPanel } from './features/personal-memory/PersonalMemoryPanel';
import { RemindersPanel } from './features/reminders/RemindersPanel';

const sections = [
  ['home', '日常', '今天，也一起慢慢来。', '和栖栖聊聊，或记下一件小事。'],
  ['tasks', '文档', '一起完成一件事', '选定文档、核对预览，再决定是否保存。'],
  ['memories', '记忆', '我们记住的事', '记忆由你决定，随时查看、更正或删除。'],
  ['devices', '设备', '把陪伴带在身边', '登录同一账号，连接手机并核对共享内容。'],
  ['settings', '设置', '按你的习惯相处', '模型、声音与服务配置集中在这里。'],
] as const;
type Section = typeof sections[number][0];
function sectionFromHash(): Section {
  const hash = location.hash.slice(1);
  if (hash.includes('memor')) return 'memories';
  if (hash.includes('account') || hash === 'devices') return 'devices';
  if (hash.includes('settings') || hash === 'voice') return 'settings';
  if (hash.includes('execution') || hash === 'tasks') return 'tasks';
  return 'home';
}
export default function App() {
  const [section, setSection] = useState<Section>(sectionFromHash);
  const navigate = useCallback((value: Section) => { location.hash = value; setSection(value); }, []);
  useEffect(() => {
    const changed = () => setSection(sectionFromHash());
    window.addEventListener('hashchange', changed);
    const removers: (() => void)[] = []; let disposed = false;
    if (nativeDesktop) for (const [event, target] of [['model-settings-open', 'settings'], ['memory-open', 'memories']] as const) {
      void listen(event, () => navigate(target)).then(remove => { if (disposed) remove(); else removers.push(remove); }).catch(() => setError('导航通知连接失败，可使用左侧入口。'));
    }
    return () => { disposed = true; window.removeEventListener('hashchange', changed); removers.forEach(remove => remove()); };
  }, [navigate]);
  const selected = sections.find(item => item[0] === section)!;
  const [info, setInfo] = useState<RuntimeInfo | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const load = useCallback(async () => {
    setReady(false); setError(null);
    try {
      const runtime = await client.getRuntimeInfo();
      const items = await client.listTasks();
      setInfo(runtime); setTasks(items); setReady(true);
    } catch (e) { setError(errorMessage(e)); }
  }, []);
  useEffect(() => {
    void load();
    let disposed = false;
    let remove: (() => void) | undefined;
    if (nativeDesktop) void listen('panel-refresh', () => { void load(); }).then(unlisten => {
      if (disposed) unlisten(); else remove = unlisten;
    }).catch(e => setError(errorMessage(e)));
    return () => { disposed = true; remove?.(); };
  }, [load]);

  async function create(title: string) {
    setBusy(true); setError(null);
    try {
      const task = await client.createTask(title);
      setTasks(items => [task, ...items]);
      return true;
    } catch (e) { setError(errorMessage(e)); return false; }
    finally { setBusy(false); }
  }
  async function cancel(id: string) {
    setBusy(true); setError(null);
    try {
      const updated = await client.cancelTask(id);
      setTasks(items => items.map(task => task.id === id ? updated : task));
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }

  return <div className="app-shell workspace-shell">
    <aside className="sidebar">
      <a className="brand" href="#home"><span className="brand-mark">✳</span><span>栖伴<small>COMPANION</small></span></a>
      <nav aria-label="主导航">{sections.map(([id, label]) => <a key={id} href={`#${id}`} className={`nav-link ${section === id ? 'active' : ''}`} aria-current={section === id ? 'page' : undefined}>{label}</a>)}</nav>
      <div className="sidebar-note"><p>陪伴留在桌面，<br/>琐事放在这里。</p></div>
      <button className="workspace-chat-link" onClick={() => void petAction('open_chat').catch(e => setError(errorMessage(e)))}>与栖栖对话 ↗</button>
    </aside>
    <main id="home">
      <header className="topbar"><span>我们的空间 / {selected[1]}</span><span className="environment">{info?.runtime === 'desktop' ? '桌面客户端' : info?.runtime === 'preview' ? '浏览器预览' : '正在连接…'}</span></header>
      <div className="page-content">
        <div className="page-heading"><div><h1>{selected[2]}</h1><p>{selected[3]}</p></div>{nativeDesktop && <button className="text-button" onClick={() => void petAction('restore').catch(e => setError(errorMessage(e)))}>显示桌面角色</button>}</div>
        {error && <div className="error-banner" role="alert"><span>{error}</span>{!ready && <button onClick={() => void load()}>重新连接</button>}</div>}
        <section hidden={section !== 'home'} aria-label="日常空间">
          <div className="daily-welcome"><Avatar/><div><span className="eyebrow">和栖栖在一起</span><h2>有话，慢慢说。</h2><p>打开宽敞的对话空间，读一段回复，也写下你的想法。</p><button className="workspace-primary" onClick={() => void petAction('open_chat').catch(e => setError(errorMessage(e)))}>打开文字对话 ↗</button><p className="workspace-hint">也可以轻点桌面角色，用语音聊聊。</p></div></div>
          <TaskPanel tasks={tasks} busy={busy} ready={ready} onCreate={create} onCancel={cancel}/>
          <RemindersPanel/>
          <p className="workspace-hint">{info?.persistence === 'sqlite' ? '待办保存在这台电脑，重开可恢复。' : '浏览器刷新后，待办会清空'}</p>
        </section>
        <section hidden={section !== 'tasks'} aria-label="文档空间"><ExecutionPanel/></section>
        <section hidden={section !== 'memories'} aria-label="记忆空间"><MemoryPanel/><PersonalMemoryPanel/></section>
        <section hidden={section !== 'devices'} aria-label="设备空间"><AccountPanel/></section>
        <section hidden={section !== 'settings'} aria-label="设置空间"><ModelSettings/><VoiceLab/></section>
        <footer className="page-footer"><span>栖伴 · 给日常一点陪伴</span><span>模型服务由你选择</span></footer>
      </div>
    </main>
  </div>;
}
