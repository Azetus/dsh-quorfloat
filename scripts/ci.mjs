/** Version selection and existing test suites for the release workflow. */
import { spawnSync } from 'node:child_process'
import { appendFileSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

import { MANIFEST, ROOT, packageStem, runNpm } from './release-shared.mjs'
import { assertVersionsAgree } from './version.mjs'

/**
 * Dependencies the existing upstream test suites resolve from `.tooling/ci`.
 *
 * Pinned here, not in the workflow, so the local install and CI install cannot
 * drift; the workflow calls `npm run ci:install`.
 */
const CI_DEPENDENCIES = [
  '@deepseek-ai/cordis@4.0.5-alpha.1',
  'yaml@2.9.1',
  '@deepseek-ai/dsh-api-session-controller@0.2.1-alpha.1',
]

function version() {
  const value = assertVersionsAgree(ROOT)
  if (!/^\d+\.\d+\.\d+$/u.test(value)) throw new Error('this workflow supports stable x.y.z releases only')
  if (process.env.GITHUB_EVENT_NAME === 'release') {
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'))
    if (event.release.tag_name !== `v${value}`) {
      throw new Error(`Release tag must be v${value}, got ${event.release.tag_name}`)
    }
  }
  if (process.env.GITHUB_OUTPUT) {
    const outputs = {
      version: value,
      main: packageStem(MANIFEST.name),
      offline: MANIFEST.name.slice(MANIFEST.name.lastIndexOf('/') + 1),
      darwin: packageStem(MANIFEST.quorfloat.platformPackages.darwin),
      win32: packageStem(MANIFEST.quorfloat.platformPackages['win32-x64']),
    }
    appendFileSync(process.env.GITHUB_OUTPUT,
      Object.entries(outputs).map(([key, entry]) => `${key}=${entry}\n`).join(''))
  }
  process.stdout.write(`ci: version ${value}\n`)
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, stdio: 'inherit', shell: false })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} failed (exit ${result.status ?? result.signal})`)
}

/** Install the exact upstream packages `ci:test` resolves outside the repo. */
function install() {
  runNpm([
    'install',
    '--prefix', '.tooling/ci',
    '--cache', '.tooling/.npm-cache',
    '--no-save',
    '--package-lock=false',
    '--ignore-scripts',
    '--legacy-peer-deps',
    '--no-audit',
    '--no-fund',
    ...CI_DEPENDENCIES,
  ])
  process.stdout.write('ci: installed upstream test dependencies in .tooling/ci\n')
}

function test() {
  let dependencies
  try {
    const require = createRequire(pathToFileURL(join(ROOT, '.tooling', 'ci', 'package.json')))
    dependencies = {
      DSH_CORDIS_PATH: require.resolve('@deepseek-ai/cordis'),
      DSH_YAML_PATH: require.resolve('yaml'),
      DSH_HARNESS_CONTRACT: require.resolve('@deepseek-ai/dsh-api-session-controller/typert'),
    }
  } catch (error) {
    throw new Error(
      'missing .tooling/ci dependencies; run `npm run ci:install` first ' +
        `(${error instanceof Error ? error.message : String(error)})`,
    )
  }
  const state = mkdtempSync(join(tmpdir(), 'quorfloat-ci-tests-'))
  const overrides = {
    ...dependencies,
    DSH_QUORFLOAT_PINNED: join(state, 'pinned.json'),
    DSH_QUORFLOAT_PREFERENCES: join(state, 'preferences.json'),
    DSH_QUORFLOAT_WINDOW_STATE: join(state, 'window.json'),
    DSH_QUORFLOAT_RUST_MARKER: join(state, 'marker.log'),
  }
  const original = new Map(Object.keys(overrides).map(key => [key, process.env[key]]))
  Object.assign(process.env, overrides)
  try {
    // The publish gate excludes desktop integration: it never launches the real
    // Tauri sidecar. Release executables are built by the preceding package steps.
    run('cargo', ['test', '--locked', '--manifest-path', 'quorfloat/Cargo.toml'])
    runNpm(['--prefix', 'quorfloat/frontend', 'run', 'typecheck'])
    runNpm(['run', 'test:frontend'])
    run(process.execPath, ['scripts/test.mjs', 'unit', '--tap'])
  } finally {
    for (const [key, value] of original) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
    rmSync(state, { recursive: true, force: true })
  }
}

try {
  const command = process.argv[2]
  if (command === 'version') version()
  else if (command === 'install') install()
  else if (command === 'test') test()
  else throw new Error('usage: node scripts/ci.mjs version|install|test')
} catch (error) {
  process.stderr.write(`ci: ${error.message}\n`)
  process.exitCode = 1
}
