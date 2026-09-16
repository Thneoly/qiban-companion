import { expect, test } from 'vitest';
import { pcmWave } from './audio';
import { decodeVoiceResult } from '@companion/contracts';

test('WAV encodes mono PCM16 with clipping and bounded duration', () => {
  const wave = pcmWave(new Float32Array([-2, -1, 0, 1, 2, NaN]), 16000);
  const view = new DataView(wave.buffer);
  expect(new TextDecoder().decode(wave.slice(0, 4))).toBe('RIFF');
  expect(view.getUint32(4, true) + 8).toBe(wave.length);
  expect(view.getUint32(28, true)).toBe(32000);
  expect([0, 1, 2, 3, 4, 5].map(i => view.getInt16(44 + i * 2, true))).toEqual([-32768, -32768, 0, 32767, 32767, 0]);
  expect(() => pcmWave(new Float32Array(480001), 16000)).toThrow();
  expect(() => pcmWave(new Float32Array(), 16000)).toThrow();
});
test('voice IPC rejects invalid audio or invented cost instead of playing it', () => {
  const result = { requestId: 'test', transcript: 'hello', reply: 'hi', wav: Array.from(pcmWave(new Float32Array(160),16000)), recognitionMs: 1, generationMs: 2, synthesisMs: 3, inputSeconds: .01, outputSeconds: .01, modelTotalTokens: null, audioCost: null };
  expect(decodeVoiceResult(result).modelTotalTokens).toBeNull();
  for (const change of [{ wav: [256] }, { inputSeconds: 31 }, { outputSeconds: Infinity }, { audioCost: 0 }, { modelTotalTokens: -1 }]) expect(() => decodeVoiceResult({ ...result, ...change })).toThrow();
});
