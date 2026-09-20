import { describe, expect, it } from 'vitest';
import { importModel, localPath, normalizeAppearance } from './avatarStorage';
function files(ref: unknown = 'body.moc3') {
  const bytes = new Uint8Array(24), data = new DataView(bytes.buffer);
  data.setUint32(0, 0x89504e47); data.setUint32(4, 0x0d0a1a0a); data.setUint32(12, 0x49484452); data.setUint32(16, 64); data.setUint32(20, 64);
  return [new File([JSON.stringify({ Version:3, FileReferences:{ Moc:ref, Textures:['texture.png'], Motions:{Idle:[{File:'idle.motion3.json',Sound:'https://remote.invalid/audio.wav'}]} } })], 'pet.model3.json'),
    new File(['MOC3'], 'body.moc3'), new File([bytes], 'texture.png'), new File(['{}'], 'idle.motion3.json'), new File(['throw Error("never run")'], 'unused.js')];
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
    await expect(importModel(files().filter(f => !f.name.endsWith('.png')))).rejects.toThrow('缺少');
  });
  it('rejects oversized GPU textures and multiple model entries', async () => {
    const selected = files(); const texture = selected[2]!;
    const bytes = await texture.arrayBuffer(); new DataView(bytes).setUint32(16, 10000);
    selected[2] = new File([bytes], 'texture.png');
    await expect(importModel(selected)).rejects.toThrow('4096');
    await expect(importModel([...files(), new File(['{}'], 'other.model3.json')])).rejects.toThrow('只能有一个');
  });
  it('normalizes stale appearance values', () => {
    expect(normalizeAppearance({opacity:NaN})).toEqual({renderer:'svg',scene:'stage',opacity:65});
    expect(normalizeAppearance({opacity:200})).toHaveProperty('opacity',100);
  });
});
