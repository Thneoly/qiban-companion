import { decodeMemories, decodeContextEpoch, type MemoryRecord } from './memory';
import { decodePersonalMemoryRecord, type PersonalMemoryRecord } from './personal-memory';
export interface MemoryScope { baseUrl: string; model: string }
export interface MemoryPolicy { enabled: boolean; revision: number; selectedIds: string[] }
export interface PersonalMemoryInjectionPolicy { enabled: boolean; revision: number; selectedIds: number[] }
export interface PersonalContextFamily {
  status: 'online' | 'offline';
  policy: PersonalMemoryInjectionPolicy;
  /** Active subset of the selection, in saved order; empty while offline. */
  items: PersonalMemoryRecord[];
  /** Selected ids currently superseded/expired/gone — surfaced, never dropped. */
  inactiveSelectedIds: number[];
  bodyChars: number;
  contextChars: number;
}
export interface ContextPreview { scope: MemoryScope; contextEpoch: number; policy: MemoryPolicy; items: MemoryRecord[]; bodyChars: number; contextChars: number; personal: PersonalContextFamily }
export interface PersonalUsageFamily { status: 'sent' | 'offline'; memories: { id: number; seq: number }[]; bodyChars: number; contextChars: number }
export interface MemoryUsage { scope: MemoryScope; contextEpoch: number; memories: {id:string;revision:number}[]; bodyChars:number; contextChars:number; personal: PersonalUsageFamily }
const bad = (): never => { throw Error('记忆上下文协议不兼容，请更新客户端'); };
function object(value: unknown): Record<string, unknown> { if (!value || typeof value !== 'object' || Array.isArray(value)) return bad(); return value as Record<string, unknown>; }
export function sameScope(a: MemoryScope, b: MemoryScope) { return a.baseUrl === b.baseUrl && a.model === b.model; }
export function decodeScope(value: unknown): MemoryScope {
  const v=object(value);
  if (typeof v.baseUrl !== 'string' || !v.baseUrl || v.baseUrl.length>512 || typeof v.model !== 'string' || !v.model || v.model.length>160) return bad();
  return {baseUrl:v.baseUrl,model:v.model};
}
function integer(value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) return bad();
  return value;
}
function idList(value: unknown, max: number): number[] {
  if (!Array.isArray(value) || value.length > max) return bad();
  const ids = value.map(integer);
  if (ids.some(id => id < 1) || new Set(ids).size !== ids.length) return bad();
  return ids;
}
function decodePersonalFamily(value: unknown): PersonalContextFamily {
  const v = object(value), p = object(v.policy);
  if (v.status !== 'online' && v.status !== 'offline') return bad();
  const selectedIds = idList(p.selectedIds, 5);
  if (typeof p.enabled !== 'boolean' || (p.enabled === true) === (selectedIds.length === 0)) return bad();
  const revision = integer(p.revision);
  const items = v.status === 'online' ? (Array.isArray(v.items) ? v.items.map(decodePersonalMemoryRecord) : bad()) : [];
  const inactiveSelectedIds = idList(v.inactiveSelectedIds, 5);
  // items must be an ORDERED SUBSET of selectedIds (active filter can drop
  // entries; offline drops all), and online the active+inactive ids must
  // partition the selection exactly.
  if (v.status === 'offline' && (items.length !== 0 || inactiveSelectedIds.length !== 0)) return bad();
  let cursor = 0;
  for (const item of items) {
    while (cursor < selectedIds.length && selectedIds[cursor] !== item.id) cursor += 1;
    if (cursor === selectedIds.length) return bad();
    cursor += 1;
  }
  if (items.some(item => inactiveSelectedIds.includes(item.id))) return bad();
  if (inactiveSelectedIds.some(id => !selectedIds.includes(id))) return bad();
  if (v.status === 'online' && items.length + inactiveSelectedIds.length !== selectedIds.length) return bad();
  const bodyChars = items.reduce((sum, m) => sum + [...m.content].length, 0);
  const contextChars = integer(v.contextChars);
  // The PREVIEW is a status report and must stay decodable even when
  // service-side content growth pushed the sum past the send budget (the
  // send path rejects that; the usage receipt keeps the strict ≤800 cap).
  // Caps here are sanity bounds: 5 records × the 20000-char content limit.
  if (bodyChars > 100000 || bodyChars !== v.bodyChars || contextChars < bodyChars || contextChars > 200000 || (!items.length && contextChars !== 0)) return bad();
  return { status: v.status, policy: { enabled: p.enabled, revision, selectedIds }, items, inactiveSelectedIds, bodyChars, contextChars };
}
export function decodeContextPreview(value: unknown): ContextPreview {
  const v=object(value), p=object(v.policy), items=decodeMemories(v.items), scope=decodeScope(v.scope);
  const contextEpoch=decodeContextEpoch(v.contextEpoch), revision=decodeContextEpoch(p.revision);
  const selectedIds=p.selectedIds;
  if(typeof p.enabled!=='boolean' || !Array.isArray(selectedIds) || selectedIds.length>5 || selectedIds.length!==items.length || items.some((m,i)=>m.id!==selectedIds[i]) || p.enabled!==!!items.length) return bad();
  const bodyChars=items.reduce((sum,m)=>sum+[...m.body].length,0), contextChars=decodeContextEpoch(v.contextChars);
  if(bodyChars>800 || bodyChars!==v.bodyChars || contextChars<bodyChars || contextChars>20000 || (!items.length && contextChars!==0)) return bad();
  return {scope,contextEpoch,policy:{enabled:p.enabled,revision,selectedIds:items.map(m=>m.id)},items,bodyChars,contextChars,personal:decodePersonalFamily(v.personal)};
}
function decodePersonalUsage(value: unknown): PersonalUsageFamily {
  const v = object(value);
  if (v.status !== 'sent' && v.status !== 'offline') return bad();
  if (!Array.isArray(v.memories) || v.memories.length > 5) return bad();
  const memories = v.memories.map(raw => { const m = object(raw); const id = integer(m.id); const seq = integer(m.seq); if (id < 1 || seq < 1) return bad(); return { id, seq }; });
  const bodyChars = integer(v.bodyChars), contextChars = integer(v.contextChars);
  if (new Set(memories.map(m => m.id)).size !== memories.length
    || bodyChars > 800 || contextChars > 4000 || contextChars < bodyChars
    || (v.status === 'offline' ? memories.length !== 0 || bodyChars !== 0 || contextChars !== 0
        : memories.length === 0 ? bodyChars !== 0 || contextChars !== 0 : bodyChars < memories.length)) return bad();
  return { status: v.status, memories, bodyChars, contextChars };
}
export function decodeMemoryUsage(value: unknown): MemoryUsage {
  const v=object(value), scope=decodeScope(v.scope), contextEpoch=decodeContextEpoch(v.contextEpoch);
  if(!Array.isArray(v.memories) || v.memories.length>5) return bad();
  const memories=v.memories.map(raw=>{const m=object(raw); if(typeof m.id!=='string'||!/^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/i.test(m.id)) return bad(); const revision=decodeContextEpoch(m.revision); if(!revision)return bad(); return {id:m.id,revision};});
  const bodyChars=decodeContextEpoch(v.bodyChars), contextChars=decodeContextEpoch(v.contextChars);
  if(new Set(memories.map(m=>m.id)).size!==memories.length || bodyChars>800 || contextChars<bodyChars || contextChars>20000 || (memories.length===0 ? bodyChars!==0||contextChars!==0 : bodyChars<memories.length))return bad();
  return {scope,contextEpoch,memories,bodyChars,contextChars,personal:decodePersonalUsage(v.personal)};
}
const personalPolicyErrors: Record<string, string> = {
  service_offline: '个人记忆服务未运行，无法核对你选择的内容；可先关闭注入或启动服务后再试。',
  timeout: '个人记忆服务响应超时，请稍后重试。',
  not_found: '所选个人记忆已失效或不存在，请刷新个人记忆后重试。',
  selection_too_large: '个人记忆最多选择5条、合计800字。',
  invalid_input: '选择内容无效，请调整后重试。',
  context_changed: '记忆或会话已变化，请刷新后重新操作。',
  conflict: '记忆版本已变化，请刷新。',
  confirmation_required: '请先确认停止回复并清空本机全部模型的聊天记录。',
  storage_unavailable: '本机记忆不可用，请刷新或检查数据目录。',
};
export function personalPolicyErrorMessage(value: unknown): string {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const code = (value as Record<string, unknown>).code;
    if (typeof code === 'string' && code in personalPolicyErrors) return personalPolicyErrors[code] ?? '个人记忆错误协议不兼容，请更新客户端。';
  }
  return '个人记忆错误协议不兼容，请更新客户端。';
}
