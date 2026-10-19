/**
 * Text diagnostics for the settings page's "Export diagnostics log" action.
 *
 * The file contains a current-state snapshot plus the bounded recent history the
 * host already observed: supervisor lifecycle events and sidecar stderr. It does
 * not read DSH's own logs, and it does not read the sidecar marker (which exists
 * only in dev/CI). IDs are shortened in the snapshot and the user's home prefix
 * is replaced with `~` in both the snapshot and the log lines.
 */

import { mkdirSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

import type { QuorfloatConfig } from '../config.js'
import type { AuthorityVerdict } from '../harness/presence.js'
import type { PluginState } from '../index.js'
import type { SupervisorSnapshot } from './supervisor.js'
import {
  EMPTY_LOGS,
  EVENT_LIMIT,
  EXPORT_BYTE_LIMIT,
  STDERR_LIMIT,
  truncateUtf8,
  type DiagnosticLogs,
} from './diagnostic-buffer.js'

/** Everything the formatter reads; `state` is the activation's own projection. */
export interface DiagnosticsInput {
  readonly generatedAt: string
  readonly protocolVersion: string
  readonly dshVersion: string
  readonly platform: string
  readonly arch: string
  readonly config: QuorfloatConfig
  readonly state: PluginState
  readonly authority: AuthorityVerdict
  readonly home: string
  /** Bounded recent history; absent means empty for older callers/tests. */
  readonly logs?: DiagnosticLogs
}

/** One saved diagnostics file. */
export interface SavedDiagnostics {
  /** Absolute path, as shown to the user. */
  readonly path: string
  readonly filename: string
}

/** Fields whose values are identifiers rather than free text. */
const ID_KEYS = new Set([
  'sessionId',
  'activeSessionId',
  'interactionId',
  'requestId',
  'workspaceId',
  'defaultWorkspaceId',
])

/** Fields whose values are local paths. */
const PATH_KEYS = new Set(['binaryPath', 'quorfloatPath', 'path', 'cwd'])

/**
 * Shorten an opaque identifier while keeping both ends recognisable.
 *
 * @param value - the identifier.
 * @returns the identifier as-is when short, otherwise `first8…last4`.
 */
export function shortId(value: string): string {
  return value.length <= 12 ? value : `${value.slice(0, 8)}…${value.slice(-4)}`
}

/**
 * Replace the user's home prefix with `~`, including inside error messages.
 *
 * Both slash directions are tried so a macOS/Linux path and a Windows path are
 * redacted whichever form a message happens to use.
 *
 * @param value - text that may contain the home directory.
 * @param home - the home directory to replace.
 * @returns the redacted text.
 */
export function redactHome(value: string, home: string): string {
  if (home === '') return value
  let redacted = value
  for (const variant of [home, home.replaceAll('\\', '/'), home.replaceAll('/', '\\')]) {
    if (variant === '') continue
    redacted = redacted.split(variant).join('~')
  }
  return redacted
}

/**
 * Recursively redact paths and shorten known identifier fields.
 *
 * The snapshot is data from our own state objects, so it is a JSON-like tree;
 * no cycles are possible.
 */
function sanitize(value: unknown, home: string): unknown {
  if (typeof value === 'string') return redactHome(value, home)
  if (Array.isArray(value)) return value.map(entry => sanitize(entry, home))
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, entry]) => {
      if (typeof entry === 'string' && ID_KEYS.has(key)) return [key, shortId(entry)]
      if (typeof entry === 'string' && PATH_KEYS.has(key)) return [key, redactHome(entry, home)]
      return [key, sanitize(entry, home)]
    }))
  }
  return value
}

/** Placeholder for an absent or empty value in the flattened snapshot. */
const EMPTY = '(none)'

/** Separator between the `key=value` fields of one object-array item. */
const ITEM_SEPARATOR = '\uFF5C'

/**
 * Render one sanitized snapshot section as flat, human-readable text.
 *
 * Plain objects become one `key: value` line per field, with nested fields using
 * dotted paths. Arrays of objects become one `- key=value...` line per item,
 * separated by a full-width vertical bar. Scalar arrays stay on one comma-separated
 * line. There is deliberately no width-based fallback: the format is stable, and
 * support reads this file as a whole rather than parsing fixed columns.
 *
 * @param value - section value from the plugin state.
 * @param home - home directory to redact.
 * @returns the section body without a trailing newline.
 */
function section(value: unknown, home: string): string {
  const sanitized = sanitize(value, home)
  const record = asRecord(sanitized)
  if (record === null) return formatScalar(sanitized)
  return renderObject(record).join('\n')
}

/** Render object fields, recursing into nested objects with dotted paths. */
function renderObject(record: Record<string, unknown>, prefix = ''): string[] {
  const lines: string[] = []
  for (const [name, value] of Object.entries(record)) {
    if (value === undefined) continue
    const key = prefix === '' ? name : `${prefix}.${name}`
    if (value === null) {
      lines.push(`${key}: ${EMPTY}`)
    } else if (Array.isArray(value)) {
      lines.push(...renderArray(key, value))
    } else if (asRecord(value) !== null) {
      const nestedRecord = asRecord(value) as Record<string, unknown>
      const nested = Object.entries(nestedRecord)
      if (nested.length === 0) lines.push(`${key}: ${EMPTY}`)
      else lines.push(...renderObject(nestedRecord, key))
    } else {
      lines.push(`${key}: ${formatScalar(value)}`)
    }
  }
  return lines
}

/** Render one array on one or more lines, depending on whether it holds objects. */
function renderArray(key: string, values: readonly unknown[]): string[] {
  if (values.length === 0) return [`${key}: ${EMPTY}`]
  if (values.every(isScalar)) {
    return [`${key}: ${values.map(formatScalar).join(', ')}`]
  }
  const lines = [`${key} (${values.length}):`]
  for (const value of values) {
    const item = asRecord(value)
    if (item === null) {
      lines.push(`  - ${formatScalar(value)}`)
      continue
    }
    const pairs = renderPairs(item)
    lines.push(`  - ${pairs === '' ? EMPTY : pairs}`)
  }
  return lines
}

/** Render one object-array item as `key=value` fields joined by the item separator. */
function renderPairs(record: Record<string, unknown>, prefix = ''): string {
  const pairs: string[] = []
  for (const [name, value] of Object.entries(record)) {
    if (value === undefined) continue
    const key = prefix === '' ? name : `${prefix}.${name}`
    if (value === null) {
      pairs.push(`${key}=${EMPTY}`)
    } else if (Array.isArray(value)) {
      pairs.push(`${key}=${value.length === 0 ? EMPTY : value.map(formatScalar).join(',')}`)
    } else if (asRecord(value) !== null) {
      const nestedRecord = asRecord(value) as Record<string, unknown>
      const nested = Object.entries(nestedRecord)
      if (nested.length === 0) pairs.push(`${key}=${EMPTY}`)
      else pairs.push(renderPairs(nestedRecord, key))
    } else {
      pairs.push(`${key}=${formatScalar(value)}`)
    }
  }
  return pairs.join(ITEM_SEPARATOR)
}

/** Whether an array entry can be printed as a plain scalar. */
function isScalar(value: unknown): boolean {
  return !Array.isArray(value) && asRecord(value) === null
}

/** Print one scalar without JSON quoting. */
function formatScalar(value: unknown): string {
  if (value === null || value === undefined) return EMPTY
  if (typeof value === 'string') return value === '' ? EMPTY : value
  return String(value)
}

/** Read a record-shaped value without trusting the wire. */
function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

/**
 * Project the session summary onto the fields support needs.
 *
 * This is a whitelist on purpose: `describe()` is free to grow, and a field
 * added there must not silently enter a user-exported file. Only identifiers,
 * cursors and counts are kept; no conversation text is read at all.
 */
function sessionProjection(value: unknown): unknown {
  const record = asRecord(value)
  if (record === null) return null
  const follows = Array.isArray(record['follows'])
    ? record['follows'].map(entry => {
        const row = asRecord(entry)
        return row === null
          ? null
          : {
              sessionId: row['sessionId'],
              generation: row['generation'],
              cursor: row['cursor'],
              snapshotDelivered: row['snapshotDelivered'],
            }
      })
    : []
  const prompts = Array.isArray(record['prompts'])
    ? record['prompts'].map(entry => {
        const row = asRecord(entry)
        return row === null
          ? null
          : {
              requestId: row['requestId'],
              sessionId: row['sessionId'],
              state: row['state'],
            }
      })
    : []
  return { activeSessionId: record['activeSessionId'], follows, prompts }
}

/** Project the interaction summary onto identifiers, kinds and counters. */
function interactionProjection(value: unknown): unknown {
  const record = asRecord(value)
  if (record === null) return null
  const pending = Array.isArray(record['pending'])
    ? record['pending'].map(entry => {
        const row = asRecord(entry)
        return row === null
          ? null
          : {
              interactionId: row['interactionId'],
              sessionId: row['sessionId'],
              kind: row['kind'],
            }
      })
    : []
  return {
    registered: record['registered'],
    deadlineMs: record['deadlineMs'],
    pending,
    counters: record['counters'],
  }
}

/**
 * Build the complete `.log` text.
 *
 * @param input - current state, bounded recent logs and version facts.
 * @returns a UTF-8 text document ending with one newline.
 */
export function formatDiagnostics(input: DiagnosticsInput): string {
  const { state, config, authority, home } = input
  const logs = input.logs ?? EMPTY_LOGS
  const snapshot: SupervisorSnapshot | null = state.supervisor
  const lines = [
    'dsh-quorfloat diagnostics',
    `generatedAt: ${input.generatedAt}`,
    `protocolVersion: ${input.protocolVersion}`,
    `dshVersion: ${input.dshVersion}`,
    `platform: ${input.platform} ${input.arch}`,
    `stopped: ${state.stopped}`,
    '',
    '[supervisor]',
    section(snapshot, home),
    '',
    '[config]',
    section(config, home),
    '',
    '[services]',
    section(state.services === null
      ? { probed: false, missing: state.missingServices }
      : { ...state.services, missing: state.missingServices }, home),
    '',
    '[sessions]',
    section(sessionProjection(state.sessions), home),
    '',
    '[interactions]',
    section(interactionProjection(state.interactions), home),
    '',
    '[authority]',
    section(authority, home),
    '',
    '[logs]',
    `supervisorEvents: ${logs.supervisor.length} (limit ${EVENT_LIMIT}, dropped ${logs.supervisorDropped}, truncated: ${logs.supervisorTruncated})`,
    `stderrLines: ${logs.stderr.length} (limit ${STDERR_LIMIT}, dropped ${logs.stderrDropped}, truncated: ${logs.stderrTruncated})`,
    '',
    '[logs.supervisor]',
    supervisorLogText(logs.supervisor, home),
    '',
    '[logs.stderr]',
    stderrLogText(logs.stderr, home),
    '',
  ]
  const full = `${lines.join('\n')}\n`
  if (Buffer.byteLength(full, 'utf8') <= EXPORT_BYTE_LIMIT) return full
  const suffix = '\n[export truncated]\n'
  const budget = EXPORT_BYTE_LIMIT - Buffer.byteLength(suffix, 'utf8')
  return `${truncateUtf8(full, budget)}${suffix}`
}

/** Keep one log line on one line while applying the same home redaction as the snapshot. */
function logLine(value: string, home: string): string {
  return redactHome(value, home).replaceAll('\r', '\\r').replaceAll('\n', '\\n')
}

/** One ISO timestamp for a stored entry. */
function logTime(at: number): string {
  return new Date(at).toISOString()
}

/** Render the supervisor event ring. */
function supervisorLogText(entries: DiagnosticLogs['supervisor'], home: string): string {
  if (entries.length === 0) return '(none)'
  return entries.map(entry => {
    const pid = entry.pid === null ? '-' : String(entry.pid)
    const startedAt = entry.startedAt === null ? '-' : logTime(entry.startedAt)
    const detail = entry.detail === null ? '' : ` | ${logLine(entry.detail, home)}`
    return `${logTime(entry.at)} ${entry.kind} pid=${pid} startedAt=${startedAt} state=${entry.state} restarts=${entry.restarts}${detail}`
  }).join('\n')
}

/** Render the sidecar stderr ring. */
function stderrLogText(entries: DiagnosticLogs['stderr'], home: string): string {
  if (entries.length === 0) return '(none)'
  return entries.map(entry => {
    const pid = entry.pid === null ? '-' : String(entry.pid)
    const startedAt = entry.startedAt === null ? '-' : logTime(entry.startedAt)
    return `${logTime(entry.at)} pid=${pid} startedAt=${startedAt} state=${entry.state} | ${logLine(entry.line, home)}`
  }).join('\n')
}

/**
 * Directory used for exported diagnostics.
 *
 * Precedence:
 * 1. `DSH_QUORFLOAT_LOG_DIR`;
 * 2. `logs/` beside an overridden preferences file (keeps `dev` isolated);
 * 3. `~/.dsh-quorfloat/logs/`.
 *
 * @param env - environment to read.
 * @param home - user's home directory.
 * @returns an absolute directory path.
 */
export function logDirectory(
  env: NodeJS.ProcessEnv = process.env,
  home: string = homedir(),
): string {
  const explicit = env['DSH_QUORFLOAT_LOG_DIR']?.trim()
  if (explicit !== undefined && explicit !== '') return resolve(explicit)
  const preferences = env['DSH_QUORFLOAT_PREFERENCES']?.trim()
  if (preferences !== undefined && preferences !== '') {
    return join(dirname(resolve(preferences)), 'logs')
  }
  return join(home, '.dsh-quorfloat', 'logs')
}

/**
 * Write one diagnostics file and return its absolute path.
 *
 * The file is created with `wx` so two exports in the same millisecond do not
 * overwrite each other; a numeric suffix is tried before giving up. The
 * directory and file are created user-only on platforms that honour modes.
 *
 * @param text - the complete log content.
 * @param options.directory - directory override, mainly for tests.
 * @param options.now - timestamp override, mainly for tests.
 * @returns the file that was written.
 */
export function saveDiagnostics(
  text: string,
  options: { readonly directory?: string; readonly now?: Date } = {},
): SavedDiagnostics {
  const directory = resolve(options.directory ?? logDirectory())
  mkdirSync(directory, { recursive: true, mode: 0o700 })
  const stamp = timestamp(options.now ?? new Date())
  const body = text.endsWith('\n') ? text : `${text}\n`
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const suffix = attempt === 0 ? '' : `-${attempt}`
    const filename = `dsh-quorfloat-diagnostics-${stamp}${suffix}.log`
    const path = join(directory, filename)
    try {
      writeFileSync(path, body, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
      return { path, filename }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'EEXIST') continue
      throw error
    }
  }
  throw new Error(`could not create a unique diagnostics file in ${directory}`)
}

/** `YYYYMMDD-HHmmss-SSS` in UTC, matching the file name format. */
function timestamp(date: Date): string {
  const iso = date.toISOString()
  return `${iso.slice(0, 4)}${iso.slice(5, 7)}${iso.slice(8, 10)}-${iso.slice(11, 13)}${iso.slice(14, 16)}${iso.slice(17, 19)}-${iso.slice(20, 23)}`
}
