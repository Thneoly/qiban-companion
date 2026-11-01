// align-bgm.mjs - cross-correlate the delivered draft-v2 audio against the
// reconstructed BGM stem to find the true lag/scale before subtraction.
// Usage: node align-bgm.mjs  (reads draft-audio.wav + bgmproc.wav in cwd)
import { readFileSync } from 'node:fs';

function readWavStereoMono(path) {
  const buf = readFileSync(path);
  // naive RIFF walk to the data chunk
  let off = 12, channels = 0, rate = 0, bits = 0, dataOff = 0, dataLen = 0;
  while (off + 8 <= buf.length) {
    const id = buf.toString('ascii', off, off + 4);
    const sz = buf.readUInt32LE(off + 4);
    if (id === 'fmt ') {
      channels = buf.readUInt16LE(off + 10);
      rate = buf.readUInt32LE(off + 12);
      bits = buf.readUInt16LE(off + 22);
    } else if (id === 'data') { dataOff = off + 8; dataLen = sz; break; }
    off += 8 + sz + (sz % 2);
  }
  const bytesPer = bits / 8;
  const frames = Math.floor(dataLen / (bytesPer * channels));
  const mono = new Float64Array(frames);
  const view = new DataView(buf.buffer, buf.byteOffset + dataOff);
  for (let i = 0; i < frames; i++) {
    let acc = 0;
    for (let c = 0; c < channels; c++) {
      const o = (i * channels + c) * bytesPer;
      acc += bits === 32 ? view.getFloat32(o, true) : view.getInt16(o, true) / 32768;
    }
    mono[i] = acc / channels;
  }
  return { rate, frames, mono };
}

const A = readWavStereoMono('draft-audio.wav');   // 48k, mixed (bgm+voice)
const B = readWavStereoMono('bgmproc.wav');       // 48k mono, bgm stem @0.16

// analysis window: 3s..29s (post-fade-in, pre-voice, full chords)
const t0 = 3 * A.rate, t1 = 29 * A.rate;
const DEC = 48;                                    // -> 1kHz effective
const n = Math.floor((t1 - t0) / DEC);
const a = new Float64Array(n), b0 = new Float64Array(n);
for (let i = 0; i < n; i++) { a[i] = A.mono[t0 + i * DEC]; b0[i] = B.mono[t0 + i * DEC]; }
const mean = (x) => { let s = 0; for (const v of x) s += v; return s / x.length; };
const ma = mean(a), mb = mean(b0);
for (let i = 0; i < n; i++) { a[i] -= ma; b0[i] -= mb; }

// coarse search: lag in decimated units; positive = stem later than draft
const MAXLAG = Math.round(0.15 * A.rate / DEC);    // +-150ms
let best = { c: -Infinity };
for (let lag = -MAXLAG; lag <= MAXLAG; lag++) {
  let dot = 0, ea = 0, eb = 0, cnt = 0;
  for (let i = Math.max(0, lag); i < Math.min(n, n + lag); i++) {
    const av = a[i], bv = b0[i - lag];
    dot += av * bv; ea += av * av; eb += bv * bv; cnt++;
  }
  const c = dot / Math.sqrt(ea * eb || 1);
  if (c > best.c) best = { c, lag, dot, ea, eb, cnt };
}
const scaleCoarse = best.dot / (best.ea || 1);
console.log(`coarse: lag=${best.lag} dec-samples (${(best.lag * DEC / A.rate * 1000).toFixed(1)}ms) corr=${best.c.toFixed(4)} scale=${scaleCoarse.toFixed(4)}`);

// fine search around coarse lag at full 48k on a 6s window
const f0 = 5 * A.rate, f1 = 11 * A.rate;
const nf = f1 - f0;
const fa = new Float64Array(nf);
for (let i = 0; i < nf; i++) fa[i] = A.mono[f0 + i] - ma;
const coarseSamples = best.lag * DEC;
let bestF = { c: -Infinity };
for (let d = -DEC - 2; d <= DEC + 2; d++) {
  const lagS = coarseSamples + d;
  let dot = 0, ea = 0, eb = 0;
  for (let i = 0; i < nf; i++) {
    const j = f0 + i - lagS;
    if (j < 0 || j >= B.frames) continue;
    const av = fa[i], bv = B.mono[j] - mb;
    dot += av * bv; ea += av * av; eb += bv * bv;
  }
  const c = dot / Math.sqrt(ea * eb || 1);
  if (c > bestF.c) bestF = { c, lagS, dot, ea, eb };
}
console.log(`fine:   lag=${bestF.lagS} samples (${(bestF.lagS / A.rate * 1000).toFixed(2)}ms) corr=${bestF.c.toFixed(4)} scale=${(bestF.dot / bestF.ea).toFixed(4)}`);
console.log(`suggest: adelay draft by ${bestF.lagS} samples (or atrim stem), subtract at scale ${((bestF.dot / bestF.ea) / 0.16).toFixed(3)} x 0.16`);
