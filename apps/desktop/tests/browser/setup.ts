import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';

// Own Vite in-process so teardown does not depend on Windows shell process-tree termination.
export default async function setup() {
  const root = fileURLToPath(new URL('../../', import.meta.url));
  const server = await createServer({ root, server: { host: '127.0.0.1', port: 1430, strictPort: true } });
  await server.listen();
  return async () => { await server.close(); };
}
