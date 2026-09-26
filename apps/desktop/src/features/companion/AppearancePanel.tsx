import { useEffect, useState } from 'react';
import { importModel, type Appearance, type AvatarRecord } from './avatarStorage';
export function AppearancePanel({ record, save, close }: { record: AvatarRecord; save: (record: AvatarRecord) => Promise<void>; close: () => void }) {
  const [opacity, setOpacity] = useState(record.appearance.opacity);
  useEffect(() => setOpacity(record.appearance.opacity), [record.appearance.opacity]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const change = async (appearance: Appearance) => {
    setBusy(true); setError('');
    try { await save({ ...record, appearance }); } catch { setError('设置未保存，请检查本机存储后重试。'); } finally { setBusy(false); }
  };
  return <div className="appearance-panel">
    <div className="appearance-title"><strong>角色与场景</strong><button onClick={close}>返回互动</button></div>
    <fieldset disabled={busy}><legend>陪伴形象</legend>
      <button aria-pressed={record.appearance.renderer === 'svg'} onClick={() => void change({ ...record.appearance, renderer: 'svg' })}>栖栖</button>
      <button disabled={!record.model} aria-pressed={record.appearance.renderer === 'live2d'} onClick={() => void change({ ...record.appearance, renderer: 'live2d' })}>2D 数字人</button>
      <p>{record.model ? `本机模型：${record.model.name}` : '导入自己的 Live2D 模型后即可切换。'}</p>
      <label className="model-import">{busy ? '正在保存…' : '导入模型文件夹'}<input aria-label="导入模型文件夹" type="file" multiple
        {...{ webkitdirectory: '' }} onChange={async e => {
          const files = [...(e.currentTarget.files ?? [])]; e.currentTarget.value = '';
          if (!files.length) return;
          setBusy(true); setError('');
          try { const model = await importModel(files); await save({ ...record, model, appearance: { ...record.appearance, renderer: 'live2d' } }); }
          catch (e) { setError(e instanceof Error ? e.message : '导入失败'); } finally { setBusy(false); }
        }}/></label>
      {record.model && <button onClick={async () => {
        setBusy(true); setError('');
        try { await save({ appearance: { ...record.appearance, renderer: 'svg' } }); } catch { setError('移除失败，请重试。'); } finally { setBusy(false); }
      }}>移除本机模型</button>}
      <small>选择含一个 .model3.json 的完整文件夹（含 moc3、PNG 和动作），最多 80 MB。只复制到本机，不上传；移除不影响源文件。</small>
    </fieldset>
    <fieldset disabled={busy}><legend>脚下的小天地</legend>
      <label>场景<select aria-label="场景" value={record.appearance.scene} onChange={e => void change({ ...record.appearance, scene: e.target.value as Appearance['scene'] })}>
        <option value="stage">悬浮舞台 · 立体层次</option><option value="none">无背景 · 仅角色</option><option value="room">完整家园</option>
      </select></label>
      <label>背景不透明度 {opacity}%<input aria-label="背景不透明度" type="range" min="0" max="100" step="5" value={opacity}
        onChange={e => setOpacity(Number(e.target.value))}
        onPointerUp={e => void change({ ...record.appearance, opacity: Number(e.currentTarget.value) })}
        onKeyUp={e => void change({ ...record.appearance, opacity: Number(e.currentTarget.value) })}
        onBlur={() => { if (!busy && opacity !== record.appearance.opacity) void change({ ...record.appearance, opacity }); }}/></label>
      <small>角色保持清晰。无背景或 0% 时背景不挡鼠标；仍可拖动角色移动。</small>
    </fieldset>
    {error && <p role="alert" className="pet-error">{error}</p>}
  </div>;
}
