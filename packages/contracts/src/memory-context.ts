import { decodeMemories, decodeContextEpoch, type MemoryRecord } from './memory';
export interface MemoryScope { baseUrl: string; model: string }
export interface MemoryPolicy { enabled: boolean; revision: number; selectedIds: string[] }
export interface ContextPreview { scope: MemoryScope; contextEpoch: number; policy: MemoryPolicy; items: MemoryRecord[]; bodyChars: number; contextChars: number }
export interface MemoryUsage { scope: MemoryScope; contextEpoch: number; memories: {id:string;revision:number}[]; bodyChars:number; contextChars:number }
const bad = (): never => { throw Error('记忆上下文协议不兼容，请更新客户端'); };
function object(value: unknown): Record<string, unknown> { if (!value || typeof value !== 'object' || Array.isArray(value)) return bad(); return value as Record<string, unknown>; }
export function sameScope(a: MemoryScope, b: MemoryScope) { return a.baseUrl === b.baseUrl && a.model === b.model; }
export function decodeScope(value: unknown): MemoryScope {
  const v=object(value);
  if (typeof v.baseUrl !== 'string' || !v.baseUrl || v.baseUrl.length>512 || typeof v.model !== 'string' || !v.model || v.model.length>160) return bad();
  return {baseUrl:v.baseUrl,model:v.model};
}
export function decodeContextPreview(value: unknown): ContextPreview {
  const v=object(value), p=object(v.policy), items=decodeMemories(v.items), scope=decodeScope(v.scope);
  const contextEpoch=decodeContextEpoch(v.contextEpoch), revision=decodeContextEpoch(p.revision);
  const selectedIds=p.selectedIds;
  if(typeof p.enabled!=='boolean' || !Array.isArray(selectedIds) || selectedIds.length>5 || selectedIds.length!==items.length || items.some((m,i)=>m.id!==selectedIds[i]) || p.enabled!==!!items.length) return bad();
  const bodyChars=items.reduce((sum,m)=>sum+[...m.body].length,0), contextChars=decodeContextEpoch(v.contextChars);
  if(bodyChars>800 || bodyChars!==v.bodyChars || contextChars<bodyChars || contextChars>20000 || (!items.length && contextChars!==0)) return bad();
  return {scope,contextEpoch,policy:{enabled:p.enabled,revision,selectedIds:items.map(m=>m.id)},items,bodyChars,contextChars};
}
export function decodeMemoryUsage(value: unknown): MemoryUsage {
  const v=object(value), scope=decodeScope(v.scope), contextEpoch=decodeContextEpoch(v.contextEpoch);
  if(!Array.isArray(v.memories) || v.memories.length>5) return bad();
  const memories=v.memories.map(raw=>{const m=object(raw); if(typeof m.id!=='string'||!/^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/i.test(m.id)) return bad(); const revision=decodeContextEpoch(m.revision); if(!revision)return bad(); return {id:m.id,revision};});
  const bodyChars=decodeContextEpoch(v.bodyChars), contextChars=decodeContextEpoch(v.contextChars);
  if(new Set(memories.map(m=>m.id)).size!==memories.length || bodyChars>800 || contextChars<bodyChars || contextChars>20000 || (memories.length===0 ? bodyChars!==0||contextChars!==0 : bodyChars<memories.length))return bad();
  return {scope,contextEpoch,memories,bodyChars,contextChars};
}
