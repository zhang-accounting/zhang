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
  build: {
    // Vite 7+ defaults to "baseline widely available" (Chrome 111, Safari 16.4); keep the browser support of Vite 5's
    // "modules" default instead.
    target: ['es2020', 'edge88', 'firefox78', 'chrome87', 'safari14'],
  },
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
