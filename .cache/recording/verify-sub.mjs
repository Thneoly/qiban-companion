// verify-sub.mjs - direct sample-domain check of draft vs bgmproc2 per channel
import { readFileSync } from 'node:fs';

function readWav(path) {
  const buf = readFileSync(path);
  let off = 12, channels = 0, rate = 0, bits = 0, dataOff = 0, dataLen = 0;
  while (off + 8 <= buf.length) {
    const id = buf.toString('ascii', off, off + 4);
    const sz = buf.readUInt32LE(off + 4);
    if (id === 'fmt ') { channels = buf.readUInt16LE(off + 10); rate = buf.readUInt32LE(off + 12); bits = buf.readUInt16LE(off + 22); }
    else if (id === 'data') { dataOff = off + 8; dataLen = sz; break; }
    off += 8 + sz + (sz % 2);
  }
  const bytesPer = bits / 8, frames = Math.floor(dataLen / (bytesPer * channels));
  const chs = Array.from({ length: channels }, () => new Float64Array(frames));
  const view = new DataView(buf.buffer, buf.byteOffset + dataOff);
  for (let i = 0; i < frames; i++) for (let c = 0; c < channels; c++)
    chs[c][i] = bits === 32 ? view.getFloat32((i * channels + c) * bytesPer, true) : view.getInt16((i * channels + c) * bytesPer, true) / 32768;
  return { rate, frames, chs };
}

const A = readWav('draft-audio.wav');
const B = readWav('bgmproc2.wav');
console.log(`draft: ${A.chs.length}ch ${A.frames}f; bgmproc2: ${B.chs.length}ch ${B.frames}f`);

const f0 = 5 * A.rate, f1 = 11 * A.rate;
function fitLag(ac, bc, lagLo, lagHi) {
  let best = { c: -Infinity };
  for (let lag = lagLo; lag <= lagHi; lag++) {
    let dot = 0, ea = 0, eb = 0;
    for (let i = f0; i < f1; i++) {
      const j = i - lag; if (j < 0 || j >= bc.length) continue;
      dot += ac[i] * bc[j]; ea += ac[i] * ac[i]; eb += bc[j] * bc[j];
    }
    const c = dot / Math.sqrt(ea * eb || 1);
    if (c > best.c) best = { c, lag, dot, ea, eb };
  }
  return best;
}
for (let c = 0; c < Math.min(2, A.chs.length); c++) {
  for (const bl of [B.chs[0], B.chs[B.chs.length - 1]]) {
    // search around 0 (bgmproc2 already has adelay 239 built in)
    const r = fitLag(A.chs[c], bl, -600, 600);
    const scale = r.dot / r.ea;
    // residual with best fit
    let res = 0, sig = 0;
    for (let i = f0; i < f1; i++) { const j = i - r.lag; if (j < 0 || j >= bl.length) continue; res += (A.chs[c][i] - scale * bl[j]) ** 2; sig += A.chs[c][i] ** 2; }
    console.log(`draft ch${c} vs ${bl === B.chs[0] ? 'B ch0' : 'B chL'}: lag=${r.lag} corr=${r.c.toFixed(4)} scale=${scale.toFixed(4)} residRMS=${(10 * Math.log10(res / (f1 - f0))).toFixed(1)}dB sigRMS=${(10 * Math.log10(sig / (f1 - f0))).toFixed(1)}dB`);
  }
}
