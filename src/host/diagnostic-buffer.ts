/**
 * Bounded in-memory history for the diagnostics export.
 *
 * The plugin is meant to stay light: a log buffer that grows with uptime would
 * trade a support feature for a memory leak. Every entry is therefore copied into
 * a compact record and kept inside a fixed count *and* byte budget; the oldest
 * entry is dropped first, and the export reports how many were dropped.
 *
 * This is host memory, not DSH's own log store. It is also deliberately separate
 * from the sidecar marker, which exists only in dev/CI and is not exported.
 */

import type { SupervisorEvent, SupervisorState } from './supervisor.js'

/** Maximum number of supervisor events kept. */
export const EVENT_LIMIT = 100

/** Maximum text bytes kept for supervisor events. */
export const EVENT_BYTE_LIMIT = 64 * 1024

/** Maximum bytes kept for one event's detail string. */
export const EVENT_DETAIL_BYTE_LIMIT = 512

/** Maximum number of stderr lines kept. */
export const STDERR_LIMIT = 200

/** Maximum text bytes kept for stderr lines. */
export const STDERR_BYTE_LIMIT = 32 * 1024

/** Maximum bytes kept for one stderr line. */
export const STDERR_LINE_BYTE_LIMIT = 2 * 1024

/** Conservative per-entry overhead added to the text-byte accounting. */
const ENTRY_OVERHEAD_BYTES = 64

/** Hard limit for the complete exported `.log` file. */
export const EXPORT_BYTE_LIMIT = 256 * 1024

/** One compact supervisor event. */
export interface DiagnosticSupervisorEntry {
  readonly at: number
  readonly kind: Exclude<SupervisorEvent['kind'], 'stderr'>
  readonly state: SupervisorState
  readonly pid: number | null
  readonly startedAt: number | null
  readonly restarts: number
  readonly detail: string | null
}

/** One compact sidecar stderr line. */
export interface DiagnosticStderrEntry {
  readonly at: number
  readonly pid: number | null
  readonly startedAt: number | null
  readonly state: SupervisorState
  readonly line: string
}

/** A copy of the current log buffers, safe to format and then discard. */
export interface DiagnosticLogs {
  readonly supervisor: readonly DiagnosticSupervisorEntry[]
  readonly stderr: readonly DiagnosticStderrEntry[]
  readonly supervisorDropped: number
  readonly stderrDropped: number
  readonly supervisorTruncated: boolean
  readonly stderrTruncated: boolean
}

/** Empty logs, used when the formatter receives an older-shaped input. */
export const EMPTY_LOGS: DiagnosticLogs = {
  supervisor: [],
  stderr: [],
  supervisorDropped: 0,
  stderrDropped: 0,
  supervisorTruncated: false,
  stderrTruncated: false,
}

/** A stored entry and the bytes it is charged against its ring. */
interface Stored<T> {
  readonly entry: T
  readonly bytes: number
}

/**
 * Truncate a string to a UTF-8 byte budget without splitting a code point.
 *
 * The ellipsis is charged inside the budget.
 *
 * @param value - text to truncate.
 * @param maxBytes - maximum bytes including the ellipsis.
 * @returns the original text when it fits, otherwise a safely cut prefix plus `…`.
 */
export function truncateUtf8(value: string, maxBytes: number): string {
  if (maxBytes <= 0) return ''
  if (Buffer.byteLength(value, 'utf8') <= maxBytes) return value
  const suffix = '…'
  const suffixBytes = Buffer.byteLength(suffix, 'utf8')
  if (maxBytes <= suffixBytes) return ''
  let prefix = Buffer.from(value, 'utf8')
    .subarray(0, maxBytes - suffixBytes)
    .toString('utf8')
  // A cut inside a multi-byte sequence leaves a replacement character at the end;
  // strip it so the ellipsis is the only marker of the cut.
  prefix = prefix.replace(/\uFFFD+$/u, '')
  return `${prefix}${suffix}`
}

/** Turn an event detail into one line, bounded before it is stored. */
function detailText(detail: unknown): string {
  if (typeof detail === 'string') return detail
  try {
    return JSON.stringify(detail) ?? String(detail)
  } catch {
    return String(detail)
  }
}

/** The detail worth keeping for one supervisor event. */
function eventDetail(event: SupervisorEvent): string | null {
  switch (event.kind) {
    case 'exit':
    case 'frame-rejected':
      return detailText(event.detail)
    case 'restart-given-up':
      return event.snapshot.lastError?.message ?? 'restart budget exhausted'
    default:
      return null
  }
}

/** The projection of one `SupervisorEvent` that the export needs. */
function supervisorEntry(event: SupervisorEvent): {
  readonly entry: DiagnosticSupervisorEntry
  readonly truncated: boolean
} {
  const raw = eventDetail(event)
  const detail = raw === null ? null : truncateUtf8(raw, EVENT_DETAIL_BYTE_LIMIT)
  return {
    entry: {
      at: Date.now(),
      // `stderr` is the only kind this helper does not receive.
      kind: event.kind as Exclude<SupervisorEvent['kind'], 'stderr'>,
      state: event.snapshot.state,
      pid: event.snapshot.pid ?? null,
      startedAt: event.snapshot.startedAt ?? null,
      restarts: event.snapshot.restarts,
      detail,
    },
    truncated: raw !== null && detail !== raw,
  }
}

/** The projection of one stderr event. */
function stderrEntry(event: SupervisorEvent): {
  readonly entry: DiagnosticStderrEntry
  readonly truncated: boolean
} {
  const raw = typeof event.detail === 'string' ? event.detail : String(event.detail ?? '')
  const line = truncateUtf8(raw, STDERR_LINE_BYTE_LIMIT)
  return {
    entry: {
      at: Date.now(),
      pid: event.snapshot.pid ?? null,
      startedAt: event.snapshot.startedAt ?? null,
      state: event.snapshot.state,
      line,
    },
    truncated: line !== raw,
  }
}

/**
 * Fixed-size diagnostic history.
 *
 * The byte counter includes a fixed overhead per entry on top of its text, so
 * the cap remains meaningful after the strings themselves are accounted for.
 */
export class DiagnosticBuffers {
  #supervisor: Stored<DiagnosticSupervisorEntry>[] = []
  #stderr: Stored<DiagnosticStderrEntry>[] = []
  #supervisorBytes = 0
  #stderrBytes = 0
  #supervisorDropped = 0
  #stderrDropped = 0
  #supervisorTruncated = false
  #stderrTruncated = false

  /**
   * Record one supervisor event.
   *
   * The caller's snapshot is projected immediately; no reference to it survives
   * this call.
   *
   * @param event - the event published by the supervisor.
   */
  record(event: SupervisorEvent): void {
    if (event.kind === 'stderr') {
      const { entry, truncated } = stderrEntry(event)
      if (truncated) this.#stderrTruncated = true
      this.#push(
        this.#stderr,
        { entry, bytes: Buffer.byteLength(entry.line, 'utf8') + ENTRY_OVERHEAD_BYTES },
        STDERR_LIMIT,
        STDERR_BYTE_LIMIT,
        () => this.#stderrBytes,
        bytes => { this.#stderrBytes += bytes },
        bytes => { this.#stderrBytes -= bytes },
        () => { this.#stderrDropped += 1 },
      )
      return
    }
    const { entry, truncated } = supervisorEntry(event)
    if (truncated) this.#supervisorTruncated = true
    const detailBytes = entry.detail === null ? 0 : Buffer.byteLength(entry.detail, 'utf8')
    this.#push(
      this.#supervisor,
      { entry, bytes: detailBytes + ENTRY_OVERHEAD_BYTES },
      EVENT_LIMIT,
      EVENT_BYTE_LIMIT,
      () => this.#supervisorBytes,
      bytes => { this.#supervisorBytes += bytes },
      bytes => { this.#supervisorBytes -= bytes },
      () => { this.#supervisorDropped += 1 },
    )
  }

  /** Copy the current history for formatting. */
  snapshot(): DiagnosticLogs {
    return {
      supervisor: this.#supervisor.map(stored => stored.entry),
      stderr: this.#stderr.map(stored => stored.entry),
      supervisorDropped: this.#supervisorDropped,
      stderrDropped: this.#stderrDropped,
      supervisorTruncated: this.#supervisorTruncated,
      stderrTruncated: this.#stderrTruncated,
    }
  }

  /**
   * Append one entry, then drop the oldest until both caps hold.
   *
   * A single entry larger than the whole budget is dropped too, which is the one
   * case where the ring can end up empty rather than over budget.
   */
  #push<T>(
    ring: Stored<T>[],
    stored: Stored<T>,
    countLimit: number,
    byteLimit: number,
    bytesOf: () => number,
    addBytes: (bytes: number) => void,
    subtractBytes: (bytes: number) => void,
    drop: () => void,
  ): void {
    ring.push(stored)
    addBytes(stored.bytes)
    while (ring.length > countLimit || bytesOf() > byteLimit) {
      const removed = ring.shift()
      if (removed === undefined) break
      subtractBytes(removed.bytes)
      drop()
    }
  }
}
