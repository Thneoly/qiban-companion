/** Appearance and user-selected artwork only; never credentials, conversations or memories. */
export type Appearance = { renderer: 'svg' | 'live2d'; scene: 'stage' | 'none' | 'room'; opacity: number };
export type ModelBundle = { name: string; settings: Record<string, unknown>; files: Record<string, Blob> };
export type AvatarRecord = { appearance: Appearance; model?: ModelBundle };
export const defaultAppearance: Appearance = { renderer: 'svg', scene: 'stage', opacity: 65 };
export function normalizeAppearance(value: Partial<Appearance> = {}): Appearance {
  return { renderer: value.renderer === 'live2d' ? 'live2d' : 'svg',
    scene: value.scene === 'none' || value.scene === 'room' ? value.scene : 'stage',
    opacity: typeof value.opacity === 'number' && Number.isFinite(value.opacity) ? Math.min(100, Math.max(0, value.opacity)) : 65 };
}
export async function avatarRecord(record?: AvatarRecord): Promise<AvatarRecord | undefined> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open('qiban-avatar', 1);
    open.onupgradeneeded = () => open.result.createObjectStore('appearance');
    open.onerror = () => reject(Error('无法打开角色存储'));
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction('appearance', record ? 'readwrite' : 'readonly');
      const store = tx.objectStore('appearance');
      const request = record ? store.put(record, 'current') : store.get('current');
      tx.oncomplete = () => { db.close(); resolve(record ?? request.result); };
      tx.onabort = tx.onerror = () => { db.close(); reject(Error('角色设置保存失败，请检查本机可用空间')); };
    };
  });
}
const object = (value: unknown): Record<string, any> => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error('模型 JSON 格式无效');
  return value as Record<string, any>;
};
export function localPath(value: unknown): string {
  if (typeof value !== 'string' || !value || /[\\:%?#\x00-\x1f]/.test(value) || value.startsWith('/') || value.split('/').some(x => !x || x === '.' || x === '..'))
    throw Error('模型引用必须是目录内的相对路径');
  return value;
}
/** Rebuild the resource table so a model cannot cause remote fetches, script or audio execution. */
export async function importModel(selected: File[]): Promise<ModelBundle> {
  if (!selected.length || selected.length > 256 || selected.reduce((n, f) => n + f.size, 0) > 80 * 1024 * 1024)
    throw Error('请选择单个模型文件夹：最多 256 个文件、80 MB');
  const all = new Map<string, File>();
  for (const file of selected) {
    const key = localPath(file.webkitRelativePath || file.name);
    if (all.has(key)) throw Error('模型文件路径重复');
    all.set(key, file);
  }
  const entries = [...all.keys()].filter(x => x.endsWith('.model3.json'));
  if (entries.length !== 1) throw Error('文件夹中需要且只能有一个 .model3.json');
  const entry = entries[0]!;
  const sourceFile = all.get(entry)!;
  if (sourceFile.size > 1024 * 1024) throw Error('模型描述文件过大');
  const source = object(JSON.parse(await sourceFile.text()));
  if (source.Version !== 3) throw Error('当前支持 Cubism 3/4 的 model3 格式');
  const refs = object(source.FileReferences);
  const directory = entry.slice(0, entry.lastIndexOf('/') + 1);
  const files: Record<string, Blob> = Object.create(null);
  const use = (value: unknown, suffix: string) => {
    const key = localPath(value);
    if (!key.toLowerCase().endsWith(suffix)) throw Error('模型引用文件类型不支持');
    const file = all.get(directory + key);
    if (!file) throw Error('模型缺少文件：' + key);
    if (!file.size || file.size > 20 * 1024 * 1024) throw Error('模型单个资源需要在 0～20 MB 之间');
    files[key] = file.slice(0, file.size, suffix === '.png' ? 'image/png' : 'application/octet-stream');
    return key;
  };
  const references: Record<string, any> = { Moc: use(refs.Moc, '.moc3') };
  if (!Array.isArray(refs.Textures) || !refs.Textures.length || refs.Textures.length > 8) throw Error('需要 1～8 张 PNG 纹理');
  references.Textures = refs.Textures.map(x => use(x, '.png'));
  for (const field of ['Physics', 'Pose']) if (refs[field]) references[field] = use(refs[field], '.json');
  if (refs.Expressions) {
    if (!Array.isArray(refs.Expressions) || refs.Expressions.length > 64) throw Error('表情列表无效');
    references.Expressions = refs.Expressions.map((entry: unknown) => {
      const item = object(entry);
      return { Name: String(item.Name).slice(0, 100), File: use(item.File, '.json') };
    });
  }
  if (refs.Motions) {
    references.Motions = Object.create(null);
    for (const [group, entries] of Object.entries(object(refs.Motions))) {
      if (!Array.isArray(entries) || entries.length > 64) throw Error('动作列表无效');
      references.Motions[group] = entries.map((entry: unknown) => ({ File: use(object(entry).File, '.json') }));
    }
  }
  // Validate bounded JSON and PNG headers before handing them to the renderer.
  let pixels = 0;
  for (const [key, blob] of Object.entries(files)) {
    if (key.endsWith('.json')) {
      if (blob.size > 4 * 1024 * 1024) throw Error('动作或物理 JSON 文件过大');
      object(JSON.parse(await blob.text()));
    } else if (key.endsWith('.png')) {
      const data = new DataView(await blob.slice(0, 24).arrayBuffer());
      if (data.byteLength < 24 || data.getUint32(0) !== 0x89504e47 || data.getUint32(4) !== 0x0d0a1a0a || data.getUint32(12) !== 0x49484452
        || !data.getUint32(16) || !data.getUint32(20) || data.getUint32(16) > 4096 || data.getUint32(20) > 4096) throw Error('纹理必须是最大 4096×4096 的 PNG');
      pixels += data.getUint32(16) * data.getUint32(20);
      if (pixels > 32 * 1024 * 1024) throw Error('纹理总像素超过 32 MP');
    } else if (key.endsWith('.moc3')) {
      if (await blob.slice(0, 4).text() !== 'MOC3') throw Error('无效的 moc3 模型');
    }
  }
  const settings: Record<string, unknown> = { Version: 3, FileReferences: references };
  for (const field of ['Groups', 'HitAreas', 'Layout']) if (source[field]) settings[field] = source[field];
  return { name: entry.split('/').at(-1)!.replace('.model3.json', ''), settings, files };
}
/** Blob URLs are generated by us and kept until renderer teardown, including lazy motions. */
export function modelSource(bundle: ModelBundle) {
  const urls = new Map(Object.entries(bundle.files).map(([key, blob]) => [key, URL.createObjectURL(blob)]));
  return { settings: { ...bundle.settings, url: location.origin + '/avatar.model3.json' },
    resolveURL: (file: string) => { const url = urls.get(file); if (!url) throw Error('未授权的模型资源'); return url; },
    dispose: () => urls.forEach(url => URL.revokeObjectURL(url)) };
}
