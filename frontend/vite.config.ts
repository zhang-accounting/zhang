import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import viteCompression from 'vite-plugin-compression';
import path from 'path';

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    viteCompression({
      verbose: true,
      disable: false,
      threshold: 10240,
      algorithm: 'gzip',
      ext: '.gz',
    }),
  ],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
    },
  },
  server: {
    port: 3000,
    // Dev only: every /api call (fetch, uploads, the /api/sse EventSource) is same-origin so the HttpOnly session cookie of the
    // login page is sent; forward them to the local zhang server.
    proxy: {
      '/api': process.env.VITE_API_ENDPOINT || 'http://localhost:8000',
    },
  },
});
