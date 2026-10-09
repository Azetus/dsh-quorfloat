import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The panel's web frontend. Two things differ from a stock Vite project:
//
// - `publicDir` points at `quorfloat/assets` so the font (and its OFL licence,
//   fetched by `npm run fonts`) has one source of truth: it lands in `dist/fonts`
//   for both the dev server and the embedded build.
// - the app is not a page the user navigates: everything is one fixed viewport
//   that renders from the shell's `quorfloat/state` snapshots. The shell owns the
//   state, so React here is a renderer with local view state, not an app store.
export default defineConfig({
  plugins: [react()],
  publicDir: '../assets',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts'],
  },
})
