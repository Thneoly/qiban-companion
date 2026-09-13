import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests/browser',
  globalSetup: './tests/browser/setup.ts',
  fullyParallel: false,
  use: { baseURL: 'http://127.0.0.1:1430', channel: 'msedge', viewport: { width: 1280, height: 960 } },
});
