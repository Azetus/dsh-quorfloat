/**
 * Plugin configuration: shape, normalization, and validation.
 *
 * The configuration reaches the plugin through the profile's patch layer
 * (`cordis.patch.yml`, row `id: quorfloat`), never through a private file, so
 * it can be read and overridden by the user's own patch layers.
 *
 * Validation is exposed as a Standard Schema v1 object, which is the contract
 * Cordis 4 uses for plugin `Config`: the runtime calls
 * `Config['~standard'].validate(raw)` and restarts the plugin when the profile
 * changes. The schema is implemented here by hand so the plugin keeps **zero
 * runtime dependencies**; it normalizes the same way every time, and it always
 * returns a fully populated config (defaults applied) or a list of issues.
 *
 * A patch replaces the target row's whole `config` instead of merging into it,
 * so unknown keys are rejected loudly: a typo in a user patch must not silently
 * fall back to a default.
 */

import type { StandardSchemaIssue, StandardSchemaV1 } from './cordis-types.js'

/** Where the panel anchors on the display. */
export type WindowAnchor = 'top-center' | 'top-left' | 'top-right' | 'center'

/** Theme preference forwarded to the quorfloat subproject. */
export type ThemePreference = 'system' | 'light' | 'dark'

/** Log verbosity; `debug` also traces protocol frames that carry no user content. */
export type LogLevel = 'error' | 'warn' | 'info' | 'debug'

/** Window presentation preferences (pure data; the subproject owns real windows). */
export interface WindowConfig {
  readonly width: number
  readonly maxHeight: number
  readonly anchor: WindowAnchor
  readonly alwaysOnTop: boolean
  readonly reduceMotion: boolean
  readonly theme: ThemePreference
}

/** Fully normalized plugin configuration as seen by the rest of the plugin. */
export interface QuorfloatConfig {
  /** Master switch; `false` keeps the plugin loaded but starts no process. */
  readonly enabled: boolean
  /**
   * Global hotkey the subproject should register, in `global-hotkey` syntax
   * (e.g. `Alt+Space`). Empty string means "do not register a hotkey" — the
   * documented escape hatch for a development profile that must not fight the
   * desktop profile for the same accelerator.
   */
  readonly hotkey: string
  /**
   * Absolute path to the quorfloat executable. Empty string selects the
   * platform package convention resolved by `src/host/binary.ts`.
   */
  readonly quorfloatPath: string
  /**
   * Workspace used when the user asks for a new session without picking one.
   * Empty string means "always require an explicit choice".
   */
  readonly defaultWorkspaceId: string
  readonly window: WindowConfig
  /** Budget for create-process → protocol handshake, in milliseconds. */
  readonly startupTimeoutMs: number
  /** Budget for one request/response round trip, in milliseconds. */
  readonly requestTimeoutMs: number
  /** Host heartbeat period, in milliseconds; `0` disables heartbeats. */
  readonly heartbeatMs: number
  /** Consecutive missed heartbeats (no frame at all) tolerated before degraded. */
  readonly heartbeatMissLimit: number
  /**
   * How long a claimed interaction may wait for the panel before giving up.
   *
   * Claiming means no other answerer ever sees the request, so this is what
   * keeps "the panel owns the decision" from becoming "the turn hangs forever".
   * It is a human-patience budget, not a transport timeout: granting elevated
   * permissions deserves deliberation, but an abandoned panel must not hold a
   * turn open indefinitely.
   */
  readonly claimDeadlineMs: number
  /**
   * Bytes buffered for the peer before writes are considered stalled.
   *
   * The peer consumes a stream of session events; when it falls far enough
   * behind, continuing to buffer would trade a visible stall for unbounded
   * memory. Exceeding this aborts the subscription and asks for a resync.
   */
  readonly maxWriteBufferBytes: number
  /**
   * Peer stderr lines forwarded to the host log per channel generation.
   *
   * The peer's narration is the only window into what it is doing, so it is
   * forwarded; the cap keeps a chatty or looping peer from flooding the log.
   */
  readonly maxStderrLines: number
  /**
   * How long a client presence report stays valid.
   *
   * A safety net, not the primary signal: the browser half reports on every
   * `visibilitychange` / `focus` / `blur`, which are event-driven and still
   * delivered to a backgrounded page. The age only matters when a page dies
   * without reporting — and a stale report is treated as "not looking", so a
   * shorter value fails towards the panel rather than towards a window nobody
   * is watching.
   */
  readonly presenceMaxAgeMs: number
  /** Grace period between the shutdown request and escalating signals. */
  readonly shutdownGraceMs: number
  /** Automatic restarts allowed inside {@link restartWindowMs}. */
  readonly restartLimit: number
  /** Sliding window for {@link restartLimit}, in milliseconds. */
  readonly restartWindowMs: number
  readonly logLevel: LogLevel
}

/** Field specification used by the hand-written schema below. */
type FieldSpec =
  | { kind: 'boolean'; def: boolean }
  | { kind: 'string'; def: string; pattern?: RegExp; patternHint?: string }
  | { kind: 'int'; def: number; min: number; max: number }
  | { kind: 'enum'; def: string; values: readonly string[] }
  | { kind: 'object'; def: Record<string, FieldSpec> }

/** The complete schema: field name → specification, plus the nested window object. */
const SPEC = {
  enabled: { kind: 'boolean', def: true },
  hotkey: { kind: 'string', def: 'Alt+Space' },
  quorfloatPath: { kind: 'string', def: '' },
  defaultWorkspaceId: { kind: 'string', def: '' },
  window: {
    kind: 'object',
    def: {
      width: { kind: 'int', def: 640, min: 360, max: 1600 },
      maxHeight: { kind: 'int', def: 560, min: 240, max: 2000 },
      anchor: { kind: 'enum', def: 'top-center', values: ['top-center', 'top-left', 'top-right', 'center'] },
      alwaysOnTop: { kind: 'boolean', def: true },
      reduceMotion: { kind: 'boolean', def: false },
      theme: { kind: 'enum', def: 'system', values: ['system', 'light', 'dark'] },
    },
  },
  startupTimeoutMs: { kind: 'int', def: 8000, min: 500, max: 60000 },
  requestTimeoutMs: { kind: 'int', def: 10000, min: 500, max: 120000 },
  heartbeatMs: { kind: 'int', def: 5000, min: 0, max: 60000 },
  heartbeatMissLimit: { kind: 'int', def: 3, min: 1, max: 30 },
  claimDeadlineMs: { kind: 'int', def: 600000, min: 1000, max: 3600000 },
  maxWriteBufferBytes: { kind: 'int', def: 4194304, min: 65536, max: 268435456 },
  maxStderrLines: { kind: 'int', def: 200, min: 0, max: 100000 },
  presenceMaxAgeMs: { kind: 'int', def: 30000, min: 1000, max: 600000 },
  shutdownGraceMs: { kind: 'int', def: 1500, min: 100, max: 10000 },
  restartLimit: { kind: 'int', def: 3, min: 0, max: 20 },
  restartWindowMs: { kind: 'int', def: 30000, min: 1000, max: 600000 },
  logLevel: { kind: 'enum', def: 'info', values: ['error', 'warn', 'info', 'debug'] },
} as const satisfies Record<string, FieldSpec>

/** Top-level field names, used both for validation and for unknown-key reporting. */
const SPEC_KEYS: readonly string[] = Object.keys(SPEC)

/**
 * Normalize and validate a raw value produced by the patch layer.
 *
 * @param raw - config value as written in a patch layer, or `undefined`.
 * @returns the normalized config, or the collected issues.
 */
function normalize(raw: unknown): { value: QuorfloatConfig } | { issues: StandardSchemaIssue[] } {
  const issues: StandardSchemaIssue[] = []
  const input = raw === undefined || raw === null ? {} : raw
  if (typeof input !== 'object' || Array.isArray(input)) {
    return { issues: [{ message: 'config must be a mapping of field names to values' }] }
  }
  const source = input as Record<string, unknown>

  for (const key of Object.keys(source)) {
    if (!SPEC_KEYS.includes(key)) {
      issues.push({
        message: `unknown configuration field (supported: ${SPEC_KEYS.join(', ')})`,
        path: [key],
      })
    }
  }

  const out: Record<string, unknown> = {}
  for (const key of SPEC_KEYS) {
    const spec = (SPEC as Record<string, FieldSpec>)[key]!
    const field = readField(spec, source, key, issues, [key])
    out[key] = field
  }
  if (issues.length > 0) return { issues }
  return { value: out as unknown as QuorfloatConfig }
}

/**
 * Validate one field, applying its default when absent.
 *
 * @param spec - the field specification.
 * @param holder - object that should carry the field.
 * @param key - field name inside `holder`.
 * @param issues - collector for human-readable problems.
 * @param path - path prefix used in issue reporting.
 * @returns the normalized field value (a default is returned even on error).
 */
function readField(
  spec: FieldSpec,
  holder: Record<string, unknown>,
  key: string,
  issues: StandardSchemaIssue[],
  path: readonly PropertyKey[],
): unknown {
  const present = Object.hasOwn(holder, key)
  const value = present ? holder[key] : undefined
  switch (spec.kind) {
    case 'boolean': {
      if (!present) return spec.def
      if (typeof value !== 'boolean') {
        issues.push({ message: 'must be a boolean', path: [...path] })
        return spec.def
      }
      return value
    }
    case 'string': {
      if (!present) return spec.def
      if (typeof value !== 'string') {
        issues.push({ message: 'must be a string', path: [...path] })
        return spec.def
      }
      if (spec.pattern !== undefined && value !== '' && !spec.pattern.test(value)) {
        issues.push({ message: spec.patternHint ?? 'has an invalid format', path: [...path] })
        return spec.def
      }
      return value
    }
    case 'int': {
      if (!present) return spec.def
      if (typeof value !== 'number' || !Number.isInteger(value)) {
        issues.push({ message: 'must be an integer', path: [...path] })
        return spec.def
      }
      if (value < spec.min || value > spec.max) {
        issues.push({ message: `must be between ${spec.min} and ${spec.max}`, path: [...path] })
        return spec.def
      }
      return value
    }
    case 'enum': {
      if (!present) return spec.def
      if (typeof value !== 'string' || !spec.values.includes(value)) {
        issues.push({ message: `must be one of ${spec.values.join(', ')}`, path: [...path] })
        return spec.def
      }
      return value
    }
    case 'object': {
      if (!present) {
        return readObject(spec.def, {}, issues, path)
      }
      if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        issues.push({ message: 'must be a mapping', path: [...path] })
        return readObject(spec.def, {}, issues, path)
      }
      return readObject(spec.def, value as Record<string, unknown>, issues, path)
    }
  }
}

/**
 * Validate one nested object against its own field specifications.
 *
 * @param fields - nested field specifications.
 * @param source - raw nested mapping.
 * @param issues - collector for human-readable problems.
 * @param path - path prefix used in issue reporting.
 * @returns the normalized nested object.
 */
function readObject(
  fields: Record<string, FieldSpec>,
  source: Record<string, unknown>,
  issues: StandardSchemaIssue[],
  path: readonly PropertyKey[],
): Record<string, unknown> {
  for (const key of Object.keys(source)) {
    if (!Object.hasOwn(fields, key)) {
      issues.push({
        message: `unknown configuration field (supported: ${Object.keys(fields).join(', ')})`,
        path: [...path, key],
      })
    }
  }
  const out: Record<string, unknown> = {}
  for (const [key, spec] of Object.entries(fields)) {
    out[key] = readField(spec, source, key, issues, [...path, key])
  }
  return out
}

/** The config schema handed to Cordis as the plugin's `Config`. */
export const Config: StandardSchemaV1<unknown, QuorfloatConfig> = {
  '~standard': {
    version: 1,
    vendor: 'dsh-quorfloat',
    validate: (value: unknown) => normalize(value),
  },
}

/** The all-defaults configuration, useful for tests and for diagnostics. */
export const DEFAULT_CONFIG: QuorfloatConfig = (() => {
  const result = normalize(undefined)
  if ('issues' in result) {
    throw new Error('dsh-quorfloat: default configuration failed validation')
  }
  return result.value
})()
