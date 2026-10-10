#!/usr/bin/env node
/**
 * Build the browser half into the artifact the Harness client loader consumes.
 *
 * The loader does not import a plain ES module; it calls
 * `window.__ModuleLoader__.load({ id, factory })` and reads `module.exports` from
 * the factory. Official client plugins produce that shape with `tsdown` plus a
 * battery of plugins (CSS Modules, sourcemap chaining, a bundle-purity gate that
 * permits only declared module-table imports).
 *
 * This plugin's browser half is one compiled module that reads the DOM, calls the
 * host's gateway, and renders two slot entries through the module table's React,
 * so none of that machinery applies. Wrapping the compiled ES module is the whole
 * build:
 *
 * - every `require(...)` must be answerable from the loader's frozen module
 *   table — the platform seeds plus whatever `dsh.client.external` names — so a
 *   specifier nobody serves fails here instead of inside the page;
 * - no CSS means no stylesheet pipeline;
 * - `dsh.client.inject` still names the client packages whose services this half
 *   uses, which is what establishes load order.
 *
 * Usage: node scripts/build-client.mjs   (run after `tsc`)
 */

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))
const SOURCE = join(ROOT, 'lib', 'client', 'index.js')
const OUTPUT = join(ROOT, 'lib', 'client.js')

/** Package manifest: the loader id, and the externals this half declared. */
const PACKAGE = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'))

/** Package name, used as the loader id and in diagnostics. */
const PACKAGE_NAME = PACKAGE.name

/**
 * The platform singletons the shell seeds into the frozen module table.
 *
 * `PLATFORM_MODULES` in `@deepseek-ai/dsh-client-web/src/platform.ts` is the
 * authority; this copy exists because the build has no dependency on the
 * harness's own source. A specifier here needs no `dsh.client.external`
 * declaration and no graph row: the loader answers it from the seed table.
 */
const MODULE_TABLE_SEEDS = new Set([
  'react',
  'react/jsx-runtime',
  'react-dom',
  'react-dom/client',
  '@deepseek-ai/cordis',
  '@deepseek-ai/dsh-client-store',
  '@deepseek-ai/dsh-client-ui-slots',
  '@deepseek-ai/dsh-client-ui-primitives',
  '@deepseek-ai/dsh-client-ui-dockkit',
])

/**
 * Specifiers the manifest asks the loader graph to arrive before this bundle.
 *
 * @returns the declared externals, or an empty set when none were declared.
 */
function declaredExternals() {
  const external = PACKAGE.dsh?.client?.external
  return new Set(Array.isArray(external) ? external : [])
}

/** A bare specifier would need a module-table row; this half must have none. */
const BARE_IMPORT = /(?:^|[\s;{(])import\s*\(?\s*['"]([^'"]+)['"]|(?:^|[\s;{(])require\(\s*['"]([^'"]+)['"]/gu

/**
 * Verify every specifier the compiled module requests is one the module table can
 * answer.
 *
 * The loader would throw at runtime on a specifier its table cannot serve, and the
 * failure would surface as a broken plugin rather than a build error, so this is
 * checked here instead. Relative specifiers are fine: the artifact is a single
 * factory body, so a relative import resolves inside it, and `node:` is never
 * reachable from a page.
 *
 * @param code - the compiled ES module.
 * @throws {Error} when a specifier is neither a seed nor a declared external.
 */
function assertSelfContained(code) {
  const externals = declaredExternals()
  for (const match of code.matchAll(BARE_IMPORT)) {
    const specifier = match[1] ?? match[2]
    if (specifier === undefined || specifier.startsWith('.') || specifier.startsWith('node:')) continue
    if (MODULE_TABLE_SEEDS.has(specifier) || externals.has(specifier)) continue
    throw new Error(
      `client bundle: compiled code imports the bare specifier ${JSON.stringify(specifier)}. `
      + 'The browser half must be self-contained (or declare it in dsh.client.external).',
    )
  }
}

/** Build the loader artifact. */
function main() {
  let source
  try {
    source = readFileSync(SOURCE, 'utf8')
  } catch (error) {
    console.error(`client bundle: ${SOURCE} is missing; run the TypeScript build first`)
    throw error
  }
  assertSelfContained(source)

  // The compiled module is ESM; the loader expects a factory returning the module
  // namespace. TypeScript emits inline `export const` / `export function` for a
  // module like this one, so the keyword is stripped and the names are collected
  // into a trailing return.
  const exported = []
  const body = source
    .replace(/^\/\/# sourceMappingURL=.*$/mu, '')
    .replace(/^export\s+(?:declare\s+)?(const|let|var|function|class|async\s+function)\s+([A-Za-z_$][\w$]*)/gmu,
      (_whole, kind, name) => {
        exported.push(name)
        return `${kind} ${name}`
      })
    .replace(/^export\s*\{([^}]*)\};?\s*$/gmu, (_whole, names) => {
      for (const entry of names.split(',')) {
        const name = entry.trim().split(/\s+as\s+/u).pop()
        if (name !== undefined && name !== '') exported.push(name)
      }
      return ''
    })

  if (exported.length === 0) {
    throw new Error('client bundle: the compiled module exports nothing to hand to the loader')
  }
  const stripped = `${body}\nreturn { ${[...new Set(exported)].join(', ')} };\n`

  const artifact = [
    '/* Generated by scripts/build-client.mjs — do not edit. */',
    `window.__ModuleLoader__.load({ id: ${JSON.stringify(PACKAGE_NAME)}, factory: (require) => {`,
    'var module = { exports: {} }; var exports = module.exports;',
    stripped,
    'return module.exports; } });',
    '',
  ].join('\n')

  mkdirSync(dirname(OUTPUT), { recursive: true })
  writeFileSync(OUTPUT, artifact)
  console.log(`client bundle: wrote ${OUTPUT} (${Buffer.byteLength(artifact)} bytes)`)
}

main()
