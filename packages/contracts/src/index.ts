import { decodeMemoryUsage, type MemoryUsage } from './memory-context';
export * from './memory-context';
/** IPC v3. Mirror of companion-core; decode all native responses at the boundary. */
export const PROTOCOL_VERSION = 3;
export * from './memory';
export * from './execution';
export * from './account';
export * from './personal-memory';
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
  /** At least one explicit executor exists; this never authorizes automatic inbox execution. */
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

export interface ChatConfig {configured:boolean;model:string;maxOutputTokens:number}
export interface VoiceResult {
  requestId: string; transcript: string; reply: string; wav: number[];
  recognitionMs: number; generationMs: number; synthesisMs: number;
  inputSeconds: number; outputSeconds: number; modelTotalTokens: number | null; audioCost: null;
}
export function decodeVoiceResult(value: unknown): VoiceResult {
  const v = record(value);
  if (typeof v.requestId !== 'string' || typeof v.transcript !== 'string' || typeof v.reply !== 'string' ||
      !integer(v.recognitionMs) || !integer(v.generationMs) || !integer(v.synthesisMs) ||
      typeof v.inputSeconds !== 'number' || !Number.isFinite(v.inputSeconds) || v.inputSeconds <= 0 || v.inputSeconds > 30 ||
      typeof v.outputSeconds !== 'number' || !Number.isFinite(v.outputSeconds) || v.outputSeconds <= 0 || v.outputSeconds > 60 ||
      (v.modelTotalTokens !== null && !integer(v.modelTotalTokens)) || v.audioCost !== null ||
      !Array.isArray(v.wav) || v.wav.length < 44 || v.wav.length > 8 * 1024 * 1024 || !v.wav.every(b => integer(b) && b <= 255)) throw Error('语音响应协议不兼容');
  return v as unknown as VoiceResult;
}
export interface ChatDelta {requestId:string;text:string;memoryUsage:MemoryUsage|null}
export interface ChatResult {requestId:string;elapsedMs:number;usage:null|{total_tokens:number|null};historySaved:boolean;memoryUsage:MemoryUsage}
export function decodeChatConfig(value:unknown):ChatConfig {
  const v=record(value);
  if(typeof v.configured!=='boolean'||typeof v.model!=='string'||!integer(v.maxOutputTokens)||v.maxOutputTokens<128||v.maxOutputTokens>8192)throw Error('模型配置协议不兼容');
  return {configured:v.configured,model:v.model,maxOutputTokens:v.maxOutputTokens};
}
export function decodeChatDelta(value:unknown):ChatDelta {
  const v=record(value);
  if(typeof v.requestId!=='string'||typeof v.text!=='string')throw Error('回复协议不兼容');
  return {requestId:v.requestId,text:v.text,memoryUsage:v.memoryUsage===null?null:decodeMemoryUsage(v.memoryUsage)};
}
export function decodeChatResult(value:unknown):ChatResult {
  const v=record(value);
  if(typeof v.requestId!=='string'||!integer(v.elapsedMs)||(v.historySaved!==undefined&&typeof v.historySaved!=='boolean'))throw Error('回复结果不兼容');
  let usage:ChatResult['usage']=null;
  if(v.usage!=null){const u=record(v.usage);if(u.total_tokens!==null&&!integer(u.total_tokens))throw Error('用量协议不兼容');usage={total_tokens:u.total_tokens as number|null};}
  return {requestId:v.requestId,elapsedMs:v.elapsedMs,usage,historySaved:v.historySaved===true,memoryUsage:decodeMemoryUsage(v.memoryUsage)};
}

export interface ModelSettingsConfig {baseUrl:string;model:string;useApiKey:boolean;hasApiKey:boolean;maxOutputTokens:number}
export function decodeModelSettings(value:unknown):ModelSettingsConfig {
  const v=record(value);
  if(typeof v.baseUrl!=='string'||typeof v.model!=='string'||typeof v.useApiKey!=='boolean'||typeof v.hasApiKey!=='boolean'||!integer(v.maxOutputTokens)||v.maxOutputTokens<128||v.maxOutputTokens>8192)throw Error('模型设置协议不兼容');
  return {baseUrl:v.baseUrl,model:v.model,useApiKey:v.useApiKey,hasApiKey:v.hasApiKey,maxOutputTokens:v.maxOutputTokens};
}

export interface ChatTurn {user:string;assistant:string}
export function decodeChatHistory(value:unknown):ChatTurn[] {
  if(!Array.isArray(value)||value.length>6)throw Error('会话记录协议不兼容');
  return value.map(item=>{const v=record(item);if(typeof v.user!=='string'||typeof v.assistant!=='string')throw Error('会话记录协议不兼容');return {user:v.user,assistant:v.assistant};});
}

export * from "./pairing";

export * from "./remote-documents";
