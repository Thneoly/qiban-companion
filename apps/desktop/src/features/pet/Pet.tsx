import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChatBubble } from '../chat/ChatBubble';
import { FirstUseGuide } from '../onboarding/FirstUseGuide';
import { Live2DRenderer } from '../companion/Live2DRenderer';
import { AvatarArtwork } from '../companion/AvatarArtwork';
import { companionLabels, companionState, type ConversationPhase } from '../companion/presentation';
import { client, errorMessage } from '../../lib/client';
import { nativeDesktop, petAction } from '../../lib/surface';
import { AppearancePanel } from '../companion/AppearancePanel';
import { avatarRecord, defaultAppearance, normalizeAppearance, type AvatarRecord } from '../companion/avatarStorage';
import { useSceneDrag } from './useSceneDrag';

export function Pet() {
  const [guide, setGuide] = useState(false);
  const [needsGuide, setNeedsGuide] = useState(false);
  const [guideError, setGuideError] = useState('');
  const [chat, setChat] = useState(false);
  const [reading, setReading] = useState(false);
  const [phase, setPhase] = useState<ConversationPhase>('idle');
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const appearanceOpenRef = useRef(false);
  appearanceOpenRef.current = appearanceOpen;
  const [avatar, setAvatar] = useState<AvatarRecord>({ appearance: defaultAppearance });
  const [avatarReady, setAvatarReady] = useState(false);
  const [live2dFailed, setLive2dFailed] = useState(false);
  const mouth = useRef(0);
  const live2dError=useCallback(()=>{setLive2dFailed(true);setError('数字人加载失败，已恢复栖栖。请在角色与场景中检查模型，或重新选择 2D 数字人重试。');},[]);
  const saveAvatar = async (record: AvatarRecord) => {
    await avatarRecord(record); setAvatar(record); setLive2dFailed(false); setError('');
  };
  useEffect(() => {
    let disposed = false;
    void avatarRecord().then(record => {
      if (!disposed && record) setAvatar({ ...record, appearance: normalizeAppearance(record.appearance) });
    }).catch(() => { if (!disposed) setError('角色设置读取失败，暂用默认形象。'); })
      .finally(() => { if (!disposed) setAvatarReady(true); });
    return () => { disposed = true; };
  }, []);
  const [open, setOpen] = useState(false);
  const [hidden, setHidden] = useState(false);
  const [quiet, setQuiet] = useState(false);
  const presence = companionState({ hidden, quiet, open, phase: chat ? phase : 'idle' });
  const [ready, setReady] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState('慢慢来，我在这里。');
  const [error, setError] = useState('');
  const [offset, setOffset] = useState({ x:0, y:0 });
  const shell = useRef<HTMLDivElement>(null);
  const { dragging, ...dragEvents } = useSceneDrag({
    enabled: ready && !quiet && !hidden, offset, move: setOffset,
    nativeDrag: nativeDesktop ? () => petAction('drag') : undefined,
    onError: e => setError(errorMessage(e)),
  });
  const restore = useCallback(() => { setHidden(false); setQuiet(false); setOpen(false); setError(''); }, []);

  useEffect(() => {
    let disposed = false;
    const removers: (() => void)[] = [];
    async function initialize() {
      try {
        await client.getRuntimeInfo();
        if (nativeDesktop) {
          try {
            const completed: unknown = await invoke('guide_status');
            if (typeof completed !== 'boolean') throw Error('Invalid guide status');
            if (!disposed) setNeedsGuide(!completed);
          } catch {
            if (!disposed) { setNeedsGuide(true); setGuideError('未能读取引导状态，可能会在下次启动时再次出现。'); }
          }
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
    const blur = () => { if (!appearanceOpenRef.current) setOpen(false); };
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
  }, [open, quiet, hidden, error, guide, appearanceOpen, avatar.appearance.scene, avatar.appearance.opacity, avatar.appearance.renderer, live2dFailed]);

  async function act(action: 'hide' | 'quiet' | 'open_panel' | 'open_settings' | 'open_memory') {
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
  return <>
    {!nativeDesktop && <aside className="preview-desktop-note"><strong>栖伴 · 桌面角色预览</strong><p>这里模拟角色形态；真实透明悬浮、托盘和鼠标穿透请运行桌面版。</p><button onClick={restore}>恢复角色预览</button></aside>}
    <div ref={shell} {...dragEvents} data-scene={avatar.appearance.scene} data-renderer={avatar.appearance.renderer === 'live2d' && avatar.model && !live2dFailed ? 'live2d' : 'svg'} data-dragging={dragging} data-companion-state={presence} className={`pet-shell ${hidden ? 'pet-hidden' : ''} ${quiet ? 'pet-quiet' : ''}`} style={!nativeDesktop ? { transform:`translate(${offset.x}px, ${offset.y}px)` } : undefined}>
      {avatar.appearance.scene !== 'none' && avatar.appearance.opacity > 0 && <div data-pet-hit data-pet-drag style={{ opacity: avatar.appearance.opacity / 100 }} className="pet-scene" role="group" aria-label="栖栖的小天地，可拖动背景移动" title="拖动背景或栖栖来移动，轻点栖栖聊天">
        <span className="pet-scene-name">栖栖的小天地</span>
        <div className="pet-scene-window" aria-hidden="true"><i/><i/><i/></div>
        <span className="pet-scene-star" aria-hidden="true">✧</span>
        <div className="pet-stage" aria-hidden="true"/>
        <div className="pet-scene-plant" aria-hidden="true"><i/><i/><i/><b/></div>
        <span className="pet-scene-status">{quiet ? '安静陪伴中 · 托盘可唤回' : companionLabels[presence]}</span>
      </div>}
      {open && <section data-pet-hit className={appearanceOpen ? "pet-dialog pet-dialog-appearance" : reading || guide ? "pet-dialog pet-dialog-reading" : "pet-dialog"} aria-label="栖栖的交互气泡">
        <header><div className="pet-presence">{reading && <span className="pet-portrait" aria-hidden="true"><AvatarArtwork state={presence}/></span>}<strong>栖栖 <span>{companionLabels[presence]}</span></strong></div><button aria-label="收起气泡" onClick={() => setOpen(false)}>×</button></header>
        {error && <p role="alert" className="pet-error">{error}</p>}
        {appearanceOpen ? <AppearancePanel record={avatar} save={saveAvatar} close={() => setAppearanceOpen(false)}/> : guide ? <FirstUseGuide initialError={guideError} onSettings={() => void act('open_settings')} onLater={() => { setNeedsGuide(false); setGuide(false); }} onComplete={async () => { if (nativeDesktop) await invoke('guide_complete'); setGuideError(''); setNeedsGuide(false); setGuide(false); }}/>
        : <><div className="pet-mode"><button aria-pressed={!chat} onClick={()=>setChat(false)}>记待办</button><button aria-pressed={chat} onClick={()=>setChat(true)}>聊一聊</button><button disabled={!avatarReady} onClick={()=>setAppearanceOpen(true)}>角色与场景</button><button onClick={()=>void act('open_settings')}>模型设置</button><button onClick={()=>void act('open_memory')}>我们的记忆</button><button onClick={() => setGuide(true)}>使用指南</button></div>
        {chat ? <ChatBubble onPhase={setPhase} onReading={setReading} mouth={mouth}/> : <>
        <p className="pet-message" role="status">{note}</p>
        <form onSubmit={create}>
          <label className="sr-only" htmlFor="pet-draft">想记下什么？</label>
          <input id="pet-draft" value={draft} onChange={e => setDraft(e.target.value)} placeholder="想做的事，先记下来…" disabled={busy || !ready}/>
          <button className="pet-save" disabled={busy || !ready || !draft.trim()}>{busy ? '保存中…' : '记下来'}</button>
        </form>
        <p className="pet-limit">仅保存待办 · 不自动执行</p></>}

        <div className="pet-actions">
          <button onClick={() => void act('open_panel')}>任务面板 ↗</button>
          <button onClick={() => void act('quiet')}>安静陪伴</button>
          <button onClick={() => void act('hide')}>隐藏</button>
        </div></>}
      </section>}
      <button data-pet-hit data-pet-drag className="pet-character" aria-label="和栖栖互动" aria-expanded={open} disabled={quiet || !ready} onClick={() => { if (!open) setGuide(needsGuide); setOpen(value => !value); setNote('慢慢来，我在这里。'); }}>
        {avatar.appearance.renderer === 'live2d' && avatar.model && !live2dFailed && !hidden ? <Live2DRenderer active={!quiet} state={presence} bundle={avatar.model} onError={live2dError}/> : <AvatarArtwork state={presence}/>}
      </button>
      {!open && error && <button data-pet-hit className="pet-error-reopen" onClick={() => setOpen(true)}>操作未完成，点击查看</button>}
    </div>
  </>;
}
