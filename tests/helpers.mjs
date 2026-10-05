/**
 * Shared helpers for the host-plugin tests.
 *
 * The tests load the **real** `@deepseek-ai/cordis` Context (the same framework
 * the desktop runtime uses) and the compiled plugin from `lib/`. Only external
 * seams are substituted: the quorfloat executable, the Harness services, and the
 * clock where a test needs to observe a timeout without waiting for one.
 */

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

/** Absolute path to this repository root. */
export const repoRoot = new URL('..', import.meta.url).pathname.replace(/\/$/, '')

/** Load the real Cordis framework. */
export async function loadCordis() {
  const cordis = process.env['DSH_CORDIS_PATH'] ??
    join(process.env['HOME'] ?? '', '.dsh/profiles/node_modules/@deepseek-ai/cordis/lib/index.js')
  const module = await import(pathToFileURL(cordis).href)
  if (typeof module.Context !== 'function') {
    throw new Error(`could not load the Cordis Context from ${cordis}`)
  }
  return module
}

/** Load the compiled plugin module. */
export async function loadPlugin() {
  return await import(pathToFileURL(join(repoRoot, 'lib/index.js')).href)
}

/** Load another compiled module from `lib/`. */
export async function loadModule(relative) {
  return await import(pathToFileURL(join(repoRoot, 'lib', relative)).href)
}

/** Path to the mock quorfloat fixture. */
export const mockPath = join(repoRoot, 'tests/fixtures/mock-quorfloat.mjs')

/** Create a scratch directory that the caller must clean up. */
export function scratchDir() {
  return mkdtempSync(join(tmpdir(), 'dsh-quorfloat-test-'))
}

/** Remove a scratch directory without failing the test on cleanup errors. */
export function cleanupDir(path) {
  try {
    rmSync(path, { recursive: true, force: true })
  } catch {
    // A leftover temp directory must never fail a test run.
  }
}

/**
 * Create a logger that records calls, so tests can assert on what was reported
 * without depending on formatting.
 */
export function recordingLogger() {
  const lines = []
  const make = level => (...args) => {
    lines.push({ level, args })
  }
  const logger = { error: make('error'), warn: make('warn'), info: make('info'), debug: make('debug'), lines }
  logger.text = () => lines.map(line => `${line.level}: ${line.args.map(String).join(' ')}`).join('\n')
  return logger
}

/** Wait for a condition, polling; fails loudly instead of hanging forever. */
export async function waitFor(description, predicate, { timeoutMs = 5000, intervalMs = 10 } = {}) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const value = await predicate()
    if (value) return value
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${description}`)
    await sleep(intervalMs)
  }
}

/** Sleep for a number of milliseconds. */
export function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

/**
 * Build a supervisor over the mock quorfloat process.
 *
 * @param options.mode - mock scenario name.
 * @param options.config - configuration overrides.
 * @param options.reportPath - where the mock should write its observations.
 */
export async function buildSupervisor({ mode = 'normal', config = {}, reportPath, extraEnv = {}, deps = {} } = {}) {
  const { QuorfloatSupervisor } = await loadModule('host/supervisor.js')
  const { HostRouter } = await loadModule('bridge/router.js')
  const { DEFAULT_CONFIG } = await loadModule('config.js')
  const logger = recordingLogger()
  const events = []
  const effective = { ...DEFAULT_CONFIG, requestTimeoutMs: 2000, heartbeatMs: 200, heartbeatMissLimit: 2, startupTimeoutMs: 3000, shutdownGraceMs: 500, restartLimit: 1, restartWindowMs: 5000, logLevel: 'debug', ...config }

  const harnessHost = {
    hostVersion: () => 'test-host',
    listWorkspaces: async () => [{ workspaceId: 'w1', path: '/tmp/ws', title: 'ws' }],
    listSessions: async () => ({ items: [] }),
    createSession: async () => ({ sessionId: 'session-1' }),
    attachSession: async sessionId => ({ sessionId }),
    readHistory: async () => ({ records: [], hasMore: false }),
    prompt: async () => ({ accepted: true }),
    cancel: async () => ({ accepted: true }),
    answerInteraction: async () => ({ accepted: true }),
    reportPresence: async () => ({ accepted: true }),
    diagnostics: () => ({ channelSessionId: 'test-channel' }),
    ...deps.routerHost,
  }

  const supervisor = new QuorfloatSupervisor({
    config: () => effective,
    // The mock is a Node script, not a native binary; launching it through the
    // interpreter exercises the same spawn path a real executable takes.
    resolveBinary: () => ({ path: process.execPath, args: [mockPath], source: 'config', attempts: [] }),
    createRouter: channelSessionId =>
      new HostRouter({ config: () => effective, channelSessionId: () => channelSessionId, ...harnessHost }),
    log: logger,
    onEvent: event => events.push(event),
    onPeerNotification: deps.onPeerNotification,
  })

  // The mock needs the scenario in its environment; the supervisor builds the
  // child environment itself, so the scenario is injected through process.env.
  const saved = {}
  const exported = {
    DSH_QUORFLOAT_MOCK_MODE: mode,
    ...(reportPath === undefined ? {} : { DSH_QUORFLOAT_MOCK_REPORT: reportPath }),
    ...extraEnv,
  }
  for (const [key, value] of Object.entries(exported)) {
    saved[key] = process.env[key]
    process.env[key] = String(value)
  }
  const restore = () => {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }

  return { supervisor, logger, events, config: effective, restore }
}

/** Read the mock's observation report, or `undefined` when it never wrote one. */
export async function readReport(path) {
  const { readFileSync } = await import('node:fs')
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch {
    return undefined
  }
}
