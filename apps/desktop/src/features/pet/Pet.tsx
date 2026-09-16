import { useCallback, useEffect, useRef, useState, type FormEvent, type PointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AvatarArtwork } from '../companion/AvatarArtwork';
import { client, errorMessage } from '../../lib/client';
import { nativeDesktop, petAction } from '../../lib/surface';

export function Pet() {
  const [open, setOpen] = useState(false);
  const [hidden, setHidden] = useState(false);
  const [quiet, setQuiet] = useState(false);
  const [ready, setReady] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState('慢慢来，我在这里。');
  const [error, setError] = useState('');
  const [offset, setOffset] = useState({ x:0, y:0 });
  const shell = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x:number; y:number; startX:number; startY:number } | null>(null);
  const restore = useCallback(() => { setHidden(false); setQuiet(false); setOpen(false); setError(''); }, []);

  useEffect(() => {
    let disposed = false;
    const removers: (() => void)[] = [];
    async function initialize() {
      try {
        await client.getRuntimeInfo();
        if (nativeDesktop) {
          for (const [name, callback] of [
            ['pet-restored', restore],
            ['pet-quiet', () => { setQuiet(true); setOpen(false); }],
          ] as const) {
            const unlisten = await listen(name, callback);
            if (disposed) unlisten(); else removers.push(unlisten);
          }
          if (!disposed) await petAction('ready');
        }
        if (!disposed) setReady(true);
      } catch (e) { if (!disposed) { setError(errorMessage(e)); setOpen(true); } }
    }
    void initialize();
    const blur = () => setOpen(false);
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpen(false); };
    window.addEventListener('blur', blur);
    window.addEventListener('keydown', escape);
    return () => { disposed = true; removers.forEach(remove => remove()); window.removeEventListener('blur', blur); window.removeEventListener('keydown', escape); };
  }, [restore]);

  useEffect(() => {
    if (!nativeDesktop || !shell.current) return;
    let disposed = false;
    let frame = 0;
    const report = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const regions = [...(shell.current?.querySelectorAll<HTMLElement>('[data-pet-hit]') ?? [])]
          .filter(element => element.getClientRects().length > 0)
          .map(element => { const r = element.getBoundingClientRect(); return { x:r.x, y:r.y, width:r.width, height:r.height }; });
        void invoke('set_pet_regions', { regions }).catch(e => { if (!disposed) setError(errorMessage(e)); });
      });
    };
    const observer = new ResizeObserver(report);
    observer.observe(shell.current);
    shell.current.querySelectorAll('[data-pet-hit]').forEach(element => observer.observe(element));
    report();
    window.addEventListener('resize', report);
    return () => { disposed = true; observer.disconnect(); cancelAnimationFrame(frame); window.removeEventListener('resize', report); };
  }, [open, quiet, hidden, error]);

  async function act(action: 'hide' | 'quiet' | 'open_panel') {
    setError('');
    try {
      await petAction(action);
      if (action === 'hide') setHidden(true);
      if (action === 'quiet') setQuiet(true);
      setOpen(false);
    } catch (e) { setError(errorMessage(e)); }
  }
  async function create(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true); setError('');
    try {
      await client.createTask(draft);
      setDraft(''); setNote('已经记下了，稍后可以在任务面板查看。');
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  function beginDrag(event: PointerEvent<HTMLButtonElement>) {
    if (event.button !== 0) return;
    if (nativeDesktop) { void petAction('drag').catch(e => setError(errorMessage(e))); return; }
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { x:event.clientX, y:event.clientY, startX:offset.x, startY:offset.y };
  }
  function moveDrag(event: PointerEvent<HTMLButtonElement>) {
    if (!drag.current) return;
    setOffset({
      x: Math.max(-Math.max(0, innerWidth - 344), Math.min(8, drag.current.startX + event.clientX - drag.current.x)),
      y: Math.max(-Math.max(0, innerHeight - 464), Math.min(8, drag.current.startY + event.clientY - drag.current.y)),
    });
  }
  return <>
    {!nativeDesktop && <aside className="preview-desktop-note"><strong>栖伴 · 桌面角色预览</strong><p>这里模拟角色形态；真实透明悬浮、托盘和鼠标穿透请运行桌面版。</p><button onClick={restore}>恢复角色预览</button></aside>}
    <div ref={shell} className={`pet-shell ${hidden ? 'pet-hidden' : ''} ${quiet ? 'pet-quiet' : ''}`} style={!nativeDesktop ? { transform:`translate(${offset.x}px, ${offset.y}px)` } : undefined}>
      {open && <section data-pet-hit className="pet-dialog" aria-label="栖栖的交互气泡">
        <header><strong>栖栖 <span>在你身边</span></strong><button aria-label="收起气泡" onClick={() => setOpen(false)}>×</button></header>
        <p className="pet-message" role="status">{note}</p>
        <form onSubmit={create}>
          <label className="sr-only" htmlFor="pet-draft">想记下什么？</label>
          <input id="pet-draft" value={draft} onChange={e => setDraft(e.target.value)} placeholder="想做的事，先记下来…" disabled={busy || !ready}/>
          <button className="pet-save" disabled={busy || !ready || !draft.trim()}>{busy ? '保存中…' : '记下来'}</button>
        </form>
        <p className="pet-limit">仅保存待办 · AI 对话和执行尚未接入</p>
        {error && <p role="alert" className="pet-error">{error}</p>}
        <div className="pet-actions">
          <button onClick={() => void act('open_panel')}>任务面板 ↗</button>
          <button onClick={() => void act('quiet')}>安静陪伴</button>
          <button onClick={() => void act('hide')}>隐藏</button>
        </div>
      </section>}
      <button data-pet-hit className="pet-character" aria-label="和栖栖互动" aria-expanded={open} disabled={quiet} onClick={() => { setOpen(value => !value); setNote('慢慢来，我在这里。'); }}>
        <AvatarArtwork/>
      </button>
      <button data-pet-hit className="pet-drag" disabled={quiet} onPointerDown={beginDrag} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; }} aria-label="拖动栖栖">⠿ <span>{quiet ? '安静陪伴中 · 托盘可唤回' : '栖栖 · 拖动这里'}</span></button>
      {!open && error && <button data-pet-hit className="pet-error-reopen" onClick={() => setOpen(true)}>操作未完成，点击查看</button>}
    </div>
  </>;
}
