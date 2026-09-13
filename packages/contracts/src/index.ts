/** IPC v1. Mirror of companion-core; decode all native responses at the boundary. */
export const PROTOCOL_VERSION = 1;
export const taskStatuses = ['queued', 'waiting_authorization', 'running', 'verifying', 'completed', 'cancel_requested', 'cancelled', 'failed', 'unknown'] as const;
export type TaskStatus = (typeof taskStatuses)[number];
export interface Task {
  id: string;
  title: string;
  status: TaskStatus;
  createdAt: number;
  updatedAt: number;
  revision: number;
}
export interface RuntimeInfo {
  protocolVersion: number;
  appVersion: string;
  runtime: 'desktop' | 'preview';
  persistence: 'sqlite' | 'memory';
  executorAvailable: boolean;
}
export interface CompanionClient {
  getRuntimeInfo(): Promise<RuntimeInfo>;
  listTasks(): Promise<Task[]>;
  createTask(title: string): Promise<Task>;
  cancelTask(id: string): Promise<Task>;
}

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('响应格式不兼容');
  return value as Record<string, unknown>;
}
const integer = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;

export function decodeTask(value: unknown): Task {
  const v = record(value);
  if (typeof v.id !== 'string' || !v.id || typeof v.title !== 'string' ||
      !taskStatuses.includes(v.status as TaskStatus) || !integer(v.createdAt) ||
      !integer(v.updatedAt) || !integer(v.revision)) throw new Error('任务协议不兼容，请更新客户端');
  return { id: v.id, title: v.title, status: v.status as TaskStatus, createdAt: v.createdAt, updatedAt: v.updatedAt, revision: v.revision };
}
export function decodeTasks(value: unknown): Task[] {
  if (!Array.isArray(value)) throw new Error('任务列表格式不兼容');
  return value.map(decodeTask);
}
export function decodeRuntime(value: unknown): RuntimeInfo {
  const v = record(value);
  if (v.protocolVersion !== PROTOCOL_VERSION || typeof v.appVersion !== 'string' ||
      !['desktop', 'preview'].includes(v.runtime as string) ||
      !['sqlite', 'memory'].includes(v.persistence as string) || typeof v.executorAvailable !== 'boolean') {
    throw new Error('客户端与本地服务版本不兼容');
  }
  return v as unknown as RuntimeInfo;
}
export function validateTitle(title: string): string {
  const cleaned = title.trim();
  if (!cleaned || [...cleaned].length > 200) throw new Error('任务标题需要包含 1～200 个字符');
  return cleaned;
}
