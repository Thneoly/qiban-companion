// Pinned runtime only. No character or arbitrary public/ file enters the installer.
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
const manifest = JSON.parse(await readFile(new URL('./live2d-runtime-manifest.json', import.meta.url), 'utf8'));
for (const item of manifest) {
  const target = new URL('../public/live2d-runtime/' + item.file, import.meta.url);
  const verify = bytes => item.sha256
    ? createHash('sha256').update(bytes).digest('hex') === item.sha256
    : createHash('sha1').update(Buffer.from('blob ' + bytes.length + '\0')).update(bytes).digest('hex') === item.gitBlob;
  let bytes;
  try { bytes = await readFile(target); } catch (e) { if (e.code !== 'ENOENT') throw e; }
  if (!bytes) {
    try { bytes = await readFile(new URL('../public/live2d-local/' + item.file, import.meta.url)); }
    catch (e) { if (e.code !== 'ENOENT') throw e; }
    if (!bytes) {
      const response = await fetch(item.url, { signal: AbortSignal.timeout(30000) });
      if (!response.ok) throw Error('Runtime download failed: ' + item.file);
      bytes = Buffer.from(await response.arrayBuffer());
    }
  }
  if (!verify(bytes)) throw Error('Runtime hash mismatch: ' + item.file);
  await mkdir(fileURLToPath(new URL('.', target)), { recursive: true });
  await writeFile(target, bytes);
}
console.log('Verified 4 pinned Live2D runtime/notice files (no character).');
