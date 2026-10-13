#!/usr/bin/env node
/**
 * Extract DeepSeek Harness' generated Remote contract from the installed app.
 *
 * The host plugin calls `sessionController` directly, so the request and result
 * shapes it must produce are exactly the ones the running Harness validates with
 * Zod. Those validators are generated into `typert.host.js` inside `app.asar`.
 *
 * This script copies that one file into `.tooling/conformance/` so
 * `tests/conformance.test.mjs` can validate the plugin's real payloads against
 * the real schemas — without a running dsh and without vendoring Harness code
 * into the repository.
 *
 * Usage: node tests/fixtures/extract-harness-contract.mjs
 *
 * The asar format is: 16-byte header record, then a JSON directory, then file
 * data. The header JSON is *padded* to a 4-byte boundary, and the data section
 * starts after the padding — reading `16 + headerSize` lands a few bytes early
 * and silently prepends the tail of the previous entry. That off-by-alignment
 * is why this file parses the alignment bytes explicitly instead of trusting
 * `headerSize` alone.
 */

import { mkdirSync, openSync, readSync, closeSync, writeFileSync, existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Default location of the installed app's `app.asar`.
 *
 * Only macOS has a default that can be named without guessing: the app bundle.
 * No Windows or Linux install path is hardcoded here, because none can be
 * verified from this repository — there `DSH_APP_ASAR` is required, and the
 * failure below names the platform and the variable instead of a wrong path.
 */
const DEFAULT_ASAR = process.platform === 'darwin'
  ? '/Applications/DeepSeek Harness.app/Contents/Resources/app.asar'
  : undefined
const APP = process.env['DSH_APP_ASAR'] ?? DEFAULT_ASAR
const TARGET = join('dsh', 'node_modules', '@deepseek-ai', 'dsh-api-session-controller', 'lib', 'typert.host.js')
const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '.tooling', 'conformance')

/** Read the asar directory for the installed app. */
function readArchive(asarPath) {
  const fd = openSync(asarPath, 'r')
  try {
    const head = Buffer.alloc(16)
    readSync(fd, head, 0, 16, 0)
    const headerSize = head.readUInt32LE(12)
    // The JSON header is padded so the data section starts 4-byte aligned; reading
    // `16 + headerSize` lands early and prepends the tail of the previous entry.
    const padding = (4 - ((16 + headerSize) % 4)) % 4
    const dataStart = 16 + headerSize + padding
    const headerBytes = Buffer.alloc(headerSize)
    readSync(fd, headerBytes, 0, headerSize, 16)
    const text = headerBytes.toString('utf8')
    const directory = JSON.parse(text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1))
    return { fd, directory, dataStart }
  } catch (error) {
    closeSync(fd)
    throw error
  }
}

/** Walk the directory tree to one entry. */
function lookup(directory, parts) {
  let node = directory
  for (const part of parts) {
    node = node?.files?.[part]
    if (node === undefined) return undefined
  }
  return node
}

/** Read one file out of the archive. */
function readEntry(fd, entry, dataStart) {
  const buffer = Buffer.alloc(Number(entry.size))
  readSync(fd, buffer, 0, buffer.length, dataStart + Number(entry.offset))
  return buffer.toString('utf8')
}

/** Write the generated contract next to the zod it imports. */
function main() {
  if (APP === undefined || !existsSync(APP)) {
    const problem = APP === undefined
      ? `DSH_APP_ASAR is not set and there is no default for ${process.platform}`
      : `not found: ${APP} (${process.platform})`
    console.error(
      `${problem}\n` +
        'Set DSH_APP_ASAR to the app.asar path inside the installed DeepSeek Harness ' +
        '(on macOS the default is /Applications/DeepSeek Harness.app/Contents/Resources/app.asar).\n' +
        (process.platform === 'darwin'
          ? 'The conformance tests skip while the contract is missing; that skip is not a green signal.'
          : `Without it the conformance tests cannot skip on ${process.platform}; they fail, ` +
            'because "not verifiable here" must not read as green.'),
    )
    process.exit(1)
  }
  const { fd, directory, dataStart } = readArchive(APP)
  try {
    const entry = lookup(directory, TARGET.split('/'))
    if (entry === undefined) {
      console.error(`not found in the archive: ${TARGET}`)
      process.exit(1)
    }
    const source = readEntry(fd, entry, dataStart)
    if (!source.includes('export const TYPERT')) {
      console.error('the extracted file does not look like a Typert manifest; refusing to write it')
      process.exit(1)
    }
    mkdirSync(OUT_DIR, { recursive: true })
    // This directory owns the generated ES module on every platform, regardless
    // of the package type used by the surrounding developer toolchain.
    writeFileSync(join(OUT_DIR, 'package.json'), JSON.stringify({ type: 'module' }) + '\n')
    const outFile = join(OUT_DIR, 'typert.host.js')
    writeFileSync(outFile, source)
    const invocations = [...source.matchAll(/id: '([^']+)#([^']+)'/g)].map(match => `${match[1]}#${match[2]}`)
    console.log(`wrote ${outFile} (${Buffer.byteLength(source)} bytes, ${invocations.length} invocations)`)
    console.log('note: this file is generated and ignored by git; re-run after upgrading Harness.')
  } finally {
    closeSync(fd)
  }
}

main()
