import { listen } from '@tauri-apps/api/event';
import { nativeDesktop, petAction } from './lib/surface';
import { useCallback, useEffect, useState } from 'react';
import type { RuntimeInfo, Task } from '@companion/contracts';
import { client, errorMessage } from './lib/client';
import { ModelSettings } from './features/settings/ModelSettings';
import { Avatar } from './features/companion/Avatar';
import { TaskPanel } from './features/tasks/TaskPanel';

export default function App() {
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

  return <div className="app-shell">
    <aside className="sidebar">
      <a className="brand" href="#home"><span className="brand-mark">✳</span><span>栖伴<small>COMPANION</small></span></a>
      <div className="nav-label">我们的空间</div>
      <nav aria-label="主导航"><a href="#home" className="nav-link active"><span>⌂</span>相处空间<span className="nav-dot" /></a><a href="#tasks-title" className="nav-link"><span>☷</span>待办手记</a></nav>
      <div className="sidebar-note"><span className="mini-star">✧</span><p>一段陪伴，<br/>从小小的日常开始。</p></div>
      <div className="build-tag"><span className="live-dot"/> 开发预览 <span>v0.1</span></div>
    </aside>
    <main id="home">
      <header className="topbar">{nativeDesktop && <button className="text-button" onClick={() => { void petAction('restore').catch(e => setError(errorMessage(e))); }}>显示桌面角色</button>}<span>个人空间 <span className="slash">/</span> 与栖栖的日常</span><span className="environment"><span className="live-dot"/>{info?.runtime === 'desktop' ? '桌面客户端' : info?.runtime === 'preview' ? '浏览器预览' : '正在连接本地服务…'}</span></header>
      <div className="page-content">
        <div className="page-heading"><div><span className="eyebrow">YOUR EVERYDAY COMPANION</span><h1>今天，也一起慢慢来。</h1><p>想法可以先记下，陪伴可以很简单。</p></div><span className="chapter">01 <span>/ 我们的起点</span></span></div>
        {error && <div className="error-banner" role="alert"><span>{error}</span>{!ready && <button onClick={() => void load()}>重新连接</button>}</div>}
        <div className="hero-grid">
          <section className="companion-card" aria-label="角色相处空间">
            <div className="card-top"><span className="pill"><span className="live-dot"/>在你身边</span><span className="scene-label">一隅安静的小世界</span></div>
            <Avatar/>
            <div className="companion-footer"><div><h2>栖栖 <span>QIQI</span></h2><p>安静陪伴 · 一点点好奇心</p></div><span className="small-badge">互动样机</span></div>
          </section>
          <aside className="today-card"><span className="eyebrow">HERE & NOW</span><h2>从一个小念头开始</h2><p className="today-intro">我们先建立一个可靠的起点：把想做的事记下来，随时回来看看。</p><div className="detail-item"><span className="detail-icon">✎</span><div><strong>待办有迹可循</strong><p>创建、查看与取消</p></div></div><div className="detail-item"><span className="detail-icon">◇</span><div><strong>{info?.persistence === 'sqlite' ? '记录留在这台电脑' : '当前为临时预览'}</strong><p>{info?.persistence === 'sqlite' ? 'SQLite 本地保存，重启可恢复' : '浏览器刷新后，待办会清空'}</p></div></div><div className="connection-note"><span className="outline-dot"/><div><strong>手机接续 · 尚未连接</strong><p>将在后续版本接入账号与设备配对。</p></div></div><p className="preview-note">可在角色气泡中发起临时多轮对话；语音和远程执行尚未接入。</p></aside>
        </div>
        <ModelSettings/>
        <TaskPanel tasks={tasks} busy={busy} ready={ready} onCreate={create} onCancel={cancel}/>
        <footer className="page-footer"><span>栖伴 · 给想法一个停靠的地方</span><span>本地设置 / 模型服务由你选择</span></footer>
      </div>
    </main>
  </div>;
}
