/** Select existing JS suites by execution environment, without test-level skips. */
import { spawnSync } from 'node:child_process'
import { readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = dirname(dirname(fileURLToPath(import.meta.url)))

function suiteFiles(directory) {
  const files = readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith('.test.mjs'))
    .map(entry => join(directory, entry.name))
    .sort()
  if (files.length === 0) throw new Error(`no test files found in ${directory}`)
  return files
}

try {
  const [scope = 'all', ...options] = process.argv.slice(2)
  if (!['unit', 'desktop', 'all'].includes(scope) || options.some(option => option !== '--tap')) {
    throw new Error('usage: node scripts/test.mjs unit|desktop|all [--tap]')
  }
  const files = [
    ...(scope === 'desktop' ? [] : suiteFiles(join(root, 'tests'))),
    ...(scope === 'unit' ? [] : suiteFiles(join(root, 'tests', 'desktop'))),
  ]
  process.stdout.write(`tests: scope=${scope}, files=${files.length}; desktop integration ${scope === 'unit' ? 'excluded' : 'included'}\n`)
  const result = spawnSync(process.execPath, [
    '--test',
    '--test-timeout=30000',
    ...(options.includes('--tap') ? ['--test-reporter=tap'] : []),
    ...files,
  ], { cwd: root, stdio: 'inherit', shell: false })
  if (result.error) throw result.error
  if (result.signal) throw new Error(`test runner terminated by ${result.signal}`)
  process.exitCode = result.status ?? 1
} catch (error) {
  process.stderr.write(`tests: ${error.message}\n`)
  process.exitCode = 1
}
