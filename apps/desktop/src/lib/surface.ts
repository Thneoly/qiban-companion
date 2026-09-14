import { isTauri, invoke } from '@tauri-apps/api/core';
export const nativeDesktop = isTauri();
export type Surface = 'pet' | 'panel';
export function currentSurface(): Surface { return new URLSearchParams(location.search).get('view') === 'panel' ? 'panel' : 'pet'; }
export function navigatePreview(surface: Surface) {
  history.pushState(null, '', surface === 'panel' ? '?view=panel' : location.pathname);
  dispatchEvent(new PopStateEvent('popstate'));
}
export async function petAction(action: 'ready' | 'open_panel' | 'open_settings' | 'hide' | 'quiet' | 'restore' | 'drag') {
  if (nativeDesktop) await invoke('pet_action', { action });
  else if(action==='open_settings'){navigatePreview('panel');setTimeout(()=>document.getElementById('model-settings-title')?.scrollIntoView(),100);}
  else if (action === 'open_panel') navigatePreview('panel');
}
