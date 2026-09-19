import { expect, it } from 'vitest';
import fixture from '../fixtures/manual-memory.json';
import { decodeMemory, decodeMemories } from './memory';

it('accepts the same serialized fixture as Rust, preserving source and local date', () => {
  expect(decodeMemory(fixture)).toEqual(fixture);
  expect(decodeMemory({ ...fixture, body: '🌱'.repeat(200), revision: Number.MAX_SAFE_INTEGER })).toBeTruthy();
  expect(decodeMemory({ ...fixture, eventDate: null }).eventDate).toBeNull();
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
