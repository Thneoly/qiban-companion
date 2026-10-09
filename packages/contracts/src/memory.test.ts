import { expect, it } from 'vitest';
import fixture from '../fixtures/manual-memory.json';
import { decodeMemory, decodeMemories, decodeMemorySnapshot, decodeMemoryReceipt, decodeMemoryExport, decodeUsageImpactReport, memoryErrorMessage } from './memory';

it('accepts the same serialized fixture as Rust, preserving source and local date', () => {
  expect(decodeMemory(fixture)).toEqual(fixture);
  expect(decodeMemory({ ...fixture, body: '🌱'.repeat(200), revision: Number.MAX_SAFE_INTEGER })).toBeTruthy();
  expect(decodeMemory({ ...fixture, eventDate: null }).eventDate).toBeNull();
});

it('requires explicit snapshot, commit notification and export outcomes', () => {
  expect(decodeMemorySnapshot({items: [], contextEpoch: 0}).items).toEqual([]);
  expect(() => decodeMemorySnapshot({items: [], contextEpoch: Number.MAX_SAFE_INTEGER + 1})).toThrow();
  expect(() => decodeMemoryReceipt({contextEpoch: 1, chatCleared: true})).toThrow();
  expect(decodeMemoryReceipt({contextEpoch: 1, chatCleared: true, clearedTurns: 2, notificationsDelivered: false}).notificationsDelivered).toBe(false);
  expect(() => decodeMemoryReceipt({contextEpoch: 1, chatCleared: true, clearedTurns: -1, notificationsDelivered: false})).toThrow();
  expect(decodeUsageImpactReport({scopes: [], affectedTurnsTotal: 0}).affectedTurnsTotal).toBe(0);
  expect(decodeUsageImpactReport({scopes: [{scope: {baseUrl: 'https://a', model: 'm'}, affectedTurns: 2, keptTurns: 1}], affectedTurnsTotal: 2}).scopes)
    .toEqual([{scope: {baseUrl: 'https://a', model: 'm'}, affectedTurns: 2, keptTurns: 1}]);
  for (const bad of [
    {scopes: [], affectedTurnsTotal: 7},
    {scopes: [{scope: {baseUrl: '', model: 'm'}, affectedTurns: 1, keptTurns: 0}], affectedTurnsTotal: 1},
    {scopes: [{scope: {baseUrl: 'https://a', model: 'm'}, affectedTurns: 7, keptTurns: 0}], affectedTurnsTotal: 7},
    {scopes: [{scope: {baseUrl: 'https://a', model: 'm'}, affectedTurns: 1.5, keptTurns: 0}], affectedTurnsTotal: 1},
  ]) expect(() => decodeUsageImpactReport(bad)).toThrow();
  expect(decodeMemoryExport({status:'cancelled'})).toEqual({status:'cancelled'});
  expect(() => decodeMemoryExport({status:'saved',count:31})).toThrow();
  expect(memoryErrorMessage({code:'future_error'})).toContain('协议不兼容');
});
it('rejects invalid, untrusted, deleted and imprecise records', () => {
  for (const patch of [
    { body: '🌱'.repeat(201) }, { body: ' ' }, { body: 'x\0' }, { body: '\ud800' },
    { eventDate: '2023-02-29' }, { eventDate: '1900-02-29' }, { eventDate: '0000-01-01' },
    { kind: 'task_fact' }, { sourceKind: 'model' }, { sourceLabel: 'verified' },
    { revision: Number.MAX_SAFE_INTEGER + 1 }, { revision: 0 }, { createdAt: -1 },
    { updatedAt: 999 }, { confirmedAt: 999 }, { deletedAt: 1002 },
  ]) expect(() => decodeMemory({ ...fixture, ...patch })).toThrow();
});
it('does not turn malformed or duplicate lists into empty state', () => {
  expect(decodeMemories([])).toEqual([]);
  for (const input of [null, {}, [fixture, fixture], Array(31).fill(fixture)]) {
    expect(() => decodeMemories(input)).toThrow();
  }
});
