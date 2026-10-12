import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The panel is one fixed viewport rendered from the shell's state snapshots.
// It uses system fonts and has no public asset directory.
export default defineConfig({
  plugins: [react()],
  publicDir: false,
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
