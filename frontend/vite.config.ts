import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import viteCompression from 'vite-plugin-compression';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';

/** A `test` for the modules of these npm packages (pnpm keeps every package under a `node_modules/<name>/` directory). */
const packages = (...names: string[]) => new RegExp(`[\\\\/]node_modules[\\\\/](${names.join('|')})[\\\\/]`);

/** The libraries that get a chunk of their own; an app module that imports one of them is never put in the `shared` chunk. */
const CHUNKED_LIBRARY_IMPORT = /from ['"](recharts|@codemirror\/|@uiw\/|codemirror)/;
const importsChunkedLibrary = (id: string) => existsSync(id) && CHUNKED_LIBRARY_IMPORT.test(readFileSync(id, 'utf8'));
const APP_MODULE = /[\\/]src[\\/].*\.tsx?$/;

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
  build: {
    rolldownOptions: {
      output: {
        // The pages are chunks of their own (router.tsx). The two heavy libraries get a chunk each, loaded with the first
        // page that uses them: recharts with its d3 and redux dependencies (dashboard, report, account, commodity and query
        // pages) and CodeMirror (raw editor, query page). Only the listed packages: a captured module's own dependencies
        // (react, clsx, ...) stay where they are shared, or the shell would load the whole group. The rest of npm is one
        // `vendor` chunk, and the app code that several pages share is one `shared` chunk instead of one chunk per shared
        // module (the server speaks HTTP/1.1: every file is a round trip). An app module that imports a chunked library
        // stays out of `shared`, so only the pages that draw a chart or edit text import the library; a module in `shared`
        // is a static import of the entry, and index.html would preload the library for every page.
        advancedChunks: {
          includeDependenciesRecursively: false,
          groups: [
            {
              name: 'charts',
              priority: 2,
              test: packages(
                'recharts',
                'victory-vendor',
                'd3-[a-z-]+',
                'internmap',
                'decimal\\.js-light',
                '@reduxjs/toolkit',
                'redux',
                'react-redux',
                'reselect',
                'immer',
                'es-toolkit',
                'eventemitter3',
                'fast-equals',
              ),
            },
            {
              name: 'codemirror',
              priority: 2,
              test: packages(
                '@codemirror/[a-z-]+',
                '@lezer/[a-z-]+',
                '@uiw/[a-z-]+',
                'codemirror',
                'style-mod',
                'w3c-keyname',
                'crelt',
                '@marijn/find-cluster-break',
              ),
            },
            { name: 'vendor', priority: 1, test: /[\\/]node_modules[\\/]/ },
            { name: 'shared', priority: 0, minShareCount: 2, test: (id: string) => APP_MODULE.test(id) && !importsChunkedLibrary(id) },
          ],
        },
      },
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
