/**
 * Streaming sentence splitter for queued TTS: a presentation-layer decision
 * only. History stores the raw reply text; these sentences are never persisted
 * and carry no replay semantics.
 */
const TERMINATORS = new Set(['。', '！', '？', '；', '!', '?', ';', '…', '\n']);
// ASCII '.' is deliberately absent: "3.14" must never cut.
const SOFT_SEPARATORS = new Set(['，', '、', ',', '：', ':']);
/** Beyond this many code points without a terminator, the tail is cut anyway. */
export const SENTENCE_LIMIT = 80;

function isContent(ch: string): boolean {
  return !/\s/.test(ch) && !TERMINATORS.has(ch) && !SOFT_SEPARATORS.has(ch);
}
function contentCount(text: string): number {
  let count = 0;
  for (const ch of text) if (isContent(ch)) count++;
  return count;
}

/**
 * Feed one stream delta onto the pending buffer and return the new buffer
 * plus every sentence that became complete. Code-point based throughout, so
 * surrogate pairs (emoji) can never be cut in half.
 */
export function feedSentence(
  buffer: string,
  delta: string,
): { buffer: string; sentences: string[] } {
  const chars = Array.from(buffer + delta);
  const sentences: string[] = [];
  let start = 0;
  for (let index = 0; index < chars.length; index++) {
    const ch = chars[index];
    if (ch === undefined || !TERMINATORS.has(ch)) continue;
    const candidate = chars.slice(start, index + 1).join('').trim();
    // A lone terminator (or a run of them) carries no speakable content.
    if (contentCount(candidate) > 0) sentences.push(candidate);
    start = index + 1;
  }
  return { buffer: cutLongTail(chars.slice(start).join(''), sentences), sentences };
}

/** Cut an overlong unpunctuated tail at its last soft separator, else hard. */
function cutLongTail(tail: string, sentences: string[]): string {
  let chars = Array.from(tail);
  while (chars.length > SENTENCE_LIMIT) {
    let cut = -1;
    for (let index = SENTENCE_LIMIT - 1; index >= 0; index--) {
      const ch = chars[index];
      if (ch !== undefined && SOFT_SEPARATORS.has(ch)) {
        cut = index;
        break;
      }
    }
    if (cut === -1) cut = SENTENCE_LIMIT - 1; // hard cut, never mid-surrogate
    const piece = chars.slice(0, cut + 1).join('');
    if (contentCount(piece) > 0) sentences.push(piece);
    chars = chars.slice(cut + 1);
  }
  return chars.join('');
}

/** End of stream: whatever content remains is the final sentence. */
export function flushSentences(buffer: string): string[] {
  const candidate = buffer.trim();
  return contentCount(candidate) > 0 ? [candidate] : [];
}
