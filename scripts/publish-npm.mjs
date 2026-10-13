/** Publish the workflow's tarballs in dependency order, using npm OIDC. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { MANIFEST, ROOT, packageStem, runNpm } from './release-shared.mjs'
import { assertVersionsAgree } from './version.mjs'

const REGISTRY = 'https://registry.npmjs.org/'

async function published(name, version) {
  const response = await fetch(`${REGISTRY}${encodeURIComponent(name)}/${version}`, {
    signal: AbortSignal.timeout(30_000),
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
    const result = await published(item.name, version)
    if (result === null) throw new Error(`${item.name}@${version} is not yet visible; rerun the failed publish job`)
    verifyExisting(item, result)
  }
}

main().catch(error => {
  process.stderr.write(`publish: ${error.message}\n`)
  process.exitCode = 1
})
