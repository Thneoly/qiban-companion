import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { SurfaceRouter } from './SurfaceRouter';
import { nativeDesktop } from './lib/surface';
import './styles.css';
import './features/pet/pet.css';

// The packaged app is a product, not a web page: on the desktop surface the
// WebView's default context menu (reload / save / print) is suppressed in
// both windows. Browser preview keeps it for devtools while developing.
if (nativeDesktop) {
  document.addEventListener('contextmenu', event => event.preventDefault());
}

createRoot(document.getElementById('root')!).render(<StrictMode><SurfaceRouter/></StrictMode>);
