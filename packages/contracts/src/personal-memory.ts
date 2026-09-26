/**
 * Personal memory service DTOs (read-only desktop view).
 *
 * Validation is layered to mirror the service's actual guarantees:
 * - schema-v2.sql CHECK constraints are strict invariants and are mirrored
 *   exactly (seq/importance bounds, origin slug, supersededBy ≠ id, the
 *   digit-class timestamp GLOB).
 * - Columns the schema is silent on (project/tag lengths) only get loose
 *   sanity caps: the v1 migration preserved Python-era rows without those
 *   limits, so copying the write-path caps here would misread real data as
 *   "incompatible".
 * - Timestamps deliberately use the GLOB digit classes, not calendar rules:
 *   legacy rows can be GLOB-legal but not real dates, and both the service
 *   and this client compare them as plain strings.
 */
export const personalMemoryKinds = ['fact', 'decision', 'preference', 'project', 'person', 'insight', 'context'] as const;
export type PersonalMemoryKind = (typeof personalMemoryKinds)[number];

export interface PersonalMemoryRecord {
  id: number;
  seq: number;
  type: PersonalMemoryKind;
  project: string | null;
  title: string;
  content: string;
  importance: number;
  createdAt: string;
  updatedAt: string;
  validUntil: string | null;
  supersededBy: number | null;
  contradicts: number | null;
  tags: string[];
  origin: string | null;
}
export interface PersonalMemoryList { count: number; memories: PersonalMemoryRecord[] }
export interface PersonalMemoryDetail { memory: PersonalMemoryRecord; chain: PersonalMemoryRecord[] }
export interface PersonalMemoryStats {
  total: number;
  active: number;
  superseded: number;
  byType: Record<string, number>;
  byProject: Record<string, number>;
}
export interface PersonalMemoryOverview { online: boolean; stats: PersonalMemoryStats | null; serviceUrl: string }

const MAX_SAFE = 9007199254740991;
const incompatible = (): never => { throw new Error('个人记忆协议不兼容，请更新客户端'); };
const object = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return incompatible();
  return value as Record<string, unknown>;
};
const safeInteger = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const length = (v: string): number => [...v].length;
/** Mirrors the schema GLOB digit classes exactly (loose on calendar rules). */
const looseTimestamp = (v: unknown): v is string =>
  typeof v === 'string' && /^\d{4}-[0-1]\d-[0-3]\d [0-2]\d:[0-5]\d:[0-5]\d$/.test(v);
const nullableReference = (v: unknown, id: number): v is number | null =>
  v === null || (safeInteger(v) && (v as number) >= 1 && (v as number) !== id);

export function decodePersonalMemoryRecord(value: unknown): PersonalMemoryRecord {
  const v = object(value);
  if (!safeInteger(v.id) || v.id < 1 ||
      !safeInteger(v.seq) || v.seq < 1 || v.seq > MAX_SAFE ||
      !personalMemoryKinds.includes(v.type as PersonalMemoryKind) ||
      !(v.project === null || (typeof v.project === 'string' && length(v.project) <= 512)) ||
      typeof v.title !== 'string' || length(v.title) < 1 || length(v.title) > 200 || v.title.includes('\0') ||
      typeof v.content !== 'string' || length(v.content) < 1 || length(v.content) > 20000 || v.content.includes('\0') ||
      !safeInteger(v.importance) || v.importance < 1 || v.importance > 5 ||
      !looseTimestamp(v.createdAt) || !looseTimestamp(v.updatedAt) ||
      !(v.validUntil === null || looseTimestamp(v.validUntil)) ||
      !nullableReference(v.supersededBy, v.id as number) || !nullableReference(v.contradicts, v.id as number) ||
      !Array.isArray(v.tags) || v.tags.length > 32 ||
      v.tags.some(tag => typeof tag !== 'string' || length(tag) < 1 || length(tag) > 128 || tag.includes(',')) ||
      !(v.origin === null || (typeof v.origin === 'string' && /^[a-z0-9_-]{1,32}$/.test(v.origin)))) return incompatible();
  return {
    id: v.id as number, seq: v.seq as number, type: v.type as PersonalMemoryKind,
    project: v.project as string | null, title: v.title, content: v.content,
    importance: v.importance as number, createdAt: v.createdAt, updatedAt: v.updatedAt,
    validUntil: v.validUntil as string | null,
    supersededBy: v.supersededBy as number | null, contradicts: v.contradicts as number | null,
    tags: v.tags as string[], origin: v.origin as string | null,
  };
}

export function decodePersonalMemoryList(value: unknown): PersonalMemoryList {
  const v = object(value);
  if (!safeInteger(v.count) || v.count > 50 || !Array.isArray(v.memories)) return incompatible();
  const memories = v.memories.map(decodePersonalMemoryRecord);
  if (v.count !== memories.length || new Set(memories.map(m => m.id)).size !== memories.length) return incompatible();
  return { count: v.count as number, memories };
}

export function decodePersonalMemoryDetail(value: unknown): PersonalMemoryDetail {
  const v = object(value);
  const memory = decodePersonalMemoryRecord(v.memory);
  if (!Array.isArray(v.chain) || v.chain.length < 1 || v.chain.length > 1000) return incompatible();
  const chain = v.chain.map(decodePersonalMemoryRecord);
  const ids = new Set(chain.map(m => m.id));
  if (ids.size !== chain.length || !ids.has(memory.id)) return incompatible();
  return { memory, chain };
}

function counterMap(value: unknown, keyLimit: (key: string) => boolean, maxKeys: number): Record<string, number> {
  const v = object(value);
  const keys = Object.keys(v);
  if (keys.length > maxKeys || keys.some(key => !keyLimit(key) || !safeInteger(v[key]))) return incompatible();
  return Object.fromEntries(keys.map(key => [key, v[key] as number]));
}

export function decodePersonalMemoryStats(value: unknown): PersonalMemoryStats {
  const v = object(value);
  if (!safeInteger(v.total) || !safeInteger(v.active) || !safeInteger(v.superseded) ||
      (v.active as number) + (v.superseded as number) > (v.total as number)) return incompatible();
  const byType = counterMap(v.byType, key => personalMemoryKinds.includes(key as PersonalMemoryKind), 7);
  const byProject = counterMap(v.byProject, key => length(key) >= 1 && length(key) <= 64, 10);
  // Every active row has exactly one kind, so the type counters must sum to
  // the active total; the project view is a top-10 projection and may sum
  // lower. Both are read from one snapshot, so they cannot contradict.
  const typeSum = Object.values(byType).reduce((a, b) => a + b, 0);
  const projectSum = Object.values(byProject).reduce((a, b) => a + b, 0);
  if (typeSum !== v.active || projectSum > v.active) return incompatible();
  return { total: v.total as number, active: v.active as number, superseded: v.superseded as number, byType, byProject };
}

export function decodePersonalMemoryOverview(value: unknown): PersonalMemoryOverview {
  const v = object(value);
  if (typeof v.online !== 'boolean' ||
      !(v.stats === null || (v.online && typeof v.stats === 'object')) ||
      typeof v.serviceUrl !== 'string' || !/^https?:\/\/[!-~]+$/.test(v.serviceUrl)) return incompatible();
  return {
    online: v.online,
    stats: v.stats === null ? null : decodePersonalMemoryStats(v.stats),
    serviceUrl: v.serviceUrl,
  };
}

const personalMemoryErrors: Record<string, string> = {
  service_offline: '个人记忆服务未运行或无法连接，请先启动服务后再刷新。',
  not_found: '未找到该记忆，可能已被删除，请刷新后重试。',
  busy: '个人记忆服务正忙，请稍后重试。',
  schema_mismatch: '个人记忆服务数据库结构不兼容，请升级服务后再使用。',
  capacity: '个人记忆服务容量已达上限，请联系维护。',
  storage_unavailable: '个人记忆服务暂时不可用，请稍后重试。',
  invalid_request: '检索条件不被服务接受，请调整后重试。',
  incompatible: '个人记忆服务响应协议不兼容，请更新客户端或服务。',
};
export function personalMemoryErrorMessage(value: unknown): string {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const code = (value as Record<string, unknown>).code;
    if (typeof code === 'string' && code in personalMemoryErrors) return personalMemoryErrors[code] ?? '个人记忆错误协议不兼容，请更新客户端。';
  }
  return '个人记忆错误协议不兼容，请更新客户端。';
}
