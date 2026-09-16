/** Encode short mono PCM16 audio; no external codec or persistent microphone. */
export function pcmWave(samples: Float32Array, sampleRate: number): Uint8Array {
  if (!Number.isInteger(sampleRate) || sampleRate < 8000 || sampleRate > 48000 || samples.length === 0 || samples.length > sampleRate * 30) throw Error('音频需要在30秒以内');
  const bytes = new Uint8Array(44 + samples.length * 2);
  const view = new DataView(bytes.buffer);
  const text = (offset: number, value: string) => [...value].forEach((c, i) => bytes[offset + i] = c.charCodeAt(0));
  text(0, 'RIFF'); view.setUint32(4, bytes.length - 8, true); text(8, 'WAVEfmt ');
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, 'data'); view.setUint32(40, samples.length * 2, true);
  samples.forEach((sample, i) => { const s = Math.max(-1, Math.min(1, Number.isFinite(sample) ? sample : 0)); view.setInt16(44 + i * 2, Math.round(s * (s < 0 ? 32768 : 32767)), true); });
  return bytes;
}
export async function normalizeRecording(blob: Blob): Promise<Uint8Array> {
  if (!blob.size || blob.size > 8 * 1024 * 1024) throw Error('录音为空或超过8 MiB');
  const context = new AudioContext({ sampleRate: 16000 });
  try {
    const decoded = await context.decodeAudioData(await blob.arrayBuffer());
    if (decoded.duration > 30) throw Error('音频超过30秒');
    const mono = new Float32Array(decoded.length);
    for (let ch = 0; ch < decoded.numberOfChannels; ch++) {
      const samples = decoded.getChannelData(ch);
      for (let i = 0; i < mono.length; i++) mono[i] = mono[i]! + samples[i]! / decoded.numberOfChannels;
    }
    return pcmWave(mono, decoded.sampleRate);
  } finally { await context.close(); }
}
