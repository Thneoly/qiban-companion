import { describe, expect, it } from 'vitest';
import { decodeRuntime, decodeTask, validateTitle } from './index';

describe('IPC boundary', () => {
  it('rejects unknown native state instead of silently treating it as completed', () => {
    expect(() => decodeTask({ id: 'x', title: 'x', status: 'success', createdAt: 1, updatedAt: 1, revision: 0 })).toThrow();
    expect(() => decodeRuntime({ protocolVersion: 99 })).toThrow();
  });
  it('requires camelCase fields and valid timestamps', () => {
    const wire = { id: 'x', title: 'x', status: 'queued', createdAt: 1, updatedAt: 1, revision: 0 };
    expect(decodeTask(wire).status).toBe('queued');
    expect(() => decodeTask({ ...wire, createdAt: NaN })).toThrow();
    expect(() => decodeTask({ ...wire, revision: -1 })).toThrow();
  });
  it('counts unicode codepoints consistently with Rust', () => {
    expect(validateTitle('🌿'.repeat(200))).toHaveLength(400);
    expect(() => validateTitle('🌿'.repeat(201))).toThrow();
    expect(() => validateTitle('  ')).toThrow();
  });
});
