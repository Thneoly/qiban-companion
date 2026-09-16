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

export interface ChatConfig {configured:boolean;model:string}
export interface ChatDelta {requestId:string;text:string}
export interface ChatResult {requestId:string;elapsedMs:number;usage:null|{total_tokens:number|null}}
export function decodeChatConfig(value:unknown):ChatConfig {
  const v=record(value);
  if(typeof v.configured!=='boolean'||typeof v.model!=='string')throw Error('模型配置协议不兼容');
  return {configured:v.configured,model:v.model};
}
export function decodeChatDelta(value:unknown):ChatDelta {
  const v=record(value);
  if(typeof v.requestId!=='string'||typeof v.text!=='string')throw Error('回复协议不兼容');
  return {requestId:v.requestId,text:v.text};
}
export function decodeChatResult(value:unknown):ChatResult {
  const v=record(value);
  if(typeof v.requestId!=='string'||!integer(v.elapsedMs))throw Error('回复结果不兼容');
  let usage:ChatResult['usage']=null;
  if(v.usage!=null){const u=record(v.usage);if(u.total_tokens!==null&&!integer(u.total_tokens))throw Error('用量协议不兼容');usage={total_tokens:u.total_tokens as number|null};}
  return {requestId:v.requestId,elapsedMs:v.elapsedMs,usage};
}

export interface ModelSettingsConfig {baseUrl:string;model:string;useApiKey:boolean;hasApiKey:boolean}
export function decodeModelSettings(value:unknown):ModelSettingsConfig {
  const v=record(value);
  if(typeof v.baseUrl!=='string'||typeof v.model!=='string'||typeof v.useApiKey!=='boolean'||typeof v.hasApiKey!=='boolean')throw Error('模型设置协议不兼容');
  return {baseUrl:v.baseUrl,model:v.model,useApiKey:v.useApiKey,hasApiKey:v.hasApiKey};
}
