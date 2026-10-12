/**
 * Session subscriptions and wire projection.
 *
 * This layer answers the questions the adapter deliberately does not:
 * - which sessions does this host currently follow, and with what generation;
 * - how do upstream snapshot/event/stream frames become protocol notifications;
 * - why may a prompt never be sent twice, even after a reconnect;
 * - which frames belong to a subscription that was already replaced.
 *
 * It never talks to Harness directly: it consumes {@link QuorfloatHarness} and
 * emits notifications through an injected sender, so its rules (generation
 * guards, dedup keys, sequence checks) are unit-testable without a runtime.
 */

import type {
  FollowFrame,
  FollowHandle,
  HistoryPageView,
  ModelChoiceView,
  QuorfloatHarness,
  SessionOptionsView,
  SessionStatsView,
  SessionSummaryView,
  WorkspaceView,
} from './adapter.js'
import { HarnessError, oldestSeq } from './adapter.js'

/** Minimal logger shape used here. */
export interface SessionLayerLogger {
  warn(message: string, detail?: unknown): void
  debug(message: string, detail?: unknown): void
}

/** One `session/snapshot` frame, as the adapter projects it. */
type SnapshotFrame = Extract<FollowFrame, { type: 'snapshot' }>

/**
 * Byte budget for one `session/snapshot/part` payload.
 *
 * A quarter of the protocol's 1 MiB frame ceiling. A snapshot used to travel as one
 * frame, and a conversation whose records passed that ceiling was dropped by the peer's
 * reader before any of it could be folded — leaving the panel blank with no reason on
 * the record. The budget leaves room for the frame envelope and the peer's write buffer
 * so the ceiling is never the thing that decides whether a conversation is deliverable.
 */
export const SNAPSHOT_PART_BUDGET_BYTES = 256 * 1024

/**
 * Byte budget for one `session/history` answer.
 *
 * A page travels as a single JSON-RPC response, so it is bounded by the same 1 MiB frame
 * ceiling a snapshot used to be — and an oversized frame is dropped by the peer's reader
 * before anything can fold it, which for a page means the reader's request simply never
 * arrives (the peer's own give-up marker is all that explains it). Half the ceiling: a page
 * is one reader gesture, and asking twice is a smaller problem than a page that never lands.
 */
export const HISTORY_PAGE_BUDGET_BYTES = 512 * 1024

/**
 * Byte budget for **one record** of a history page.
 *
 * The page budget above bounds the page; on its own it does not bound a *record*, and one
 * record larger than the page budget would still travel as an oversized frame — the exact
 * failure that made long snapshots blank, one level down. So each record is clamped before
 * the page is measured, which is what makes "a page fits a frame" true for any history and
 * not only for ordinary history.
 *
 * The value mirrors the transcript's own per-entry clip — `MAX_ENTRY_BYTES`, 32 KiB, in
 * `quorfloat/src/app/session/transcript/mod.rs` — because that is the budget the receiving
 * side keeps per line: a record clamped here to what the transcript would keep anyway costs
 * the conversation nothing, while the page budget only needs a handful of records to fill.
 * It is also small enough that a page trimmed to {@link HISTORY_PAGE_BUDGET_BYTES} always
 * carries at least one whole record.
 */
export const HISTORY_RECORD_BUDGET_BYTES = 32 * 1024

/**
 * The floor of the record clamp: a string at or below this is never clipped.
 *
 * The fields that say what a record *is* — `event.type`, a role, an id, `source.kind` — are all
 * far shorter than this, and clipping one would turn a record the transcript folds into a record
 * it counts as unrecognised. Only text long enough to be payload is touched, which is the same
 * line the transcript's clip draws by clipping text rather than structure. A clipped string also
 * keeps at least this much, so every clip removes more than the marker adds.
 */
const MIN_CLIPPED_TEXT_BYTES = 1024

/**
 * How many times the record clamp may clip before it gives up and stubs the record.
 *
 * Each pass strictly shrinks the record (only strings longer than the floor are clipped, and
 * only when clipping removes more than the marker adds), so one pass is enough for every
 * ordinary record; the cap is what keeps a pathological one from looping.
 */
const MAX_CLAMP_PASSES = 8

/** The marker appended to a clipped string, mirroring the transcript's own. */
function clampNote(dropped: number): string {
  return `\n…（已截断 ${dropped} 字节；完整内容请在 Harness 窗口查看）`
}

/**
 * An upper bound of {@link clampNote}'s JSON size, used to decide how much to keep.
 *
 * Eight digits is more than any record this clamp can be handed can drop, so the estimate is
 * never short; a short one would only cost a second pass, and the bound is verified by
 * measuring the record afterwards either way.
 */
const CLAMP_MARKER_BYTES = Buffer.byteLength(clampNote(99_999_999))

/**
 * Trim a page to what one frame can carry, keeping the records nearest the window.
 *
 * Two bounds, in this order, because they answer different failures:
 *
 * - **Each record is clamped** to {@link HISTORY_RECORD_BUDGET_BYTES}. Without this the page
 *   bound below is not a bound at all: a single record over the page budget travels as an
 *   oversized frame, and the peer's reader drops it before it can be folded.
 * - **The page is trimmed** to `budget`, keeping its newest end. The panel extends its window
 *   *upwards*, so the records adjacent to what it already shows have to arrive first or the
 *   window would have a hole in it. What is left out is not lost — `cursor` names the oldest
 *   record that did travel, and the next request asks below it.
 *
 * A page that needed neither is handed back as it came, so a caller can compare identity and a
 * test can assert that a page which already fits is not rewritten.
 *
 * @param page - the page the adapter read.
 * @param budget - ceiling in bytes for the serialized records of the page.
 * @param recordBudget - ceiling in bytes for any one record; never above `budget`.
 * @returns the page, clamped and/or trimmed when it did not fit.
 */
export function boundedHistoryPage(
  page: HistoryPageView,
  budget: number = HISTORY_PAGE_BUDGET_BYTES,
  recordBudget: number = HISTORY_RECORD_BUDGET_BYTES,
): HistoryPageView {
  const records = page.records
  if (records.length === 0) return page
  const perRecord = Math.min(recordBudget, budget)
  const clamped = records.map(record => clampHistoryRecord(record, perRecord))
  const kept = clamped.map(entry => entry.record)
  const rewritten = clamped.some(entry => entry.clamped)
  if (historyPageBytes(kept) <= budget) {
    if (!rewritten) return page
    const clampedPage: HistoryPageView = { records: kept, hasMore: page.hasMore, cursor: page.cursor }
    return clampedPage
  }
  // The largest trailing slice that fits. Binary search rather than a walk: a page can hold
  // hundreds of records, and measuring each candidate slice is a serialization of the whole.
  // At least one record is kept whatever the arithmetic says: a record is the smallest thing
  // there is to send, and an empty answer would tell the reader that nothing exists below the
  // window when something does. With the budgets this module ships, a record always fits
  // anyway — the clamp above is what makes that true rather than hopeful.
  let low = 1
  let high = kept.length
  while (low < high) {
    const middle = Math.ceil((low + high) / 2)
    if (historyPageBytes(kept.slice(kept.length - middle)) <= budget) low = middle
    else high = middle - 1
  }
  const trimmed = kept.slice(kept.length - low)
  // `hasMore` is true either way: a page this function trimmed has records below the slice it
  // kept, whether or not the untrimmed page had reached the end of the history.
  return { records: trimmed, hasMore: true, cursor: oldestSeq(trimmed) }
}

/** The wire size of a page's records, in bytes. */
function historyPageBytes(records: readonly unknown[]): number {
  return Buffer.byteLength(JSON.stringify(records))
}

/** One record, and whether the clamp had to rewrite it. */
export interface ClampedRecord {
  /** The record as it will travel: the original when it already fits. */
  readonly record: unknown
  /** Whether the record was rewritten. */
  readonly clamped: boolean
}

/**
 * Clamp one record to a byte budget, keeping its envelope and marking what it lost.
 *
 * Mirrors the transcript's per-entry clip (`bounded`/`clip` in
 * `quorfloat/src/app/session/transcript/mod.rs`) at the record level: the largest strings are
 * cut, on a code-point boundary, and each one says how much was dropped. What is never touched
 * is the envelope — `event.seq` is what the page's backwards cursor is derived from, and
 * `event.type` is what the transcript folds by — because those are short by construction and
 * are the only part of a record every consumer needs.
 *
 * The budget is enforced by **measurement**, not by arithmetic: each pass clips and the record
 * is serialized again, so escaping and multi-byte characters cannot make the result larger than
 * it looks. A record whose *structure* alone is over budget (hundreds of small fields) cannot be
 * clipped into the budget at all, and is replaced by its envelope plus a marker rather than
 * travelling oversized.
 *
 * @param record - one record of a page.
 * @param budget - ceiling in bytes for this record's JSON form.
 * @returns the record to send, and whether it is not the one that was read.
 */
export function clampHistoryRecord(record: unknown, budget: number = HISTORY_RECORD_BUDGET_BYTES): ClampedRecord {
  if (recordBytes(record) <= budget) return { record, clamped: false }
  const value = cloneRecord(record)
  if (value === undefined) return { record: stubHistoryRecord(record), clamped: true }
  for (let pass = 0; pass < MAX_CLAMP_PASSES; pass += 1) {
    const over = recordBytes(value) - budget
    if (over <= 0) return { record: value, clamped: true }
    if (!clipLargestStrings(value, over)) break
  }
  if (recordBytes(value) <= budget) return { record: value, clamped: true }
  return { record: stubHistoryRecord(record), clamped: true }
}

/** The wire size of one record, in bytes. */
function recordBytes(record: unknown): number {
  const wire = JSON.stringify(record)
  return wire === undefined ? 0 : Buffer.byteLength(wire)
}

/**
 * A mutable copy of a record, through its wire form.
 *
 * A record is JSON by definition — it arrived as `SessionHistoryRecord` from the Harness — so
 * the round trip is lossless for everything this path handles, and it means the clamp never
 * reaches back into the caller's objects. `undefined` for a value that is not JSON at all.
 *
 * @param record - the record to copy.
 * @returns the copy, or `undefined` when it cannot be represented as JSON.
 */
function cloneRecord(record: unknown): unknown {
  const wire = JSON.stringify(record)
  if (wire === undefined) return undefined
  try {
    return JSON.parse(wire)
  } catch {
    return undefined
  }
}

/**
 * Clip the record's largest strings until it has lost at least `over` bytes.
 *
 * Longest first, and only strings above {@link MIN_CLIPPED_TEXT_BYTES}: the excess is taken out
 * of the text that caused it, and the short fields that carry identity are left alone. Each
 * string is kept at no less than `MIN_CLIPPED_TEXT_BYTES` bytes, so every clip removes more than
 * the marker adds and the caller's loop always makes progress.
 *
 * @param value - the record copy to rewrite, in place.
 * @param over - how many bytes must be removed.
 * @returns whether anything was clipped.
 */
function clipLargestStrings(value: unknown, over: number): boolean {
  const found: { path: (string | number)[]; text: string; bytes: number }[] = []
  collectStrings(value, [], found)
  found.sort((left, right) => right.bytes - left.bytes)
  let clipped = false
  let excess = over
  for (const entry of found) {
    if (excess <= 0) break
    const size = entry.bytes
    // Sorted longest first, so the first string at or below the floor ends the walk.
    if (size <= MIN_CLIPPED_TEXT_BYTES + CLAMP_MARKER_BYTES) break
    const keep = Math.max(MIN_CLIPPED_TEXT_BYTES, size - excess - CLAMP_MARKER_BYTES)
    if (keep >= size) continue
    const replacement = clipValue(entry.text, keep)
    // Measured rather than assumed: the marker and the boundary trim both count.
    excess -= size - Buffer.byteLength(replacement)
    setAtPath(value, entry.path, replacement)
    clipped = true
  }
  return clipped
}

/** Collect every string value of a JSON value, with the path that reaches it. */
function collectStrings(
  value: unknown,
  path: (string | number)[],
  out: { path: (string | number)[]; text: string; bytes: number }[],
): void {
  if (typeof value === 'string') {
    out.push({ path: [...path], text: value, bytes: Buffer.byteLength(value) })
    return
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => collectStrings(item, [...path, index], out))
    return
  }
  if (typeof value === 'object' && value !== null) {
    for (const [key, item] of Object.entries(value)) {
      // `__proto__` is the one key this walk cannot navigate back into (assigning to it goes
      // through the accessor rather than the own property `JSON.parse` created), so a string
      // under it is left unclipped — and the record falls to the stub path if that is what made
      // it too large.
      if (key === '__proto__') continue
      collectStrings(item, [...path, key], out)
    }
  }
}

/**
 * Replace one string in a record copy.
 *
 * Only paths this module collected are ever set, which is why a plain assignment is enough:
 * `collectStrings` skips the one key whose assignment would not be a data write.
 */
function setAtPath(root: unknown, path: readonly (string | number)[], value: string): void {
  let node = root as Record<string | number, unknown>
  for (const key of path.slice(0, -1)) node = node[key] as Record<string | number, unknown>
  node[path[path.length - 1] as string | number] = value
}

/**
 * Cut one string to at most `keep` bytes, on a code-point boundary, saying what was cut.
 *
 * Bytes rather than characters, exactly as {@link planSnapshotParts} measures: a budget
 * enforced on `String.length` would ship three times its size in CJK text. The cut is stated in
 * the text rather than applied quietly, because a clipped answer that looks complete is worse
 * than a long one — and it is the visible half of a clamp that has no other way to be noticed.
 *
 * @param text - the string to cut.
 * @param keep - how many bytes of it to keep.
 * @returns the kept prefix, on a code-point boundary, with the marker stating what was dropped.
 */
function clipValue(text: string, keep: number): string {
  let end = codePointBoundary(text, Math.min(text.length, keep))
  let used = Buffer.byteLength(text.slice(0, end))
  if (used > keep) {
    // One scaling step (a code point can be three bytes in CJK, one in ASCII), then trim the
    // remainder exactly, which is a handful of code points at most.
    end = codePointBoundary(text, Math.max(1, Math.floor((end * keep) / used)))
    used = Buffer.byteLength(text.slice(0, end))
    while (end > 0 && used > keep) {
      let start = end - 1
      if (start > 0 && isLowSurrogate(text.charCodeAt(start))) start -= 1
      used -= Buffer.byteLength(text.slice(start, end))
      end = start
    }
  }
  const dropped = Buffer.byteLength(text) - used
  return `${text.slice(0, end)}${clampNote(dropped)}`
}

/** Move a code-unit index back off a halved surrogate pair. */
function codePointBoundary(text: string, end: number): number {
  return end > 0 && end < text.length && isLowSurrogate(text.charCodeAt(end)) ? end - 1 : end
}

/** Whether a UTF-16 code unit is the second half of a surrogate pair. */
function isLowSurrogate(unit: number): boolean {
  return unit >= 0xdc00 && unit <= 0xdfff
}

/**
 * The envelope of a record whose structure alone is over budget.
 *
 * Every consumer of a page places a record by its envelope: the backwards cursor is
 * `event.seq`, the transcript folds by `event.type`, and the forward watermark needs both. The
 * payload is what cannot fit, so it is replaced by a marker that says so rather than the record
 * being sent oversized (dropped at the frame ceiling, and then nothing arrives at all).
 *
 * @param record - the record that could not be clamped.
 * @returns the envelope, with a marker payload.
 */
function stubHistoryRecord(record: unknown): unknown {
  const envelope = (typeof record === 'object' && record !== null ? record : {}) as Record<string, unknown>
  const event = (typeof envelope['event'] === 'object' && envelope['event'] !== null ? envelope['event'] : {}) as Record<string, unknown>
  return {
    type: typeof envelope['type'] === 'string' ? envelope['type'] : 'event',
    event: {
      type: typeof event['type'] === 'string' ? event['type'] : 'unknown',
      ...(typeof event['seq'] === 'number' ? { seq: event['seq'] } : {}),
      ...(typeof event['time'] === 'number' ? { time: event['time'] } : {}),
      data: { truncated: true, note: clampNote(recordBytes(record)) },
    },
  }
}

/** One bounded slice of a snapshot's records. */
export interface SnapshotPart {
  /** Position in the sequence, zero-based. */
  readonly index: number
  /** The records this part carries, in order. */
  readonly records: readonly unknown[]
  /** The part's wire size in bytes, as {@link planSnapshotParts} measured it. */
  readonly bytes: number
}

/**
 * Split a snapshot's records into parts that fit comfortably inside one frame.
 *
 * Sizes are measured in **bytes**, not characters: records are mostly text, and
 * `String.length` counts a CJK character as one where `Buffer.byteLength` counts three,
 * so a budget enforced on `length` would emit a frame three times the ceiling for exactly
 * the conversations this exists to deliver.
 *
 * A single record larger than the budget travels as its own part rather than being cut
 * in half — a record is the host's own unit, and splitting one would corrupt it. The
 * caller is told the measured size so that one case is findable rather than silent.
 *
 * @param records - the snapshot's records, in order.
 * @param context - the identity every part carries (sessionId, generation).
 * @param budget - per-part ceiling in bytes.
 * @returns the parts, in order, covering every record exactly once.
 */
export function planSnapshotParts(
  records: readonly unknown[],
  context: Record<string, unknown>,
  budget: number = SNAPSHOT_PART_BUDGET_BYTES,
): readonly SnapshotPart[] {
  const parts: SnapshotPart[] = []
  let current: unknown[] = []
  for (const record of records) {
    const grown = [...current, record]
    const size = snapshotPartBytes(context, parts.length, grown)
    // Flush only what is already there: a single record over the budget still needs a
    // part of its own, because there is no smaller unit to send.
    if (current.length > 0 && size > budget) {
      parts.push({ index: parts.length, records: current, bytes: snapshotPartBytes(context, parts.length, current) })
      current = [record]
      continue
    }
    current = grown
  }
  if (current.length > 0) {
    parts.push({ index: parts.length, records: current, bytes: snapshotPartBytes(context, parts.length, current) })
  }
  return parts
}

/**
 * The wire size of one candidate part.
 *
 * Measured through `JSON.stringify`, which is what the frame writer encodes: a
 * character count would miss the escaping and the multi-byte text that make the real
 * frame bigger than the string suggests.
 *
 * @param context - the identity fields the part carries.
 * @param index - the part's position, part of the payload.
 * @param records - the records the part would carry.
 * @returns the part's UTF-8 byte length.
 */
function snapshotPartBytes(
  context: Record<string, unknown>,
  index: number,
  records: readonly unknown[],
): number {
  return Buffer.byteLength(JSON.stringify({ ...context, index, records }))
}

/** Inputs the session layer needs from the plugin around it. */
export interface SessionLayerDeps {
  /** The Harness adapter. */
  harness: QuorfloatHarness
  /** Send one notification to the peer; failures are logged and swallowed. */
  notify(method: string, params: unknown): Promise<void>
  /** Workspace used when the caller asks for a session without naming one. */
  defaultWorkspaceId(): string
  /** Observation hook for status projection and tests. */
  onStateChange?(sessionId: string, reason: string, detail?: unknown): void
  log: SessionLayerLogger
}

/** A subscription epoch; incremented on every attach so stale frames are droppable. */
export interface FollowState {
  readonly sessionId: string
  readonly generation: number
  readonly handle: FollowHandle
  /** Highest durable sequence forwarded for this subscription; `-1` before the snapshot. */
  cursor: number
  /** The durable snapshot for this generation has been delivered. */
  snapshotDelivered: boolean
  closed: boolean
}

/** Result of an attach request. */
export interface AttachResult {
  readonly sessionId: string
  readonly generation: number
  /** True when an earlier subscription for the same session was replaced. */
  readonly replaced: boolean
}

/**
 * One session's prompt bookkeeping, kept so a reconnect cannot resend input.
 *
 * `requestId` is the caller's idempotency key and is generated here, once per
 * user submission. `admitted` records that the Host acknowledged it; an
 * `admitted` entry is never re-sent, and an entry that is neither admitted nor
 * failed is reported as "awaiting confirmation" instead of being retried.
 */
interface PromptRecord {
  readonly requestId: string
  readonly sessionId: string
  readonly text: string
  state: 'pending' | 'admitted' | 'failed'
  detail?: unknown
}

/**
 * The prompt key a user message was created from, when it has one.
 *
 * The shape is the Harness's own: a `user/message` carries `source: {kind: 'user', rpcId}`, and `rpcId`
 * is the `requestId` the prompt was sent with (`packages/api/session-controller/src/commands.ts`).
 *
 * @param data - one event's `data`.
 * @returns the key, or `undefined` for anything that is not a keyed user message.
 */
function promptKeyIn(data: unknown): string | undefined {
  if (typeof data !== 'object' || data === null) return undefined
  const source = (data as Record<string, unknown>)['source']
  if (typeof source !== 'object' || source === null) return undefined
  const { kind, rpcId } = source as { kind?: unknown; rpcId?: unknown }
  if (kind !== 'user' || typeof rpcId !== 'string' || rpcId === '') return undefined
  return rpcId
}

/**
 * The `data` of the event inside one snapshot record.
 *
 * A snapshot's `records` are event envelopes (`{type, event}`), while a live frame *is* the event, so
 * the two arrive in different shapes and are reduced to one here.
 *
 * @param record - one entry of a snapshot's `records`.
 * @returns the event's `data`, or `undefined` if the record is not an event envelope.
 */
function eventDataIn(record: unknown): unknown {
  if (typeof record !== 'object' || record === null) return undefined
  const event = (record as Record<string, unknown>)['event']
  if (typeof event !== 'object' || event === null) return undefined
  return (event as Record<string, unknown>)['data']
}

export class SessionLayer {
  readonly #deps: SessionLayerDeps
  readonly #follows = new Map<string, FollowState>()
  readonly #prompts = new Map<string, PromptRecord>()
  /**
   * Prompt keys each conversation has *already used*, as seen in its own history.
   *
   * Separate from {@link SessionLayer.#prompts}, which remembers only what this process sent: the
   * Harness remembers a key for the life of the conversation, so a key can be spent by a **previous
   * run of the panel**. See {@link SessionLayer.prompt} for what a spent key costs.
   */
  readonly #usedKeys = new Map<string, Set<string>>()
  /** Session ids the user is currently looking at, newest attach wins. */
  #activeSessionId: string | undefined
  /** Monotonic generation source, per layer (not per session). */
  #generation = 0

  /**
   * @param deps - adapter, sender, defaults, and logger.
   */
  constructor(deps: SessionLayerDeps) {
    this.#deps = deps
  }

  /** Session currently attached by the peer, if any. */
  get activeSessionId(): string | undefined {
    return this.#activeSessionId
  }

  /** Session ids this layer owns; used as the interaction-ownership predicate. */
  ownedSessionIds(): readonly string[] {
    return [...this.#follows.keys()]
  }

  /** Snapshot of subscription state for diagnostics. */
  describe(): Record<string, unknown> {
    return {
      activeSessionId: this.#activeSessionId ?? null,
      follows: [...this.#follows.values()].map(state => ({
        sessionId: state.sessionId,
        generation: state.generation,
        cursor: state.cursor,
        snapshotDelivered: state.snapshotDelivered,
      })),
      prompts: [...this.#prompts.values()].map(record => ({
        requestId: record.requestId,
        sessionId: record.sessionId,
        state: record.state,
      })),
    }
  }

  /** List workspaces, required to be non-empty before a session can be created. */
  listWorkspaces(): readonly WorkspaceView[] {
    return this.#deps.harness.listWorkspaces()
  }

  /**
   * Initialize (or resolve) the Harness's first-use default workspace.
   *
   * A thin pass-through, like {@link SessionLayer.listWorkspaces}: the adapter owns what
   * "initialize" means upstream, and this layer owns only the fact that the peer asked. The
   * adapter's contract is "never throws, empty list means none" — so the peer's answer is
   * always a list, and the panel's `defaultWorkspace` state is derived from whether it has one.
   */
  async initializeDefaultWorkspace(): Promise<readonly WorkspaceView[]> {
    return await this.#deps.harness.initializeDefaultWorkspace()
  }

  /** List Harness sessions. */
  async listSessions(): Promise<readonly SessionSummaryView[]> {
    return await this.#deps.harness.listSessions()
  }

  /**
   * Create a session in an explicitly chosen workspace.
   *
   * @param workspaceId - caller-selected workspace; empty selects the configured default.
   * @returns the new session identity.
   * @throws {HarnessError} with code `unavailable` when no workspace can be chosen.
   */
  async createSession(workspaceId: string): Promise<{ sessionId: string }> {
    const chosen = workspaceId !== '' ? workspaceId : this.#deps.defaultWorkspaceId()
    if (chosen === '') {
      throw new HarnessError('unavailable', 'no workspace selected and no default workspace configured', {
        hint: 'Set defaultWorkspaceId in the plugin config, or pick a workspace in the panel.',
      })
    }
    const workspace = this.#deps.harness.workspace(chosen)
    if (workspace === undefined) {
      const known = this.#deps.harness.listWorkspaces().map(entry => entry.workspaceId)
      throw new HarnessError('stale', `workspace ${chosen} is no longer registered`, { workspaceId: chosen, known })
    }
    const created = await this.#deps.harness.createSession(chosen)
    this.#deps.onStateChange?.(created.sessionId, 'created', { workspaceId: chosen })
    return created
  }

  /**
   * Attach (or re-attach) one session and start forwarding its frames.
   *
   * Re-attaching the same session replaces the previous subscription instead of
   * running two: two subscriptions would double every frame and make the cursor
   * meaningless.
   *
   * @param sessionId - durable session identity.
   * @returns the subscription identity.
   */
  async attach(sessionId: string): Promise<AttachResult> {
    const previous = this.#follows.get(sessionId)
    if (previous !== undefined) this.#closeFollow(previous, 'replaced')
    const generation = ++this.#generation
    // Frames are forwarded **in order**, one at a time. A snapshot is no longer one
    // notification but a sequence, and an event forwarded while its parts were still
    // being written would reach the peer ahead of the conversation it belongs to — and
    // be counted stale there, which is precisely the bug chunking exists to fix.
    let chain: Promise<void> = Promise.resolve()
    const handle = this.#deps.harness.follow(sessionId, frame => {
      chain = chain
        .then(() => this.#forward(sessionId, generation, frame))
        .catch(error => {
          // A subscriber that throws must not tear down the durable subscription, and
          // an unhandled rejection here would surface as a plugin crash.
          this.#deps.log.warn('forwarding a follow frame failed', { sessionId, generation, error: String(error) })
        })
    })
    const state: FollowState = {
      sessionId,
      generation,
      handle,
      cursor: -1,
      snapshotDelivered: false,
      closed: false,
    }
    this.#follows.set(sessionId, state)
    this.#activeSessionId = sessionId
    this.#deps.onStateChange?.(sessionId, 'attached', { generation, replaced: previous !== undefined })
    // The subscription is durable but its frames are asynchronous; a subscriber
    // that never receives a snapshot must be told so by the absence of frames,
    // not by a fabricated empty conversation.
    void handle.done
      .then(() => {
        if (state.closed) return
        this.#deps.log.warn('follow subscription ended unexpectedly', { sessionId, generation })
        this.#deps.onStateChange?.(sessionId, 'subscription-lost', { generation })
      })
      .catch(error => {
        // A subscription that fails to even start must be reported, not thrown
        // into the host's shared event chain.
        this.#deps.log.warn('follow subscription failed', { sessionId, generation, error: String(error) })
        this.#deps.onStateChange?.(sessionId, 'subscription-failed', { generation })
      })
    return { sessionId, generation, replaced: previous !== undefined }
  }

  /** Stop following one session; the Harness-side task keeps running. */
  detach(sessionId: string): boolean {
    const state = this.#follows.get(sessionId)
    if (state === undefined) return false
    this.#closeFollow(state, 'detached')
    this.#follows.delete(sessionId)
    if (this.#activeSessionId === sessionId) this.#activeSessionId = undefined
    return true
  }

  /**
   * Send one conversation's opening snapshot again.
   *
   * The peer asks for this when a `session/snapshot` sequence did not complete — a part
   * was refused at the frame ceiling, or the sequence stalled — and the alternative to a
   * resend is a panel that stays blank for the rest of the run. Re-attaching is the
   * recovery that exists: it is the same path that delivered the snapshot the first time,
   * so the records are re-read rather than replayed from a cache that could have gone
   * stale. The peer bounds how often it asks (per attach), so this cannot become a loop
   * between the two halves.
   *
   * @param sessionId - the conversation whose snapshot is wanted.
   * @returns the subscription identity the resend was opened on.
   */
  async requestSnapshot(sessionId: string): Promise<{ sessionId: string; generation: number; accepted: true }> {
    const result = await this.attach(sessionId)
    this.#deps.onStateChange?.(sessionId, 'snapshot-requested', { generation: result.generation })
    return { sessionId, generation: result.generation, accepted: true }
  }

  /**
   * Push one session's statistics to the peer.
   *
   * Called on attach and once per finished turn rather than per frame: the numbers only change when
   * a step closes, and the read is a projection walk. It is also fired after a prompt is admitted,
   * because that is when `running` flips and the panel's footer starts showing progress.
   *
   * A failure is logged and swallowed — statistics are an adornment, and no session call may fail
   * because a counter could not be read.
   *
   * @param sessionId - the session to report on.
   */
  async refreshStats(sessionId: string): Promise<void> {
    let stats: SessionStatsView | undefined
    try {
      stats = await this.#deps.harness.stats(sessionId)
    } catch (error) {
      this.#deps.log.debug('statistics read failed', { sessionId, error: String(error) })
      return
    }
    if (stats === undefined) return
    await this.#deps.notify('session/stats', { sessionId, stats })
  }

  /** Read the model and permission choices the panel offers. */
  async readOptions(sessionId?: string): Promise<SessionOptionsView> {
    return await this.#deps.harness.options(sessionId)
  }

  /** Switch one session's model and reasoning effort. */
  async selectModel(sessionId: string, selection: ModelChoiceView): Promise<void> {
    await this.#deps.harness.selectModel(sessionId, selection)
    // The picker's own answer is the projection the host just wrote; re-reading it here means the
    // panel shows what was *accepted* rather than what it asked for.
    this.#deps.onStateChange?.(sessionId, 'model-selected', selection)
  }

  /** Apply one permission preset. */
  async setPermission(sessionId: string, value: string): Promise<void> {
    await this.#deps.harness.setPermission(sessionId, value)
    this.#deps.onStateChange?.(sessionId, 'permission-set', { value })
  }

  /** Stop every subscription; used on plugin unload and on channel loss. */
  detachAll(reason: string): void {
    for (const state of this.#follows.values()) this.#closeFollow(state, reason)
    this.#follows.clear()
    this.#activeSessionId = undefined
  }

  /**
   * Read one page of history.
   *
   * A page is cursor-relative upstream, so the bound is resolved in this order:
   * the live subscription's cursor (the peer's own position, and the only bound
   * that can include events that arrived while it was reading), then the
   * cold-safe persisted cursor. With neither, the session has no events and an
   * empty page is the truthful answer — inventing a large bound would be
   * rejected by Harness, and guessing would risk a page the caller cannot place.
   *
   * @param sessionId - durable session identity.
   * @param beforeSeq - exclusive backwards cursor within the bound.
   * @returns the page, bounded to one frame, plus the backwards cursor for the page below it
   *   (`cursor`, the Harness's own derivation — see `oldestSeq` in `harness/adapter.ts`).
   */
  async readHistory(
    sessionId: string,
    beforeSeq?: number,
  ): Promise<{ records: readonly unknown[]; hasMore: boolean; cursor: number | null }> {
    const subscription = this.#follows.get(sessionId)
    let throughSeq = subscription !== undefined && subscription.cursor >= 0 ? subscription.cursor : undefined
    if (throughSeq === undefined) throughSeq = await this.#deps.harness.cursor(sessionId)
    if (throughSeq === undefined || throughSeq < 0) {
      this.#deps.log.debug('history requested for a session with no events', { sessionId })
      return { records: [], hasMore: false, cursor: null }
    }
    return boundedHistoryPage(await this.#deps.harness.pageHistory(sessionId, throughSeq, beforeSeq))
  }

  /**
   * Admit one prompt exactly once per user submission.
   *
   * `requestId` is generated by the caller (the peer) and remembered here. A
   * repeat of the same key returns the original acceptance; a *new* key for the
   * same text is a new user action and is admitted again, because only the peer
   * can tell "the user typed it twice" from "the frame was retried".
   *
   * @param sessionId - target session.
   * @param requestId - caller-owned idempotency key.
   * @param text - prompt text.
   */
  async prompt(sessionId: string, requestId: string, text: string): Promise<{ accepted: true }> {
    const existing = this.#prompts.get(requestId)
    if (existing !== undefined) {
      if (existing.state === 'admitted') return { accepted: true }
      if (existing.state === 'pending') {
        throw new HarnessError('unavailable', 'the same prompt is still awaiting confirmation from Harness', { requestId })
      }
      throw new HarnessError('unavailable', 'this prompt already failed; submit it again to retry deliberately', {
        requestId,
        detail: existing.detail,
      })
    }
    // **A key this conversation has already spent can never be admitted**, whatever the peer believes.
    // The Harness's session controller answers `{accepted: true}` to a repeated `requestId` *without
    // admitting anything* — `hasPromptRequest` matches the `user/message` whose `source.rpcId` is that
    // key — so forwarding it would tell the peer a message was delivered that never was. A restarted
    // panel can reuse `session-…:prompt-1` for a conversation that already has it, clearing its input
    // while nothing is sent. The peer keeps the text when it hears a refusal, so refusing is the half
    // that makes the mistake recoverable; the log line is the half that makes it findable.
    if (this.#usedKeys.get(sessionId)?.has(requestId) === true) {
      this.#deps.log.warn('refused a prompt whose key this conversation has already used', {
        sessionId,
        requestId,
      })
      this.#deps.onStateChange?.(sessionId, 'prompt-key-reused', { requestId })
      // The message is Chinese and leads with the action, because it is read in the panel's status line,
      // which is Chinese and truncates on the right — the same surface the layer's other reasons reach
      // ("正在获取工作区，请稍候再发送"). The log line above stays English, like every other host log.
      throw new HarnessError(
        'stale',
        '请再按一次发送：这条消息的幂等键本会话已经用过，Harness 会丢弃它',
        { sessionId, requestId },
      )
    }
    const record: PromptRecord = { requestId, sessionId, text, state: 'pending' }
    this.#prompts.set(requestId, record)
    try {
      const result = await this.#deps.harness.prompt(sessionId, requestId, text)
      record.state = 'admitted'
      this.#deps.onStateChange?.(sessionId, 'prompt-admitted', { requestId })
      return result
    } catch (error) {
      record.state = 'failed'
      record.detail = error instanceof Error ? error.message : String(error)
      throw error
    }
  }

  /**
   * Remember the prompt keys a conversation's history carries.
   *
   * Best effort on purpose: the opening snapshot is paged (it reports `hasMore`), so a key spent far
   * enough back may never be seen, and a guard that cannot see a key lets it through. That direction is
   * the safe one — a missed refusal costs what a refusal would have saved, while a *wrong* refusal would
   * refuse a message that could have been sent.
   *
   * @param sessionId - the conversation the records belong to.
   * @param datas - event `data` values, from a snapshot's records or from one event frame.
   */
  #rememberUsedKeys(sessionId: string, datas: readonly unknown[]): void {
    let keys: Set<string> | undefined
    for (const data of datas) {
      const key = promptKeyIn(data)
      if (key === undefined) continue
      keys ??= this.#usedKeysFor(sessionId)
      keys.add(key)
    }
  }

  /**
   * The key set for one conversation, created on first use.
   *
   * @param sessionId - the conversation.
   * @returns its set, to add to.
   */
  #usedKeysFor(sessionId: string): Set<string> {
    const existing = this.#usedKeys.get(sessionId)
    if (existing !== undefined) return existing
    const created = new Set<string>()
    this.#usedKeys.set(sessionId, created)
    return created
  }

  /**
   * Request cancellation.
   *
   * @param sessionId - target session.
   */
  async cancel(sessionId: string): Promise<{ accepted: true }> {
    const result = await this.#deps.harness.cancel(sessionId)
    // Acceptance means the request was received. The authoritative outcome is a
    // `turn/end` event with an aborted reason; until then the peer must keep
    // showing a cancelling state, so no terminal state is published here.
    this.#deps.onStateChange?.(sessionId, 'cancel-requested', {})
    return result
  }

  /**
   * Forward one upstream frame as a protocol notification.
   *
   * @param sessionId - the session the frame belongs to.
   * @param generation - the subscription generation that produced it.
   * @param frame - the projected frame.
   */
  async #forward(sessionId: string, generation: number, frame: FollowFrame): Promise<void> {
    const state = this.#follows.get(sessionId)
    if (state === undefined || state.generation !== generation || state.closed) {
      // A frame from a replaced subscription must not reach the peer: it would
      // append content the peer already received from the newer subscription.
      this.#deps.log.debug('dropped a frame from a replaced subscription', { sessionId, generation })
      return
    }
    switch (frame.type) {
      case 'snapshot': {
        state.snapshotDelivered = true
        state.cursor = frame.cursor
        // Before the peer sees the snapshot: it may prompt the moment it has it, and a prompt judged
        // against a stale key set is a prompt that slips through the guard below.
        this.#rememberUsedKeys(sessionId, frame.records.map(record => eventDataIn(record)))
        await this.#sendSnapshot(sessionId, generation, frame, state)
        // The snapshot carries projections, but only this one: their later updates travel on a
        // different stream the panel does not open. So the first statistics are sent here, and
        // refreshed when a turn closes.
        await this.refreshStats(sessionId)
        return
      }
      case 'event': {
        if (!state.snapshotDelivered) {
          // Events before the snapshot would be applied to an unknown baseline.
          // The protocol's rule is to wait, not to guess an ordering.
          this.#deps.log.warn('dropped an event that arrived before the snapshot', { sessionId, seq: frame.seq })
          return
        }
        if (frame.seq <= state.cursor) {
          this.#deps.log.debug('dropped a duplicate event', { sessionId, seq: frame.seq, cursor: state.cursor })
          return
        }
        if (frame.seq !== state.cursor + 1) {
          // A gap means this subscription can no longer be trusted to be
          // gap-free; the peer is told to re-attach rather than shown a
          // conversation with a hole in it.
          this.#deps.log.warn('sequence gap detected; requesting resync', {
            sessionId,
            expected: state.cursor + 1,
            received: frame.seq,
          })
          await this.#deps.notify('session/resync', {
            sessionId,
            reason: 'sequence-gap',
            expected: state.cursor + 1,
            received: frame.seq,
          })
          return
        }
        state.cursor = frame.seq
        if (frame.eventType === 'user/message') this.#rememberUsedKeys(sessionId, [frame.data])
        await this.#deps.notify('session/event', {
          sessionId,
          generation,
          seq: frame.seq,
          type: frame.eventType,
          time: frame.time,
          data: frame.data,
        })
        // The one event that makes the statistics stale: `turn/end` closes the last step of a turn,
        // which is exactly when turns, steps, decode time and token usage stop changing. Awaited so
        // the panel receives the totals no later than the event that produced them.
        if (frame.eventType === 'turn/end' || frame.eventType === 'step/end') {
          await this.refreshStats(sessionId)
        }
        return
      }
      case 'assistant-stream': {
        if (!state.snapshotDelivered) return
        await this.#deps.notify('session/stream', { sessionId, generation, frame: frame.frame })
        return
      }
      default: {
        this.#deps.log.debug('ignored an unknown frame type')
      }
    }
  }

  /**
   * Send one conversation's opening snapshot as `begin` + parts + `end`.
   *
   * The chunks are what make a conversation larger than the frame ceiling deliverable:
   * a single frame past 1 MiB is refused by the peer's reader, and the refusal used to
   * leave the panel blank for the rest of the run.
   *
   * `state.closed` is checked between parts because a replaced subscription's remaining
   * parts belong to a conversation nobody is following: writing them would only make the
   * peer discard them (its own identity check) after spending the bandwidth.
   *
   * @param sessionId - durable session identity.
   * @param generation - the subscription generation these parts belong to.
   * @param frame - the upstream snapshot.
   * @param state - the subscription the frame arrived on.
   */
  async #sendSnapshot(
    sessionId: string,
    generation: number,
    frame: SnapshotFrame,
    state: FollowState,
  ): Promise<void> {
    const context = { sessionId, generation }
    const parts = planSnapshotParts(frame.records, context)
    for (const part of parts) {
      if (part.bytes > SNAPSHOT_PART_BUDGET_BYTES) {
        // Not fatal — the part still travels — but the one case the budget cannot
        // cover must be findable rather than mysterious if it ever exceeds the ceiling.
        this.#deps.log.debug('a snapshot record is larger than the part budget; it travels as its own part', {
          sessionId,
          index: part.index,
          bytes: part.bytes,
        })
      }
    }
    await this.#deps.notify('session/snapshot/begin', {
      sessionId,
      generation,
      cursor: frame.cursor,
      hasMore: frame.hasMore,
      parts: parts.length,
      projections: frame.projections,
      assistantStream: frame.assistantStream,
    })
    for (const part of parts) {
      if (state.closed) return
      await this.#deps.notify('session/snapshot/part', {
        sessionId,
        generation,
        index: part.index,
        records: part.records,
      })
    }
    if (state.closed) return
    await this.#deps.notify('session/snapshot/end', { sessionId, generation, parts: parts.length })
  }

  /**
   * Close one subscription and mark it so late frames are dropped.
   *
   * @param state - the subscription to close.
   * @param reason - why it is being closed.
   */
  #closeFollow(state: FollowState, reason: string): void {
    if (state.closed) return
    state.closed = true
    try {
      state.handle.close()
    } catch (error) {
      this.#deps.log.warn('closing a follow subscription failed', error)
    }
    this.#deps.onStateChange?.(state.sessionId, 'detached', { generation: state.generation, reason })
  }
}
