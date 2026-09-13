import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { SurfaceRouter } from './SurfaceRouter';
import './styles.css';
import './features/pet/pet.css';

createRoot(document.getElementById('root')!).render(<StrictMode><SurfaceRouter/></StrictMode>);
