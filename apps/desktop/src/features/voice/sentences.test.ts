import { expect, test } from 'vitest';
import { feedSentence, flushSentences, SENTENCE_LIMIT } from './sentences';

test('terminators split inside one delta and across deltas', () => {
  expect(feedSentence('', '你好。最近怎么样？')).toEqual({
    buffer: '',
    sentences: ['你好。', '最近怎么样？'],
  });
  const first = feedSentence('', '今天天气');
  expect(first.sentences).toEqual([]);
  expect(feedSentence(first.buffer, '不错！').sentences).toEqual(['今天天气不错！']);
});

test('consecutive terminators collapse and lone punctuation is dropped', () => {
  expect(feedSentence('', '真的吗！！').sentences).toEqual(['真的吗！']);
  expect(feedSentence('', '！！！')).toEqual({ buffer: '', sentences: [] });
  expect(feedSentence('', ' \n ').sentences).toEqual([]);
  const after = feedSentence('好', '。\n下一句');
  expect(after.sentences).toEqual(['好。']);
  expect(after.buffer).toBe('下一句');
});

test('ascii dot never cuts decimals or versions', () => {
  const step = feedSentence('', '半径是3.14厘米，版本2.10。');
  expect(step.sentences).toEqual(['半径是3.14厘米，版本2.10。']);
  expect(step.buffer).toBe('');
});

test('ascii dot followed by whitespace splits latin sentences', () => {
  // 'Hello. This is a test. Done' — the trailing 'Done' has no terminator
  // yet, so it stays buffered until either a later delta or the flush.
  const step = feedSentence('', 'Hello. This is a test. Done');
  expect(step.sentences).toEqual(['Hello.', 'This is a test.']);
  expect(step.buffer).toBe('Done');
  // The whitespace can arrive in a later delta.
  const later = feedSentence('Done.', ' Next one. Fine.');
  expect(later.sentences).toEqual(['Done.', 'Next one.']);
  expect(later.buffer).toBe('Fine.');
  // A dot before a newline terminates too; before a letter it never does.
  expect(feedSentence('', 'Done.\n下句。')).toEqual({ buffer: '', sentences: ['Done.', '下句。'] });
  expect(feedSentence('', 'ok.done. go').sentences).toEqual(['ok.done.']);
});

test('overlong tail soft-cuts at the last separator, else hard-cuts', () => {
  const head = '一'.repeat(30);
  const soft = `${head}，${'二'.repeat(40)}，${'三'.repeat(20)}`;
  const step = feedSentence('', soft);
  expect(step.sentences).toEqual([`${head}，${'二'.repeat(40)}，`]);
  expect(step.buffer).toBe('三'.repeat(20));
  // No separator at all: hard cut at the code-point limit, remainder kept.
  const hard = feedSentence('', '四'.repeat(SENTENCE_LIMIT + 25));
  expect(hard.sentences).toEqual(['四'.repeat(SENTENCE_LIMIT)]);
  expect(hard.buffer).toBe('四'.repeat(25));
  // The separator must sit inside the first SENTENCE_LIMIT code points.
  const early = feedSentence('', '，'.repeat(3) + '五'.repeat(SENTENCE_LIMIT + 5));
  expect(early.sentences).toEqual(['五'.repeat(SENTENCE_LIMIT)]);
  expect(early.buffer).toBe('五'.repeat(5));
});

test('emoji and other surrogate pairs are never split mid-pair', () => {
  const emoji = '😀'.repeat(SENTENCE_LIMIT + 1);
  const step = feedSentence('', emoji);
  expect(step.sentences).toEqual([emoji.slice(0, SENTENCE_LIMIT * 2)]);
  expect(step.buffer).toBe('😀');
  expect(feedSentence('', '😀😀！').sentences).toEqual(['😀😀！']);
});

test('flush emits the remaining content once and drops punctuation-only tails', () => {
  expect(flushSentences(' 还有最后一句')).toEqual(['还有最后一句']);
  expect(flushSentences('好的')).toEqual(['好的']);
  expect(flushSentences('。！')).toEqual([]);
  expect(flushSentences(' \n … ')).toEqual([]);
});
