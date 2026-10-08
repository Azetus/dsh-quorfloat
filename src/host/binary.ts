/**
 * Locating the quorfloat executable.
 *
 * Resolution order, and the reason for it:
 * 1. `config.quorfloatPath`, when set — an explicit absolute path wins so a
 *    developer can point at `quorfloat/target/release/...` without touching
 *    anything else, and so a user can work around a broken installation;
 * 2. the `DSH_QUORFLOAT_PATH` environment variable, for scripted runs and CI;
 * 3. the platform package convention `dsh-quorfloat-<platform>-<arch>/bin/...`,
 *    resolved through Node's module resolver from this package, which is what
 *    makes `optionalDependencies` + `os`/`cpu` work without code changes.
 *    On macOS the payload may also be the shipping `.app` bundle
 *    (`bin/<name>.app/Contents/MacOS/<name>`); a bare executable next to it
 *    still wins, because that is the developer's override hatch;
 * 4. well-known development locations inside this repository.
 *
 * A miss is never silent: the diagnostic names every candidate that was tried,
 * because "the window does not appear" must not be the only symptom of a
 * missing or failed platform package.
 */

import { accessSync, constants, closeSync, openSync, readFileSync, readSync, statSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { ChannelError } from '../bridge/errors.js'

/** Executable file name per platform. */
export const EXECUTABLE_NAME = process.platform === 'win32' ? 'dsh-quorfloat.exe' : 'dsh-quorfloat'

/** npm package suffix for the current platform, mirroring `os`/`cpu` package names. */
export const PLATFORM_PACKAGE = `dsh-quorfloat-${process.platform}-${process.arch}`

/** One resolution attempt, kept for diagnostics. */
export interface ResolutionAttempt {
  readonly source: 'config' | 'env' | 'platform-package' | 'dev-build'
  readonly candidate: string
  readonly ok: boolean
  readonly reason?: string
}

/** Outcome of resolving the executable. */
export interface ResolvedBinary {
  readonly path: string
  /**
   * Arguments passed before anything else. Empty for the shipped native
   * executable; non-empty for a development launcher (for example an
   * interpreter plus a script path), which keeps the spawn logic identical.
   */
  readonly args: readonly string[]
  readonly source: ResolutionAttempt['source']
  readonly attempts: readonly ResolutionAttempt[]
}

/** Caller-supplied resolution inputs. */
export interface BinaryLookup {
  /** `config.quorfloatPath`; empty string means "use the conventions". */
  readonly configuredPath: string
  /** Environment override; defaults to `process.env.DSH_QUORFLOAT_PATH`. */
  readonly envPath?: string | undefined
  /** Package root; defaults to the directory above this module. */
  readonly packageRoot?: string | undefined
}

/**
 * Resolve the quorfloat executable, or explain exactly why it could not be found.
 *
 * @param lookup - resolution inputs.
 * @returns the resolved path and the attempts that led to it.
 * @throws {ChannelError} with code `unavailable` when no candidate is executable.
 */
export function resolveQuorfloatBinary(lookup: BinaryLookup): ResolvedBinary {
  const attempts: ResolutionAttempt[] = []
  const packageRoot = lookup.packageRoot ?? findPackageRoot()
  const envPath = lookup.envPath ?? process.env['DSH_QUORFLOAT_PATH']

  const candidates: { source: ResolutionAttempt['source']; path: string | undefined }[] = []
  if (lookup.configuredPath !== '') {
    candidates.push({ source: 'config', path: resolvePath(lookup.configuredPath, packageRoot) })
  }
  if (typeof envPath === 'string' && envPath !== '') {
    candidates.push({ source: 'env', path: resolvePath(envPath, packageRoot) })
  }
  candidates.push({ source: 'platform-package', path: resolvePlatformPackage(packageRoot) })
  candidates.push({ source: 'dev-build', path: devBuildPath(packageRoot) })

  for (const candidate of candidates) {
    if (candidate.path === undefined) {
      attempts.push({ source: candidate.source, candidate: '(not resolved)', ok: false, reason: 'not found' })
      continue
    }
    const inspection = inspectCandidate(candidate.path, candidate.source)
    attempts.push(
      inspection.ok
        ? { source: candidate.source, candidate: candidate.path, ok: true }
        : { source: candidate.source, candidate: candidate.path, ok: false, reason: inspection.reason },
    )
    if (inspection.ok) {
      return { path: candidate.path, args: inspection.args, source: candidate.source, attempts }
    }
  }

  const summary = attempts
    .map(attempt => `  - [${attempt.source}] ${attempt.candidate}${attempt.reason === undefined ? '' : ` (${attempt.reason})`}`)
    .join('\n')
  throw new ChannelError(
    'unavailable',
    `quorfloat executable not found for ${process.platform}/${process.arch}. Tried:\n${summary}\n` +
      `Install the platform package "${PLATFORM_PACKAGE}", build it locally, or set config.quorfloatPath / DSH_QUORFLOAT_PATH.`,
    { attempts, platform: process.platform, arch: process.arch, platformPackage: PLATFORM_PACKAGE },
  )
}

/**
 * Locate this package's own root, independent of whether it is running from
 * `src/` or the compiled `lib/`.
 *
 * Deriving it from the module's depth was wrong: the compiled layout is one
 * directory deeper than the sources, so the development-artifact candidate
 * pointed at `lib/quorfloat/target/...` instead of `quorfloat/target/...`.
 * Asking Node for the package entry and walking up to its manifest is the one
 * answer that holds in both layouts.
 *
 * @returns the absolute package root.
 */
function findPackageRoot(): string {
  try {
    const require = createRequire(import.meta.url)
    let dir = dirname(require.resolve('dsh-quorfloat/package.json'))
    for (let depth = 0; depth < 5; depth += 1) {
      const manifest = join(dir, 'package.json')
      try {
        const parsed = JSON.parse(readFileSync(manifest, 'utf8')) as { name?: unknown }
        if (parsed.name === 'dsh-quorfloat') return dir
      } catch {
        // Keep walking up.
      }
      const parent = dirname(dir)
      if (parent === dir) break
      dir = parent
    }
  } catch {
    // `dsh-quorfloat/package.json` is not resolvable from here; fall back below.
  }
  // Sources live in `<root>/src/host`; the compiled output in `<root>/lib/host`.
  return dirname(dirname(dirname(fileURLToPath(import.meta.url))))
}

/**
 * Resolve an absolute or package-relative path.
 *
 * @param value - configured path.
 * @param packageRoot - base for relative paths.
 * @returns an absolute path.
 */
function resolvePath(value: string, packageRoot: string): string {
  return isAbsolute(value) ? value : resolve(packageRoot, value)
}

/**
 * Locate the executable inside the platform package, if that package is installed.
 *
 * The shipping form on macOS is a Tauri `.app` bundle, whose real executable is
 * `bin/<name>.app/Contents/MacOS/<name>`; the bare `bin/<name>` layout is the
 * legacy and developer-override form. The bare path is tried first so an
 * override never loses to the shipped bundle, and the first candidate that
 * exists wins so a package carrying only one form resolves without noise.
 *
 * @param packageRoot - this package's root, used as the resolution base.
 * @returns the candidate path, or `undefined` when the package is absent.
 */
function resolvePlatformPackage(packageRoot: string): string | undefined {
  const require = createRequire(join(packageRoot, 'package.json'))
  let manifest
  try {
    manifest = require.resolve(`${PLATFORM_PACKAGE}/package.json`)
  } catch {
    return undefined
  }
  const binDir = join(dirname(manifest), 'bin')
  const bare = join(binDir, EXECUTABLE_NAME)
  const bundled =
    process.platform === 'darwin'
      ? join(binDir, `${EXECUTABLE_NAME}.app`, 'Contents', 'MacOS', EXECUTABLE_NAME)
      : undefined
  const candidates = bundled === undefined ? [bare] : [bare, bundled]
  const existing = candidates.find(candidate => {
    try {
      statSync(candidate)
      return true
    } catch {
      return false
    }
  })
  // When neither form exists, report the bare path so the diagnostic names the
  // convention the package is expected to follow.
  return existing ?? bare
}

/**
 * Development fallback inside this repository.
 *
 * @param packageRoot - this package's root.
 * @returns the cargo release artifact path (checked for existence later).
 */
function devBuildPath(packageRoot: string): string {
  return join(packageRoot, 'quorfloat', 'target', 'release', EXECUTABLE_NAME)
}

/** Result of inspecting one candidate path. */
type Inspection =
  | { readonly ok: true; readonly args: readonly string[] }
  | { readonly ok: false; readonly reason: string }

/**
 * Decide whether a candidate can be launched, and how.
 *
 * An executable file is launched directly. A **non-executable script that names
 * its interpreter** (a `node` shebang) is launched through that interpreter.
 * This exists because the documented escape hatch — point
 * `config.quorfloatPath` / `DSH_QUORFLOAT_PATH` at a launcher — is used during
 * development with script files that have no executable bit, and refusing them
 * produced the least helpful failure available: "executable not found" for a
 * file sitting right there.
 *
 * @param path - absolute candidate path.
 * @param source - which rule produced the candidate, for the error text.
 * @returns how to launch it, or why it cannot be launched.
 */
function inspectCandidate(path: string, source: ResolutionAttempt['source']): Inspection {
  let stats
  try {
    stats = statSync(path)
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code
    return { ok: false, reason: code === 'ENOENT' ? 'does not exist' : `stat failed (${String(code ?? error)})` }
  }
  if (!stats.isFile()) return { ok: false, reason: 'not a regular file' }

  if (isExecutable(path)) return { ok: true, args: [] }

  // Not executable: only an explicit developer override may be a script. A
  // platform-package payload must be a real executable, or the package is broken
  // and should be reported as such.
  if (source === 'platform-package' || source === 'dev-build') return { ok: false, reason: 'not executable' }
  const interpreter = scriptInterpreter(path)
  if (interpreter === undefined) {
    return { ok: false, reason: 'not executable, and it does not declare a node interpreter' }
  }
  return { ok: true, args: [path] }
}

/**
 * Test the executable bit.
 *
 * @param path - absolute path.
 * @returns whether it can be executed directly.
 */
function isExecutable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK)
    return true
  } catch {
    return false
  }
}

/**
 * Read a leading `#!` line and decide whether it names Node.
 *
 * @param path - absolute path.
 * @returns `true` when the file is a Node script, `undefined` otherwise.
 */
function scriptInterpreter(path: string): true | undefined {
  let fd: number | undefined
  try {
    fd = openSync(path, 'r')
    const head = Buffer.alloc(256)
    const read = readSync(fd, head, 0, head.length, 0)
    const firstLine = head.subarray(0, read).toString('utf8').split('\n', 1)[0] ?? ''
    if (!firstLine.startsWith('#!')) return undefined
    return /(?:^|[\/\s])node(?:\.exe)?\s*$|(?:^|[\/\s])nodejs\s*$|env\s+node\b/.test(firstLine.trim())
      ? true
      : undefined
  } catch {
    return undefined
  } finally {
    if (fd !== undefined) closeSync(fd)
  }
}
