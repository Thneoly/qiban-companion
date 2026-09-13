import { useEffect, useState } from 'react';
import App from './App';
import { Pet } from './features/pet/Pet';
import { currentSurface, nativeDesktop, navigatePreview } from './lib/surface';

export function SurfaceRouter() {
  const [surface, setSurface] = useState(currentSurface);
  useEffect(() => {
    const update = () => setSurface(currentSurface());
    addEventListener('popstate', update);
    return () => removeEventListener('popstate', update);
  }, []);
  document.documentElement.dataset.surface = surface;
  document.documentElement.dataset.runtime = nativeDesktop ? 'desktop' : 'preview';
  return surface === 'pet' ? <Pet/> : <>
    {!nativeDesktop && <button className="return-to-pet" onClick={() => navigatePreview('pet')}>← 回到桌面角色预览</button>}
    <App/>
  </>;
}
