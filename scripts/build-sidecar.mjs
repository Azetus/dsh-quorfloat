#!/usr/bin/env node
/**
 * Build the sidecar executable for the machine this runs on.
 *
 * Two steps, in this order, and the order is not cosmetic: `tauri::generate_context!`
 * embeds `quorfloat/frontend/dist` into the executable at compile time, so cargo must
 * not run before the frontend does or the sidecar serves whatever the last `dist/`
 * happened to contain. The frontend is a compile-time input, so this unit builds it
 * itself rather than trusting a caller to have run `build:frontend` first.
 *
 * The cargo half is `scripts/cargo.mjs`, shared with `scripts/release.mjs`, so a
 * debug build and a release slice cannot drift in their command line or artifact path.
 * `--release` selects the same helper's release profile; the default is debug, which
 * is what the local full build `build:local` uses. The publishable `build` does not
 * come through here: it builds a release slice per target in `scripts/release.mjs`.
 *
 * This script only builds: it stages nothing. `scripts/dev.mjs` copies the result into
 * a throwaway home, and `scripts/release.mjs` assembles a bundle around it.
 *
 * Usage:
 *   node scripts/build-sidecar.mjs [--release]
 *   node scripts/build-sidecar.mjs --help
 */

import { existsSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

import { buildSidecar, hostTriple } from './cargo.mjs'
import { runNpm } from './release-shared.mjs'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))

/** Parse the options this script owns. */
function parseArgs(argv) {
  const options = { release: false, help: false }
  for (let index = 0; index < argv.length; index += 1) {
    switch (argv[index]) {
      case '--help':
      case '-h':
        options.help = true
        break
      case '--release':
        options.release = true
        break
      default:
        throw new Error(`unknown option ${JSON.stringify(argv[index])}; try --help`)
    }
  }
  return options
}

/** Print usage. */
function usage() {
  process.stdout.write(`Build the sidecar executable for this machine.

Usage: node scripts/build-sidecar.mjs [options]

Options:
  --release    build the release profile (default: debug)
  -h, --help   this text

The panel frontend is built first, because cargo embeds quorfloat/frontend/dist at
compile time. The executable is left in quorfloat/target/<profile>/ and is not staged
anywhere; scripts/dev.mjs stages it for a development run and scripts/release.mjs
assembles bundles around it.
`)
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    usage()
    return
  }
  const profile = options.release ? 'release' : 'debug'

  process.stdout.write('build:sidecar: npm run build:frontend (cargo embeds the result in the sidecar)\n')
  runNpm(['run', 'build:frontend'])
  const dist = join(ROOT, 'quorfloat', 'frontend', 'dist', 'index.html')
  if (!existsSync(dist)) throw new Error(`the frontend build produced no ${relative(ROOT, dist)}`)

  const artifact = buildSidecar({
    root: ROOT,
    target: hostTriple(),
    release: options.release,
    label: 'build:sidecar',
  })
  process.stdout.write(`build:sidecar: ${profile} sidecar at ${relative(ROOT, artifact)}\n`)
}

try {
  main()
} catch (error) {
  process.stderr.write(`build:sidecar: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
}
