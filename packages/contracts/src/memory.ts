/** Manual memory DTOs and M2 IPC receipts. Model use remains disabled. */
export type MemoryKind = 'preference' | 'experience' | 'task_fact';
export interface MemoryDraft {
  kind: Exclude<MemoryKind, 'task_fact'>;
  body: string;
  eventDate: string | null;
}
export interface MemoryRecord extends MemoryDraft {
  id: string;
  sourceKind: 'user_manual';
  sourceLabel: '用户在记忆面板填写';
  createdAt: number;
  confirmedAt: number;
  updatedAt: number;
  revision: number;
}
const integer = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const incompatible = (): never => { throw new Error('记忆协议不兼容，请更新客户端'); };
function validDate(value: unknown): value is string | null {
  if (value === null) return true;
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const year = Number(value.slice(0, 4)), month = Number(value.slice(5, 7)), day = Number(value.slice(8, 10));
  const leap = year % 400 === 0 || (year % 4 === 0 && year % 100 !== 0);
  return year >= 1 && month >= 1 && month <= 12 && day >= 1 && day <= ([31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1] ?? 0);
}
export function decodeMemory(value: unknown): MemoryRecord {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return incompatible();
  const v = value as Record<string, unknown>;
  if (typeof v.id !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(v.id) ||
      !['preference', 'experience'].includes(v.kind as string) || typeof v.body !== 'string' ||
      [...v.body].length < 1 || [...v.body].length > 200 ||
      /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\ud800-\udfff]/u.test(v.body) ||
      // Match Rust str::trim's Unicode whitespace, not JS's additional BOM removal.
      /^[\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]|[\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]$/u.test(v.body) ||
      v.sourceKind !== 'user_manual' || v.sourceLabel !== '用户在记忆面板填写' || !validDate(v.eventDate) ||
      !integer(v.createdAt) || !integer(v.confirmedAt) || !integer(v.updatedAt) || !integer(v.revision) || v.revision < 1 ||
      v.confirmedAt < v.createdAt || v.updatedAt !== v.confirmedAt || 'deletedAt' in v) return incompatible();
  return { id: v.id, kind: v.kind as MemoryDraft['kind'], body: v.body, sourceKind: v.sourceKind,
    sourceLabel: v.sourceLabel, eventDate: v.eventDate, createdAt: v.createdAt, confirmedAt: v.confirmedAt,
    updatedAt: v.updatedAt, revision: v.revision };
}
export function decodeMemories(value: unknown): MemoryRecord[] {
  if (!Array.isArray(value) || value.length > 30) return incompatible();
  const memories = value.map(decodeMemory);
  if (new Set(memories.map(m => m.id)).size !== memories.length) return incompatible();
  return memories;
}

export interface MemorySnapshot { items: MemoryRecord[]; contextEpoch: number; modelUseEnabled: false }
export interface MemoryReceipt { contextEpoch: number; chatCleared: boolean; notificationsDelivered: boolean }
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return incompatible();
  return value as Record<string, unknown>;
}
export function decodeContextEpoch(value: unknown): number { return integer(value) ? value : incompatible(); }
export function decodeMemorySnapshot(value: unknown): MemorySnapshot {
  const v = object(value);
  if (!integer(v.contextEpoch) || v.modelUseEnabled !== false) return incompatible();
  return { items: decodeMemories(v.items), contextEpoch: v.contextEpoch, modelUseEnabled: false };
}
export function decodeMemoryReceipt(value: unknown): MemoryReceipt {
  const v = object(value);
  if (!integer(v.contextEpoch) || typeof v.chatCleared !== 'boolean' || typeof v.notificationsDelivered !== 'boolean') return incompatible();
  return { contextEpoch: v.contextEpoch, chatCleared: v.chatCleared, notificationsDelivered: v.notificationsDelivered };
}
export function decodeMemoryExport(value: unknown): { status: 'cancelled' } | { status: 'saved'; count: number } {
  const v = object(value);
  if (v.status === 'cancelled') return { status: 'cancelled' };
  if (v.status !== 'saved' || !integer(v.count) || v.count > 30) return incompatible();
  return { status: 'saved', count: v.count };
}
const memoryErrors: Record<string, string> = {
  invalid_input: '仅可保存偏好或经历，正文需1～200字且日期有效。',
  capacity_exceeded: '最多保留30条记忆，请先整理已有内容。',
  selection_too_large: '修改后超出已选记忆预算，请先调整选择。',
  conflict: '条目已更改或删除，请刷新后重新选择；你的编辑内容仍保留。',
  context_changed: '记忆或会话已变化，请刷新后重新操作。',
  storage_unavailable: '本机记忆不可用，请刷新或检查数据目录。',
  confirmation_required: '请先确认停止回复并清空本机全部模型的聊天记录。',
  export_failed: '导出未完成，请选择可写的JSON文件位置重试。',
};
export function memoryErrorMessage(value: unknown): string {
  if (value && typeof value === 'object' && 'code' in value && typeof value.code === 'string') {
    return memoryErrors[value.code] ?? '记忆错误协议不兼容，请更新客户端。';
  }
  return '记忆操作未完成，请刷新重试；若持续失败，请更新客户端。';
}
