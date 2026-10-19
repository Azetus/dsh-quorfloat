/**
 * The running DeepSeek Harness version, from the package DSH itself calls the authority.
 *
 * DSH does not export a version environment variable. Its own
 * `getDshRuntimeVersion()` reads `@deepseek-ai/dsh-app-boot/package.json`, and that is the
 * value the plugin-compatibility gate compares against `engines.dsh`. Resolving the same
 * manifest from the DSH entry point therefore keeps the diagnostics line and the install gate
 * on one definition instead of inventing a second one.
 *
 * The resolution must start at the running launcher, not at this plugin: a profile's
 * `node_modules` can contain an old app-boot from a previous install (or an unrelated `npx`
 * copy), while the launcher's own path points at the runtime that is actually running.
 */

import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { resolve as resolvePath } from 'node:path'

/** The manifest DSH treats as its runtime version source. */
const APP_BOOT_MANIFEST = '@deepseek-ai/dsh-app-boot/package.json'

let cached: string | undefined

/**
 * Read the running Harness version.
 *
 * Falls back to `DSH_VERSION` when the runtime package cannot be resolved, and to `unknown`
 * when neither source answers. The value is cached for the life of the process.
 *
 * @returns the runtime version, or `unknown`.
 */
export function dshRuntimeVersion(): string {
  if (cached === undefined) cached = resolve()
  return cached
}

/** Resolve the version once, trying the launcher before this module's own location. */
function resolve(): string {
  const candidates: string[] = []
  if (typeof process.argv[1] === 'string' && process.argv[1] !== '') {
    candidates.push(resolvePath(process.argv[1]))
  }
  candidates.push(import.meta.url)

  for (const from of candidates) {
    try {
      const require = createRequire(from)
      const manifestPath = require.resolve(APP_BOOT_MANIFEST)
      const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as { version?: unknown }
      if (typeof manifest.version === 'string' && manifest.version.trim() !== '') {
        return manifest.version.trim()
      }
    } catch {
      // A packaged or embedded runtime may not expose the package this way; try the next
      // source rather than turning a diagnostics field into a startup failure.
    }
  }

  const fromEnv = process.env['DSH_VERSION']
  return typeof fromEnv === 'string' && fromEnv !== '' ? fromEnv : 'unknown'
}
