// Category 1 (cross-process interface): every Tauri command the webview invokes is registered by the shell.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { test } from 'node:test'

import { repoRoot } from './helpers.mjs'

test('every command the page invokes is one the shell registers', async () => {
  // Two halves of one wire with nothing else checking them: TypeScript compiles the
  // frontend against a string, and the Rust handler list is just a macro invocation, so a
  // rename on either side is a panel whose answer silently never arrives. The webview's
  // language report is the newest and the least visible of these — when it is lost, the
  // panel simply keeps the `en` it started with.
  const api = await readFile(join(repoRoot, 'quorfloat/frontend/src/api.ts'), 'utf8')
  const rust = await readFile(join(repoRoot, 'quorfloat/src/main.rs'), 'utf8')
  const invoked = [...api.matchAll(/invoke\('([a-z_]+)'/g)].map(match => match[1])
  const registered = [...rust.matchAll(/generate_handler!\[([\s\S]*?)\]/g)]
    .flatMap(match => match[1].split(','))
    .map(entry => entry.trim())
    .filter(entry => entry !== '')

  assert.ok(invoked.includes('report_languages'), 'the language report has a wrapper to go through')
  const missing = invoked.filter(name => !registered.includes(name))
  assert.deepEqual(missing, [], 'every invoked command is registered by the shell')
})
