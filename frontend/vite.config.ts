import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import viteCompression from 'vite-plugin-compression';
import path from 'path';

/**
 * A `test` for the modules of these npm packages (pnpm keeps every package under a `node_modules/<name>/` directory) and of
 * these files of `src`: the app's own wrappers of a library go with it, or the `shared` chunk would import the library and the
 * shell would preload it (`<link rel="modulepreload">` in index.html follows the static imports of the entry).
 */
const group = (packages: string[], sources: string[] = []) =>
  new RegExp(`[\\\\/]node_modules[\\\\/](${packages.join('|')})[\\\\/]` + (sources.length > 0 ? `|[\\\\/]src[\\\\/](${sources.join('|')})$` : ''));

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
        // The pages are chunks of their own (router.tsx). The two heavy libraries get a chunk each, with the app's wrappers of
        // them, loaded with the first page that uses them: recharts with its d3 and redux dependencies (dashboard, report,
        // account, commodity and query pages) and CodeMirror (raw editor, query page). Only the listed packages: a captured
        // module's own dependencies (react, clsx, ...) stay where they are shared, or the shell would load the whole group.
        // The rest of npm is one `vendor` chunk and the app code that several pages share one `shared` chunk, instead of one
        // chunk per shared module (the server speaks HTTP/1.1: every file is a round trip).
        advancedChunks: {
          includeDependenciesRecursively: false,
          groups: [
            {
              name: 'charts',
              priority: 2,
              test: group(
                [
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
                ],
                [
                  'components/ui/chart\\.tsx',
                  'components/ReportGraph\\.tsx',
                  'components/AccountBalanceHistoryGraph\\.tsx',
                  'components/query/QueryResultChart\\.tsx',
                ],
              ),
            },
            {
              name: 'codemirror',
              priority: 2,
              test: group(
                ['@codemirror/[a-z-]+', '@lezer/[a-z-]+', '@uiw/[a-z-]+', 'codemirror', 'style-mod', 'w3c-keyname', 'crelt', '@marijn/find-cluster-break'],
                ['components/SingleFileEdit\\.tsx', 'components/query/QueryEditor\\.tsx', 'components/query/errorRange\\.ts'],
              ),
            },
            { name: 'vendor', priority: 1, test: /[\\/]node_modules[\\/]/ },
            { name: 'shared', priority: 0, test: /[\\/]src[\\/]/, minShareCount: 2 },
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
