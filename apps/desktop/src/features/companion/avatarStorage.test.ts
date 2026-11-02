import { describe, expect, it } from 'vitest';
import { decodePickedFolder, importModel, localPath, normalizeAppearance, type ImportedFile } from './avatarStorage';
function files(ref: unknown = 'body.moc3'): ImportedFile[] {
  const bytes = new Uint8Array(24), data = new DataView(bytes.buffer);
  data.setUint32(0, 0x89504e47); data.setUint32(4, 0x0d0a1a0a); data.setUint32(12, 0x49484452); data.setUint32(16, 64); data.setUint32(20, 64);
  return [{ path: 'pet.model3.json', blob: new File([JSON.stringify({ Version:3, FileReferences:{ Moc:ref, Textures:['texture.png'], Motions:{Idle:[{File:'idle.motion3.json',Sound:'https://remote.invalid/audio.wav'}]} } })], 'pet.model3.json') },
    { path: 'body.moc3', blob: new File(['MOC3'], 'body.moc3') }, { path: 'texture.png', blob: new File([bytes], 'texture.png') }, { path: 'idle.motion3.json', blob: new File(['{}'], 'idle.motion3.json') }, { path: 'unused.js', blob: new File(['throw Error("never run")'], 'unused.js') }];
}
describe('user artwork import boundary', () => {
  it.each(['../secret','a/../secret','/absolute','C:/file','https://host/file','data:abc','blob:abc','a\\b','%2e%2e/file','a?b','a#b','a//b'])('rejects non-local path %s', value => expect(() => localPath(value)).toThrow());
  it('copies only referenced local assets, stripping sound and ignoring scripts', async () => {
    const model = await importModel(files());
    expect(Object.keys(model.files)).toHaveLength(3);
    expect(model.files['unused.js']).toBeUndefined();
    expect(JSON.stringify(model.settings)).not.toContain('remote.invalid');
  });
  it('rejects remote resources and missing files before renderer gets them', async () => {
    await expect(importModel(files('https://remote.invalid/model.moc3'))).rejects.toThrow('相对路径');
    await expect(importModel(files().filter(f => !f.path.endsWith('.png')))).rejects.toThrow('缺少');
  });
  it('rejects oversized GPU textures and multiple model entries', async () => {
    const selected = files(); const texture = selected[2]!;
    const bytes = await texture.blob.arrayBuffer(); new DataView(bytes).setUint32(16, 10000);
    selected[2] = { path: 'texture.png', blob: new File([bytes], 'texture.png') };
    await expect(importModel(selected)).rejects.toThrow('4096');
    await expect(importModel([...files(), { path: 'other.model3.json', blob: new File(['{}'], 'other.model3.json') }])).rejects.toThrow('只能有一个');
  });
  it('normalizes stale appearance values', () => {
    expect(normalizeAppearance({opacity:NaN})).toEqual({renderer:'svg',scene:'stage',opacity:65});
    expect(normalizeAppearance({opacity:200})).toHaveProperty('opacity',100);
  });
});

describe('native picker payload boundary', () => {
  const manifestOf = (files: { path: string; offset: number; len: number }[], cancelled = false) =>
    new TextEncoder().encode(JSON.stringify({ cancelled, files }));
  const payloadOf = (manifest: Uint8Array, ...blobs: Uint8Array[]) => {
    const body = new Uint8Array(4 + manifest.length + blobs.reduce((n, b) => n + b.length, 0));
    new DataView(body.buffer).setUint32(0, manifest.length);
    body.set(manifest, 4); let offset = 4 + manifest.length;
    for (const blob of blobs) { body.set(blob, offset); offset += blob.length; }
    return body;
  };
  it('decodes ranges into blobs and replays the exact bytes', async () => {
    const moc = new TextEncoder().encode('MOC3'), json = new TextEncoder().encode('{}');
    const body = payloadOf(manifestOf([{ path: 'm/pet.model3.json', offset: moc.length, len: json.length }, { path: 'm/body.moc3', offset: 0, len: moc.length }]), moc, json);
    for (const input of [body.buffer, body, Array.from(body)]) {
      const picked = decodePickedFolder(input);
      expect(picked.cancelled).toBe(false);
      expect(picked.files.map(f => f.path)).toEqual(['m/pet.model3.json', 'm/body.moc3']);
      expect(await picked.files[0]!.blob.text()).toBe('{}');
      expect(await picked.files[1]!.blob.text()).toBe('MOC3');
    }
  });
  it('reports cancellation without surfacing an error', () => {
    const body = payloadOf(manifestOf([], true));
    expect(decodePickedFolder(body.buffer)).toEqual({ cancelled: true, files: [] });
  });
  it('refuses truncated or out-of-range payloads', () => {
    const body = payloadOf(manifestOf([{ path: 'a', offset: 0, len: 4 }]), new Uint8Array(2));
    expect(() => decodePickedFolder(body.buffer.slice(0, 6))).toThrow('不完整');
    expect(() => decodePickedFolder(body.buffer.slice(0, 4))).toThrow('不完整');
    expect(() => decodePickedFolder(new Uint8Array([0, 0, 0, 8, 123]).buffer)).toThrow('不完整');
    expect(() => decodePickedFolder(payloadOf(manifestOf([{ path: 'a', offset: 0, len: 9 }]), new Uint8Array(2)).buffer)).toThrow('不完整');
    expect(() => decodePickedFolder('nope')).toThrow('不兼容');
  });
});
