/**
 * The development script's workspace registry.
 *
 * This is the one piece of `scripts/dev.mjs` with behaviour worth pinning. The rest
 * of the script either calls `node:fs` or starts a process, and both are verified by
 * running it; but the registry is *data*, and getting it wrong fails far from the
 * cause: with no registered workspace, `workspaces/list` is empty and every session
 * operation fails at the plugin end, which reads as a plugin fault rather than a
 * missing prerequisite.
 *
 * Imported rather than executed, which is why the script guards its `main()` call.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { buildWorkspaceRegistry, DEV_WORKSPACE_ID } from '../scripts/dev.mjs'

/** A fixed timestamp, so the assertions do not depend on the clock. */
const NOW = '2026-10-06T00:00:00.000Z'

test('a first run registers one workspace and makes it the default', () => {
  const registry = buildWorkspaceRegistry(undefined, '/tmp/dev/workspace', NOW)
  assert.equal(registry.unit.name, 'workspace')
  assert.equal(registry.unit.version, 2, 'the version the storage unit expects')
  assert.equal(registry.global.initialized, true)
  assert.deepEqual(registry.global.workspaceIds, [DEV_WORKSPACE_ID])
  // The default is what makes a new session land there without the user choosing.
  assert.equal(registry.global.defaultWorkspaceId, DEV_WORKSPACE_ID)
  const workspace = registry.tables.workspaces[DEV_WORKSPACE_ID]
  assert.equal(workspace.path, '/tmp/dev/workspace')
  assert.deepEqual(workspace.sessionIds, [], 'a new workspace owns no sessions yet')
  assert.equal(workspace.createdAt, NOW)
})

test('the title is the directory name, matching what the client half produces', () => {
  const registry = buildWorkspaceRegistry(undefined, '/tmp/dev/workspace', NOW)
  assert.equal(registry.tables.workspaces[DEV_WORKSPACE_ID].title, 'workspace')
})

test('a later run keeps the session list instead of orphaning it', () => {
  // The failure this prevents: rebuilding the registry each run would drop
  // `sessionIds`, leaving on-disk sessions that belong to no workspace — present but
  // unreachable, which looks like data loss rather than a reset.
  const first = buildWorkspaceRegistry(undefined, '/tmp/dev/workspace', NOW)
  first.tables.workspaces[DEV_WORKSPACE_ID].sessionIds = ['session-a', 'session-b']
  const second = buildWorkspaceRegistry(first, '/tmp/dev/workspace', '2026-10-07T00:00:00.000Z')
  assert.deepEqual(second.tables.workspaces[DEV_WORKSPACE_ID].sessionIds, ['session-a', 'session-b'])
  assert.equal(second.tables.workspaces[DEV_WORKSPACE_ID].createdAt, NOW, 'the original creation time stands')
  assert.equal(second.tables.workspaces[DEV_WORKSPACE_ID].updatedAt, '2026-10-07T00:00:00.000Z')
})

test('a later run re-points the path so the home can be moved', () => {
  // A `DSH_HOME` copied to another machine or directory keeps its sessions, and the
  // workspace record has to follow it or session lookup uses a path that is gone.
  const first = buildWorkspaceRegistry(undefined, '/old/place', NOW)
  const second = buildWorkspaceRegistry(first, '/new/place', NOW)
  assert.equal(second.tables.workspaces[DEV_WORKSPACE_ID].path, '/new/place')
})

test('workspaces the user added are left alone', () => {
  // The script owns one id; anything else in the registry came from the user
  // opening a project, and dropping it would be data loss.
  const previous = {
    unit: { name: 'workspace', version: 2 },
    global: { initialized: true, workspaceIds: ['other-id'], archivedSessionIds: [], pinnedSessionIds: [] },
    tables: { workspaces: { 'other-id': { path: '/somewhere', title: 'somewhere', sessionIds: ['s1'] } } },
  }
  const registry = buildWorkspaceRegistry(previous, '/tmp/dev/workspace', NOW)
  assert.deepEqual(registry.tables.workspaces['other-id'].sessionIds, ['s1'])
  assert.deepEqual(registry.global.workspaceIds, ['other-id', DEV_WORKSPACE_ID])
})

test('the id is not registered twice when it is already present', () => {
  const first = buildWorkspaceRegistry(undefined, '/tmp/dev/workspace', NOW)
  const second = buildWorkspaceRegistry(first, '/tmp/dev/workspace', NOW)
  assert.deepEqual(second.global.workspaceIds, [DEV_WORKSPACE_ID])
})

test('a previous registry of the wrong shape is replaced rather than trusted', () => {
  // A truncated or foreign file must not throw: the script runs unattended, and a
  // corrupt throwaway home is something it should repair, not report.
  for (const previous of [{}, { tables: {} }, { tables: { workspaces: null } }, { global: null }]) {
    const registry = buildWorkspaceRegistry(previous, '/tmp/dev/workspace', NOW)
    assert.equal(registry.global.defaultWorkspaceId, DEV_WORKSPACE_ID)
    assert.equal(registry.tables.workspaces[DEV_WORKSPACE_ID].path, '/tmp/dev/workspace')
  }
})

// A performance run must never silently fall back to the debug binary.
test('release/perf are dev flags, not Harness flags', async () => {
  const { parseArgs } = await import('../scripts/dev.mjs')
  const options = parseArgs(['--release', '--perf', '--no-build', '--port', '1234'])
  assert.equal(options.rustProfile, 'release')
  assert.equal(options.perf, true)
  assert.equal(options.build, false)
  assert.deepEqual(options.passthrough, [])
  assert.equal(parseArgs([]).rustProfile, 'debug')
})

test('sidecar selection requires the requested profile even when another build exists', async () => {
  const { sidecarSource } = await import('../scripts/dev.mjs')
  const { mkdtempSync, mkdirSync, writeFileSync, rmSync } = await import('node:fs')
  const { tmpdir } = await import('node:os')
  const { join } = await import('node:path')
  const root = mkdtempSync(join(tmpdir(), 'qf-profile-'))
  const binary = process.platform === 'win32' ? 'dsh-quorfloat.exe' : 'dsh-quorfloat'
  try {
    const debug = join(root, 'quorfloat', 'target', 'debug')
    const release = join(root, 'quorfloat', 'target', 'release')
    mkdirSync(debug, { recursive: true })
    writeFileSync(join(debug, binary), 'debug')
    assert.throws(() => sidecarSource(root, 'release'), /cargo build.*--release/)
    mkdirSync(release, { recursive: true })
    writeFileSync(join(release, binary), 'release')
    assert.equal(sidecarSource(root, 'release'), join(release, binary))
    assert.equal(sidecarSource(root, 'debug'), join(debug, binary))
  } finally { rmSync(root, { recursive: true, force: true }) }
})
