import { expect, test } from 'vitest';
import { companionState, type ConversationPhase } from './presentation';

test('hiding, quiet mode and closing override every stale conversation phase', () => {
  for (const phase of ['idle', 'waiting', 'streaming', 'complete', 'stopped', 'error'] as ConversationPhase[]) {
    expect(companionState({ hidden: true, quiet: true, open: true, phase })).toBe('hidden');
    expect(companionState({ hidden: false, quiet: true, open: true, phase })).toBe('quiet');
    expect(companionState({ hidden: false, quiet: false, open: false, phase })).toBe('idle');
  }
});
