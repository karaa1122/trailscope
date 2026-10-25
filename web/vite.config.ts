import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The Logbox server the dev server proxies to (see scripts/dev/local.env).
const api = process.env.LOGBOX_API_URL ?? 'http://localhost:8090';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': api,
      '/v1': api,
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
  },
  test: {
    environment: 'jsdom',
  },
});
