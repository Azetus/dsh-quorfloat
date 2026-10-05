/**
 * The bundle patch is a user-facing contract, so it gets validated like one.
 *
 * `cordis.patch.yml` is what ships in the npm package and what a profile applies
 * to mount this plugin. A typo in a field name there would fail at install time
 * for every user, and the failure would look like "the plugin does nothing"
 * rather than "the bundle is malformed". These tests parse the shipped file and
 * push its configuration through the same schema the runtime uses.
 */

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { loadModule, repoRoot } from './helpers.mjs'

const { Config, DEFAULT_CONFIG } = await loadModule('config.js')

/** Parse the shipped patch with the real YAML parser. */
async function loadPatch() {
  const text = readFileSync(join(repoRoot, 'cordis.patch.yml'), 'utf8')
  const yaml = await importYaml()
  return { text, document: yaml.parse(text) }
}

/** Import a YAML parser from the Harness installation or the toolchain. */
async function importYaml() {
  const candidates = [
    join(process.env['HOME'] ?? '', '.dsh/profiles/node_modules/yaml/dist/index.js'),
    join(process.env['HOME'] ?? '', '.dsh/profiles/node_modules/js-yaml/index.js'),
  ]
  for (const candidate of candidates) {
    try {
      const module = await import(candidate)
      if (typeof module.parse === 'function') return module
      if (typeof module.default?.parse === 'function') return module.default
      if (typeof module.load === 'function') return { parse: module.load }
    } catch {
      // Try the next candidate.
    }
  }
  throw new Error('no YAML parser found; install one under ~/.dsh/profiles/node_modules or the toolchain')
}

test('the shipped patch is one insert targeting this plugin by name', async () => {
  const { document } = await loadPatch()
  assert.ok(Array.isArray(document), 'the patch is a top-level array of entries')
  assert.equal(document.length, 1)
  const entry = document[0]
  assert.ok(Array.isArray(entry.insert), 'the entry is an insert list')
  assert.equal(entry.insert.length, 1)
  const row = entry.insert[0]
  assert.equal(row.name, 'dsh-quorfloat', 'the row is addressed by package name so Node resolves it')
  assert.equal(row.id, 'quorfloat', 'the row id is the anchor users override')
})

test('the shipped patch passes the plugin configuration schema', async () => {
  const { document } = await loadPatch()
  const config = document[0].insert[0].config
  const result = Config['~standard'].validate(config)
  if ('issues' in result) {
    assert.fail(
      `cordis.patch.yml does not satisfy the config schema:\n` +
        result.issues.map(issue => `  - ${issue.message}${issue.path ? ` (at ${issue.path.join('.')})` : ''}`).join('\n'),
    )
  }
  // A patch replaces the whole `config`, so the shipped row must be complete:
  // every field is present, none of them left to an implicit default.
  assert.equal(result.value.enabled, true)
  assert.equal(result.value.window.width, 640)
  assert.equal(result.value.logLevel, 'info')
})

test('the shipped patch declares exactly the fields the schema knows', async () => {
  const { document } = await loadPatch()
  const declared = Object.keys(document[0].insert[0].config).sort()
  // Derived from the schema rather than listed here: a patch *replaces* the whole
  // config, so a field the schema knows but the patch omits silently falls back
  // to its default in every real install — a wording bug that no other test
  // would catch. Deriving keeps adding a field a one-file change.
  const known = Object.keys(DEFAULT_CONFIG).sort()
  assert.deepEqual(declared, known)
})

test('the package manifest points the loader at the patch and the built entry', async () => {
  const manifest = JSON.parse(readFileSync(join(repoRoot, 'package.json'), 'utf8'))
  assert.equal(manifest.type, 'module')
  assert.equal(manifest.main, 'lib/index.js', 'the loader resolves `main`')
  assert.equal(manifest.dsh?.bundle?.patch, './cordis.patch.yml')
  assert.ok(manifest.files.includes('lib'), 'the built output must be published')
  assert.ok(manifest.files.includes('cordis.patch.yml'), 'the patch must be published')
})

test('the package declares no runtime dependencies to resolve', async () => {
  const manifest = JSON.parse(readFileSync(join(repoRoot, 'package.json'), 'utf8'))
  assert.equal(manifest.dependencies, undefined, 'a runtime dependency would have to resolve inside the host process')
  // Peer dependencies on `@deepseek-ai/dsh*` are what trigger the runtime's
  // compatibility gate and its exact-version exemption flow; this plugin
  // deliberately avoids both.
  const peers = Object.keys(manifest.peerDependencies ?? {})
  assert.deepEqual(peers, [], 'no dsh peer dependencies: the plugin imports no Harness runtime values')
})
