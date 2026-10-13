/** Version selection and existing test suites for the release workflow. */
import { spawnSync } from 'node:child_process'
import { appendFileSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

import { buildSidecar } from './cargo.mjs'
import { MANIFEST, ROOT, packageStem, runNpm } from './release-shared.mjs'
import { assertVersionsAgree } from './version.mjs'

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

function test() {
  const require = createRequire(pathToFileURL(join(ROOT, '.tooling', 'ci', 'package.json')))
  const dependencies = {
    DSH_CORDIS_PATH: require.resolve('@deepseek-ai/cordis'),
    DSH_YAML_PATH: require.resolve('yaml'),
    DSH_HARNESS_CONTRACT: require.resolve('@deepseek-ai/dsh-api-session-controller/typert'),
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
    // JS protocol tests prefer the debug executable; build the current version first.
    buildSidecar({ root: ROOT, label: 'ci:test' })
    run('cargo', ['test', '--locked', '--manifest-path', 'quorfloat/Cargo.toml'])
    runNpm(['--prefix', 'quorfloat/frontend', 'run', 'typecheck'])
    runNpm(['run', 'test:frontend'])
    // TAP includes assertion details as each test finishes, even if a leaked
    // process prevents Node from reaching the final suite summary.
    run(process.execPath, ['--test', '--test-reporter=tap', '--test-timeout=30000', 'tests/*.test.mjs'])
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
  else if (command === 'test') test()
  else throw new Error('usage: node scripts/ci.mjs version|test')
} catch (error) {
  process.stderr.write(`ci: ${error.message}\n`)
  process.exitCode = 1
}
