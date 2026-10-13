/** Publish the workflow's tarballs in dependency order, using npm OIDC. */
import { spawnSync } from 'node:child_process'
import { appendFileSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { MANIFEST, ROOT, packageStem, runNpm } from './release-shared.mjs'
import { assertVersionsAgree } from './version.mjs'

const REGISTRY = 'https://registry.npmjs.org/'

async function registryJson(path) {
  const response = await fetch(`${REGISTRY}${path}`, {
    signal: AbortSignal.timeout(30_000),
    headers: { 'Cache-Control': 'no-cache' },
  })
  if (response.status === 404) return null
  if (!response.ok) throw new Error(`registry lookup failed for ${path}: HTTP ${response.status}`)
  return response.json()
}

async function existingVersion(name, version) {
  const escaped = encodeURIComponent(name)
  const remote = await registryJson(`${escaped}/${version}`)
  if (remote !== null) {
    if (remote.name !== name || remote.version !== version) {
      throw new Error(`registry returned an unexpected identity for ${name}@${version}`)
    }
    return 'version already published; skipped'
  }
  // npm exposes dist-tags while an accepted version is still being scanned.
  // A version lookup alone can return 404 and wrongly trigger a second upload.
  const tags = await registryJson(`-/package/${escaped}/dist-tags`)
  if (tags !== null && Object.values(tags).includes(version)) {
    return 'version already submitted (registry dist-tag exists); skipped; scanning may still be pending'
  }
  return null
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
  return { name, version, archive, manifest }
}

function report(item, status) {
  process.stdout.write(`publish: ${item.name}@${item.version}: ${status}\n`)
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `- ${item.name}@${item.version}: ${status}\n`)
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
  // A release may be rebuilt after a partial submission. Existing versions are
  // immutable and skipped by identity; archive byte equality is not a retry gate.
  // Resolve all three states before making the first registry write.
  const existing = await Promise.all(items.map(item => existingVersion(item.name, version)))
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY,
      '## npm publication submissions\n\n'
      + 'New submissions are accepted when npm publish succeeds. npm scans them before installation becomes available; this job does not wait for that scan.\n\n')
  }
  for (const [index, item] of items.entries()) {
    if (existing[index] !== null) {
      report(item, existing[index])
      continue
    }
    runNpm(['publish', item.archive, '--access', 'public', '--tag', 'latest', `--registry=${REGISTRY}`])
    // npm may scan accepted packages for minutes before registry reads succeed.
    // A successful publish is the submission boundary: do not block the next
    // package, repeat the upload, or fail the job while that scan is pending.
    report(item, 'accepted by npm; installation availability may still be pending')
  }
  process.stdout.write('publish: all three package versions are accepted or already exist; npm scanning may still be in progress\n')
}

main().catch(error => {
  process.stderr.write(`publish: ${error.message}\n`)
  process.exitCode = 1
})
