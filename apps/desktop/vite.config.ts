import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig(({ mode }) => ({
  // Installer artifacts contain only reviewed source assets, never local research downloads.
  publicDir: mode === 'installer' ? false : 'public',
  plugins: [react()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**', '**/target/**'] } },
  build: { target: 'es2022' },
}));
