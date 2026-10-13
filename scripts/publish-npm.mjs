/** Publish the workflow's tarballs in dependency order, using npm OIDC. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

import { MANIFEST, ROOT, packageStem, runNpm } from './release-shared.mjs'
import { assertVersionsAgree } from './version.mjs'

const REGISTRY = 'https://registry.npmjs.org/'
// npm can accept a publication before the version becomes readable. Keep the
// three packages' combined wait below the workflow's 15-minute publish budget.
const VISIBILITY_TIMEOUT_MS = 180_000
const VISIBILITY_POLL_MS = 10_000

async function published(name, version, timeoutMs = 30_000) {
  const response = await fetch(`${REGISTRY}${encodeURIComponent(name)}/${version}`, {
    signal: AbortSignal.timeout(timeoutMs),
  })
  if (response.status === 404) return null
  if (!response.ok) throw new Error(`registry lookup failed for ${name}: HTTP ${response.status}`)
  return response.json()
}

function inventory(directory, name, version) {
  const archive = join(directory, `${packageStem(name)}-${version}.tgz`)
  const result = spawnSync('tar', ['-xOf', archive, 'package/package.json'], {
    encoding: 'utf8', shell: false, maxBuffer: 1_048_576,
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`cannot read ${archive}: ${result.stderr}`)
  const manifest = JSON.parse(result.stdout)
  if (manifest.name !== name || manifest.version !== version || manifest.private) {
    throw new Error(`unexpected package identity in ${archive}`)
  }
  if (manifest.repository?.url !== MANIFEST.repository.url) {
    throw new Error(`repository mismatch in ${archive}`)
  }
  return {
    name, version, archive, manifest,
    integrity: `sha512-${createHash('sha512').update(readFileSync(archive)).digest('base64')}`,
  }
}

function verifyExisting(item, remote) {
  if (remote !== null && remote.dist?.integrity !== item.integrity) {
    throw new Error(`${item.name}@${item.version} already exists with different contents; refusing to continue`)
  }
}

async function waitForPublication(item) {
  const startedAt = Date.now()
  const deadline = startedAt + VISIBILITY_TIMEOUT_MS
  for (;;) {
    const remaining = deadline - Date.now()
    if (remaining <= 0) {
      throw new Error(`${item.name}@${item.version}: npm accepted the publish, but the version is still not visible after 180s; wait and rerun the failed publish job using the same artifacts`)
    }
    const remote = await published(item.name, item.version, Math.min(30_000, remaining))
    if (remote !== null) {
      verifyExisting(item, remote)
      process.stdout.write(`publish: ${item.name}@${item.version} is visible and its integrity matches\n`)
      return
    }
    const waitMs = Math.min(VISIBILITY_POLL_MS, Math.max(0, deadline - Date.now()))
    if (waitMs > 0) {
      process.stdout.write(`publish: ${item.name}@${item.version} accepted; waiting for registry visibility (${Math.floor((Date.now() - startedAt) / 1000)}s elapsed)\n`)
      // Poll registry reads only. Never repeat a successful npm publish.
      await delay(waitMs)
    }
  }
}

async function main() {
  if (process.env.GITHUB_ACTIONS !== 'true' || process.env.GITHUB_EVENT_NAME !== 'release') {
    throw new Error('publishing is allowed only in the GitHub Release workflow')
  }
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'))
  const version = assertVersionsAgree(ROOT)
  if (event.action !== 'published' || event.release.draft || event.release.prerelease
      || event.release.tag_name !== `v${version}`) {
    throw new Error(`publishing requires a published stable Release tagged v${version}`)
  }
  const artifacts = resolve(process.argv[2] ?? '.release-artifacts')
  const platforms = MANIFEST.quorfloat.platformPackages
  const items = [
    inventory(join(artifacts, 'darwin'), platforms.darwin, version),
    inventory(join(artifacts, 'win32'), platforms['win32-x64'], version),
    inventory(join(artifacts, 'main'), MANIFEST.name, version),
  ]
  const mainPackage = items[2].manifest
  const expectedDependencies = Object.values(platforms)
  if (Object.keys(mainPackage.optionalDependencies ?? {}).length !== expectedDependencies.length
      || expectedDependencies.some(name => mainPackage.optionalDependencies[name] !== version)) {
    throw new Error('main package must depend on both native packages at this exact version')
  }
  // Check every package before the first write, including retries after a partial publish.
  for (const item of items) verifyExisting(item, await published(item.name, version))
  for (const item of items) {
    const remote = await published(item.name, version)
    verifyExisting(item, remote)
    if (remote !== null) {
      process.stdout.write(`publish: ${item.name}@${version} already has this exact tarball; skipping\n`)
      continue
    }
    runNpm(['publish', item.archive, '--access', 'public', '--tag', 'latest', `--registry=${REGISTRY}`])
    await waitForPublication(item)
  }
}

main().catch(error => {
  process.stderr.write(`publish: ${error.message}\n`)
  process.exitCode = 1
})
