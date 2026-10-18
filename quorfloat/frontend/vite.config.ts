import { mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// esbuild spills transform inputs over 1 MiB into os.tmpdir() and deletes them
// straight after reading. In this workspace the files carry a Low integrity
// label on Windows, and a process starts at the integrity level of its image, so
// esbuild's service runs Low and cannot delete a Medium file in %TEMP%
// ("Access is denied"). Spilling inside the project keeps the temp file at the
// same level, and the same redirection keeps macOS and Linux builds consistent.
const tmp = join(dirname(fileURLToPath(import.meta.url)), '.tmp')
mkdirSync(tmp, { recursive: true })
process.env.TMPDIR = tmp
process.env.TEMP = tmp
process.env.TMP = tmp

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
