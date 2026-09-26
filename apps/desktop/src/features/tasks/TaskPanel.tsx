import { useState, type FormEvent } from 'react';
import type { Task, TaskStatus } from '@companion/contracts';

const labels: Record<TaskStatus, string> = { queued: '待执行', waiting_authorization: '等待授权', running: '执行中', verifying: '核验中', completed: '已完成', cancel_requested: '正在请求取消', cancelled: '已取消', failed: '失败', unknown: '结果未知' };
interface Props { tasks: Task[]; busy: boolean; ready: boolean; onCreate(title: string): Promise<boolean>; onCancel(id: string): Promise<void> }
export function TaskPanel({ tasks, busy, ready, onCreate, onCancel }: Props) {
  const [title, setTitle] = useState('');
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (await onCreate(title)) setTitle('');
  }
  return <section className="task-panel" aria-labelledby="tasks-title">
    <div className="section-heading"><div><span className="eyebrow">A LITTLE PROGRESS</span><h2 id="tasks-title">把想做的事，先放在这里</h2></div><span className="count">{tasks.filter(t => t.status === 'queued').length} 待办</span></div>
    <form className="task-form" onSubmit={submit}>
      <label className="sr-only" htmlFor="task-title">任务标题</label>
      <input id="task-title" value={title} onChange={e => setTitle(e.target.value)} placeholder="比如：整理今天冒出来的三个想法…" disabled={!ready || busy} />
      <button className="primary" disabled={!ready || busy || !title.trim()}>{busy ? '保存中…' : '记下来'} <span aria-hidden="true">↗</span></button>
    </form>
    <p className="helper">本地待办 · 留在这台设备，不自动上传或合并到账号。这里只记录待办，不会自动读取文件或完成任务。</p>
    <div className="task-list" aria-live="polite">
      {!tasks.length && <div className="empty-state"><span aria-hidden="true">✎</span><div><strong>留一点空间，给下一个好想法</strong><p>添加第一条待办，试试我们的协作起点。</p></div></div>}
      {tasks.map(task => <article className={`task-row ${task.status === 'cancelled' ? 'cancelled' : ''}`} key={task.id}>
        <span className="task-marker" aria-hidden="true">{task.status === 'cancelled' ? '−' : '·'}</span>
        <div className="task-content"><strong>{task.title}</strong><span>{new Date(task.createdAt).toLocaleString('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span></div>
        <span className="status-label">{labels[task.status]}</span>
        {task.status === 'queued' && <button className="text-button" disabled={busy} onClick={() => void onCancel(task.id)} aria-label={`取消任务：${task.title}`}>取消</button>}
      </article>)}
    </div>
  </section>;
}
