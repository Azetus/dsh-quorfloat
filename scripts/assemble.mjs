#!/usr/bin/env node
/**
 * The shared staging rule for the sidecar executable.
 *
 * Development and release use the same copy and executable-permission handling.
 * The caller supplies the directory: a throwaway sidecar directory in development,
 * or the bundle's bin directory for release.
 */

import { chmodSync, cpSync, existsSync, mkdirSync, rmSync, statSync } from 'node:fs'
import { basename, join } from 'node:path'

/**
 * Whether a staged path is macOS directory metadata, rather than a distribution asset.
 *
 * Uses the platform's path parser: both copy callbacks and directory walkers supply native
 * paths. Match only known metadata names, so legitimate hidden assets remain eligible.
 *
 * @param path - a file or directory path.
 * @returns whether the path must be excluded from copies and distribution inventories.
 */
export function isBundleJunk(path) {
  const name = basename(path)
  return name === '.DS_Store' || name.startsWith('._') || name === '__MACOSX'
}

/** The executable name, matching what the host half resolves per platform. */
export const SIDECAR_NAME = process.platform === 'win32' ? 'dsh-quorfloat.exe' : 'dsh-quorfloat'

/**
 * Copy a built executable into the staged layout.
 *
 * The previous executable is removed rather than overwritten in place: on Windows a
 * running sidecar holds the file open, and that failure has to read as "stop the
 * instance", not as an `EBUSY` from the middle of a copy.
 *
 * @param options.source - the built executable to stage.
 * @param options.binaryDirectory - where the executable is written.
 * @param options.label - the message prefix, `dev` or `release`.
 * @returns the staged executable path.
 * @throws {Error} when the existing executable cannot be replaced.
 */
export function stageSidecar({
  source,
  binaryDirectory,
  label = 'dev',
}) {
  mkdirSync(binaryDirectory, { recursive: true })
  const destination = join(binaryDirectory, SIDECAR_NAME)
  if (existsSync(destination)) {
    try {
      rmSync(destination, { force: true })
    } catch {
      throw new Error(`cannot replace the staged sidecar; stop the existing ${label} instance and retry`)
    }
  }
  cpSync(source, destination, { force: true })
  makeExecutable(destination)
  return { path: destination }
}

/**
 * Set the executable bit on the staged file.
 *
 * `cpSync` already carries the source's mode, but the bit is set explicitly because a
 * bundle assembled from an archive or a downloaded artifact would otherwise ship an
 * executable nobody can run — and the host half reports that as "not executable"
 * rather than as a packaging fault.
 *
 * Windows is skipped outright: there is no executable bit to set, the host half's
 * `accessSync(path, X_OK)` is an existence check there, and so the mode is not part
 * of what makes the staged sidecar launchable. The call is also non-fatal on the
 * platforms that do have the bit: a filesystem that cannot express it (exFAT, a
 * network share, a read-only mount) must not turn "stage the sidecar" into a build
 * failure, and the mode copied by `cpSync` is still in place.
 *
 * @param path - the staged executable.
 */
function makeExecutable(path) {
  if (process.platform === 'win32') return
  try {
    chmodSync(path, statSync(path).mode | 0o111)
  } catch (error) {
    process.stderr.write(
      `cannot set the executable bit on ${path}: ${error instanceof Error ? error.message : String(error)}\n`,
    )
  }
}
