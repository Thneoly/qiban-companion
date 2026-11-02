import { useState, type FormEvent } from 'react';
import type { Task, TaskStatus } from '@companion/contracts';

const labels: Record<TaskStatus, string> = { queued: '待执行', waiting_authorization: '等待授权', running: '执行中', verifying: '核验中', completed: '已完成', cancel_requested: '正在请求取消', cancelled: '已取消', failed: '失败', unknown: '结果未知' };
// 三档分组：待办=queued；进行中=未终态的其余状态（结果未知不算终态，可能仍需核对，保持可见）；
// 已结束=completed/cancelled/failed，默认折叠成计数行。
const doingStatuses: readonly TaskStatus[] = ['waiting_authorization', 'running', 'verifying', 'cancel_requested', 'unknown'];
const endedStatuses: readonly TaskStatus[] = ['completed', 'cancelled', 'failed'];
interface Props { tasks: Task[]; busy: boolean; ready: boolean; onCreate(title: string): Promise<boolean>; onCancel(id: string): Promise<void>; onDelete(id: string): Promise<void> }
export function TaskPanel({ tasks, busy, ready, onCreate, onCancel, onDelete }: Props) {
  const [title, setTitle] = useState('');
  const [showTodo, setShowTodo] = useState(true);
  const [showDoing, setShowDoing] = useState(true);
  const [showEnded, setShowEnded] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (await onCreate(title)) setTitle('');
  }
  function remove(task: Task) {
    // 删除仅移除本机列表行；任务事实沉淀与聊天记录是独立链路，不受影响。
    if (window.confirm(`仅从这台设备的待办列表移除「${task.title}」？任务事实沉淀与聊天记录不受影响。`)) void onDelete(task.id);
  }
  const todoTasks = tasks.filter(t => t.status === 'queued');
  const doingTasks = tasks.filter(t => doingStatuses.includes(t.status));
  const endedTasks = tasks.filter(t => endedStatuses.includes(t.status));
  const visible = [...(showTodo ? todoTasks : []), ...(showDoing ? doingTasks : [])];
  const row = (task: Task, ended: boolean) => <article className={`task-row ${task.status === 'cancelled' ? 'cancelled' : ''}`} key={task.id}>
    <span className="task-marker" aria-hidden="true">{task.status === 'cancelled' ? '−' : '·'}</span>
    <div className="task-content"><strong>{task.title}</strong><span>{new Date(task.createdAt).toLocaleString('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span></div>
    <span className="status-label">{labels[task.status]}</span>
    {task.status === 'queued' && <button className="text-button" disabled={busy} onClick={() => void onCancel(task.id)} aria-label={`取消任务：${task.title}`}>取消</button>}
    {ended && <button className="text-button" disabled={busy} onClick={() => remove(task)} aria-label={`删除任务：${task.title}`}>删除</button>}
  </article>;
  return <section className="task-panel" aria-labelledby="tasks-title">
    <div className="section-heading"><div><span className="eyebrow">A LITTLE PROGRESS</span><h2 id="tasks-title">把想做的事，先放在这里</h2></div><span className="count">{todoTasks.length} 待办</span></div>
    <form className="task-form" onSubmit={submit}>
      <label className="sr-only" htmlFor="task-title">任务标题</label>
      <input id="task-title" value={title} onChange={e => setTitle(e.target.value)} placeholder="比如：整理今天冒出来的三个想法…" disabled={!ready || busy} />
      <button className="primary" disabled={!ready || busy || !title.trim()}>{busy ? '保存中…' : '记下来'} <span aria-hidden="true">↗</span></button>
    </form>
    <p className="helper">本地待办 · 留在这台设备，不自动上传或合并到账号。这里只记录待办，不会自动读取文件或完成任务。</p>
    <div className="task-filters" role="group" aria-label="按状态筛选待办">
      <button className={showTodo ? 'on' : ''} aria-pressed={showTodo} onClick={() => setShowTodo(v => !v)}>待办 {todoTasks.length}</button>
      <button className={showDoing ? 'on' : ''} aria-pressed={showDoing} onClick={() => setShowDoing(v => !v)}>进行中 {doingTasks.length}</button>
      <button className={showEnded ? 'on' : ''} aria-pressed={showEnded} disabled={!endedTasks.length} onClick={() => setShowEnded(v => !v)}>已结束 {endedTasks.length}</button>
    </div>
    <div className="task-list" aria-live="polite">
      {!tasks.length && <div className="empty-state"><span aria-hidden="true">✎</span><div><strong>留一点空间，给下一个好想法</strong><p>添加第一条待办，试试我们的协作起点。</p></div></div>}
      {!!tasks.length && !visible.length && !showEnded && <p className="task-filtered">当前筛选下没有显示中的待办。</p>}
      {visible.map(task => row(task, false))}
      {!!endedTasks.length && <button className="task-fold" aria-expanded={showEnded} onClick={() => setShowEnded(v => !v)}>{showEnded ? '收起' : '展开'}已结束的 {endedTasks.length} 条</button>}
      {showEnded && endedTasks.map(task => row(task, true))}
    </div>
  </section>;
}
