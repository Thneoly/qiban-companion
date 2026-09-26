import { describe, expect, test } from 'vitest';
import fixture from '../fixtures/personal-memory.json';
import {
  decodePersonalMemoryDetail, decodePersonalMemoryList, decodePersonalMemoryOverview,
  decodePersonalMemoryRecord, decodePersonalMemoryStats, personalMemoryErrorMessage,
  type PersonalMemoryRecord,
} from './personal-memory';

const base = {
  id: 1, seq: 1, type: 'decision', project: 'R2R', title: '标题', content: '内容',
  importance: 3, createdAt: '2026-09-25 02:10:12', updatedAt: '2026-09-25 02:10:12',
  validUntil: null, supersededBy: null, contradicts: null, tags: ['a'], origin: 'mcp',
} satisfies PersonalMemoryRecord;
const record = (patch: Partial<Record<string, unknown>> = {}): Record<string, unknown> => ({ ...base, ...patch });

describe('decodePersonalMemoryRecord', () => {
  test('accepts the real-shape fixture', () => {
    const list = decodePersonalMemoryList(fixture);
    expect(list.count).toBe(2);
    expect(list.memories[1]?.tags).toEqual(['memory-service', 'architecture', '老搭档']);
  });
  test('accepts loose-GLOB timestamps and null origin from migrated rows', () => {
    // Schema GLOB digit classes only: legacy rows can be GLOB-legal yet not
    // real calendar dates, and both sides compare them as plain strings.
    const legacy = record({ createdAt: '2026-19-01 29:59:59', updatedAt: '2026-19-01 29:59:59', origin: null });
    expect(decodePersonalMemoryRecord(legacy).origin).toBeNull();
  });
  test.each([
    ['id 0', record({ id: 0 })],
    ['seq beyond safe integer', record({ seq: 9007199254740992 })],
    ['unknown kind', record({ type: 'bogus' })],
    ['importance 0', record({ importance: 0 })],
    ['importance 6', record({ importance: 6 })],
    ['iso timestamp', record({ createdAt: '2026-09-26T01:02:03' })],
    ['short timestamp', record({ updatedAt: '2026-9-6 1:2:3' })],
    ['supersededBy equals id', record({ supersededBy: 1 })],
    ['tag with comma', record({ tags: ['a,b'] })],
    ['origin uppercase', record({ origin: 'MCP' })],
    ['empty title', record({ title: '' })],
    ['oversized title', record({ title: '长'.repeat(201) })],
    ['nul in content', record({ content: 'a\0b' })],
  ])('rejects %s', (_name, value) => {
    expect(() => decodePersonalMemoryRecord(value)).toThrow('个人记忆协议不兼容');
  });
});

describe('decodePersonalMemoryList', () => {
  test('rejects count mismatch and duplicates', () => {
    expect(() => decodePersonalMemoryList({ count: 2, memories: [record()] })).toThrow();
    expect(() => decodePersonalMemoryList({ count: 2, memories: [record(), record()] })).toThrow();
  });
});

describe('decodePersonalMemoryDetail', () => {
  test('validates the chain ordering invariants', () => {
    const detail = decodePersonalMemoryDetail({ memory: record(), chain: [record(), record({ id: 2, supersededBy: null, seq: 2 })] });
    expect(detail.chain.map(m => m.id)).toEqual([1, 2]);
  });
  test('rejects empty chain and chains without the memory', () => {
    expect(() => decodePersonalMemoryDetail({ memory: record(), chain: [] })).toThrow();
    expect(() => decodePersonalMemoryDetail({ memory: record(), chain: [record({ id: 2 })] })).toThrow();
  });
});

describe('decodePersonalMemoryStats', () => {
  const stats = { total: 3, active: 2, superseded: 1, byType: { decision: 2 }, byProject: { '(global)': 1, R2R: 1 } };
  test('accepts a consistent snapshot', () => {
    expect(decodePersonalMemoryStats(stats).active).toBe(2);
  });
  test.each([
    ['active + superseded exceeds total', { ...stats, active: 3 }],
    ['byType sum diverges from active', { ...stats, byType: { decision: 1 } }],
    ['byProject sum exceeds active', { ...stats, byProject: { a: 1, b: 1, c: 1 } }],
    ['unknown kind key', { ...stats, byType: { decision: 1, bogus: 1 } }],
    ['eleven project keys', { ...stats, byProject: Object.fromEntries(Array.from({ length: 11 }, (_, i) => [String(i), 1])) }],
  ])('rejects %s', (_name, value) => {
    expect(() => decodePersonalMemoryStats(value)).toThrow();
  });
});

describe('decodePersonalMemoryOverview', () => {
  test('offline has no stats; online carries them', () => {
    expect(decodePersonalMemoryOverview({ online: false, stats: null, serviceUrl: 'http://127.0.0.1:4322' }).online).toBe(false);
    const online = decodePersonalMemoryOverview({ online: true, stats: { total: 0, active: 0, superseded: 0, byType: {}, byProject: {} }, serviceUrl: 'http://127.0.0.1:4322' });
    expect(online.stats?.total).toBe(0);
  });
  test.each([
    'stats present while offline',
    'bad service url',
  ])('rejects %s', (name) => {
    const value = name === 'stats present while offline'
      ? { online: false, stats: { total: 0, active: 0, superseded: 0, byType: {}, byProject: {} }, serviceUrl: 'http://127.0.0.1:4322' }
      : { online: true, stats: null, serviceUrl: 'ftp://x' };
    expect(() => decodePersonalMemoryOverview(value)).toThrow();
  });
});

describe('personalMemoryErrorMessage', () => {
  test('maps known codes and falls back on protocol mismatch', () => {
    expect(personalMemoryErrorMessage({ code: 'service_offline' })).toContain('未运行');
    expect(personalMemoryErrorMessage({ code: 'surprise' })).toContain('协议不兼容');
    expect(personalMemoryErrorMessage('x')).toContain('协议不兼容');
  });
});
