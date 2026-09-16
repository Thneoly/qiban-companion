/** Presentation only: these states never imply microphone capture or task execution. */
export type ConversationPhase = 'idle' | 'waiting' | 'streaming' | 'complete' | 'stopped' | 'error';
export type CompanionState = 'idle' | 'attentive' | 'thinking' | 'responding' | 'pleased' | 'paused' | 'concerned' | 'quiet' | 'hidden';

const conversationStates: Record<ConversationPhase, CompanionState> = {
  idle: 'attentive', waiting: 'thinking', streaming: 'responding', complete: 'pleased', stopped: 'paused', error: 'concerned',
};
export const companionLabels: Record<CompanionState, string> = {
  idle: '在你身边', attentive: '我在这里', thinking: '正在等回复…', responding: '正在回复你',
  pleased: '回复写好了', paused: '先停在这里', concerned: '回复没有完成', quiet: '安静陪伴中', hidden: '已隐藏',
};
export function companionState({ hidden, quiet, open, phase }: { hidden: boolean; quiet: boolean; open: boolean; phase: ConversationPhase }): CompanionState {
  if (hidden) return 'hidden';
  if (quiet) return 'quiet';
  if (!open) return 'idle';
  return conversationStates[phase];
}
