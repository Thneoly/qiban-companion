import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { nativeDesktop } from '../../lib/surface';
import { decodeLocalRoots, decodeLocalRootsAddResult, type LocalRoot } from '@companion/contracts';

/**
 * Local search consent: directories the assistant may search READ-ONLY.
 * Off by default (empty list); every search result is disclosed to the
 * configured model service — the fixed sentence says exactly that.
 */
export function LocalSearchSettings() {
  const [roots, setRoots] = useState<LocalRoot[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState('');

  const refresh = () => {
    invoke('local_roots_get')
      .then(v => setRoots(decodeLocalRoots(v)))
      .catch(e => setNote(String(e instanceof Error ? e.message : e)));
  };

  useEffect(() => {
    if (nativeDesktop) refresh();
  }, []);

  const add = async () => {
    if (busy) return;
    setBusy(true); setNote('');
    try {
      const result = decodeLocalRootsAddResult(await invoke('local_roots_add', { path: null }));
      if (result.status === 'added') { setRoots(decodeLocalRoots(await invoke('local_roots_get'))); setNote('已添加。'); }
      else if (result.status === 'invalid') setNote(result.reason);
    } catch (e) {
      setNote(String(e instanceof Error ? e.message : e));
    } finally { setBusy(false); }
  };

  const remove = async (id: number) => {
    if (busy) return;
    setBusy(true); setNote('');
    try {
      setRoots(decodeLocalRoots(await invoke('local_roots_remove', { id })));
    } catch (e) {
      setNote(String(e instanceof Error ? e.message : e));
    } finally { setBusy(false); }
  };

  if (!nativeDesktop) return null;

  return (
    <section className="model-settings" aria-label="本地检索设置">
      <h2>本地检索</h2>
      <p>
        让栖栖能在你指定的目录里查找文件与内容。检索结果会发送给你配置的模型服务，
        仅在你添加的目录内只读检索；栖栖不会修改任何文件。默认关闭。
      </p>
      <div className="account-actions">
        <button type="button" disabled={busy} onClick={() => void add()}>添加目录…</button>
      </div>
      {roots && roots.length > 0 ? (
        <ul className="account-tasks">
          {roots.map(root => (
            <li key={root.id}>
              <strong>{root.path}</strong>
              <small>{root.label ?? ''}</small>
              <button type="button" disabled={busy} onClick={() => void remove(root.id)}>移除</button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="helper">还没有授权目录；添加后，聊天里的栖栖即可检索它们。</p>
      )}
      {note && <p className="model-settings-note" role="status">{note}</p>}
    </section>
  );
}
