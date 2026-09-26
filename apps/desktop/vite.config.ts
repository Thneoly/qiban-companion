import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import { readFileSync } from 'node:fs';
import runtime from './scripts/live2d-runtime-manifest.json' with { type: 'json' };

export default defineConfig(({ mode }) => ({
  // Installer artifacts contain only reviewed source assets, never local research downloads.
  publicDir: mode === 'installer' ? false : 'public',
  plugins: [react(), ...(mode === 'installer' ? [{
    name: 'reviewed-live2d-runtime',
    generateBundle() {
      for (const item of runtime) this.emitFile({ type: 'asset', fileName: 'live2d-runtime/' + item.file,
        source: readFileSync(new URL('./public/live2d-runtime/' + item.file, import.meta.url)) });
    },
  } satisfies Plugin] : [])],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**', '**/target/**'] } },
  build: { target: 'es2022' },
}));
