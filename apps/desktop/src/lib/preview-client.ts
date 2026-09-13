import { PROTOCOL_VERSION, validateTitle, type CompanionClient, type Task } from '@companion/contracts';

/** Browser-only, in-memory preview. Never used as a fallback for failed native IPC. */
export function createPreviewClient(): CompanionClient {
  const tasks = new Map<string, Task>();
  return {
    async getRuntimeInfo() {
      return { protocolVersion: PROTOCOL_VERSION, appVersion: '0.1.0', runtime: 'preview', persistence: 'memory', executorAvailable: false };
    },
    async listTasks() { return [...tasks.values()].sort((a, b) => b.createdAt - a.createdAt || a.id.localeCompare(b.id)).map(t => ({ ...t })); },
    async createTask(title) {
      const now = Date.now();
      const task: Task = { id: crypto.randomUUID(), title: validateTitle(title), status: 'queued', createdAt: now, updatedAt: now, revision: 0 };
      tasks.set(task.id, task);
      return { ...task };
    },
    async cancelTask(id) {
      const task = tasks.get(id);
      if (!task) throw new Error('未找到任务');
      if (task.status === 'cancelled') return { ...task };
      if (task.status !== 'queued') throw new Error('此任务状态不允许直接取消');
      const cancelled: Task = { ...task, status: 'cancelled', updatedAt: Date.now(), revision: task.revision + 1 };
      tasks.set(id, cancelled);
      return { ...cancelled };
    },
  };
}
