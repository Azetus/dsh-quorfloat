/**
 * The Harness's workspace change stream, and the wire frames it becomes.
 *
 * The workspace list the panel draws cannot be a snapshot taken once: a workspace added or
 * removed in the Harness window changes it, and the panel only ever asked for it on demand and
 * then cached the answer. The Harness has a typed push stream for exactly this
 * (`workspaceController.follow`), and this module is the whole host half of it: one subscription
 * whose frames are projected into this plugin's own vocabulary and handed to the caller, which
 * forwards them over the existing host→peer notification channel as `workspaces/changed`.
 *
 * Two boundaries are drawn deliberately:
 *
 * - **Upstream shape lives here**, not in `src/index.ts`. Only the one method this plugin calls
 *   is declared, and upstream's ISO `createdAt` becomes the epoch milliseconds the rest of the
 *   project uses exactly once, through the shared {@link projectWorkspace}.
 * - **A missing service or method is reported, never fatal.** A profile without the workspace
 *   API composes no `workspaceController`, and a build without the stream has no `follow`; both
 *   leave the plugin fully usable, because `workspaces/list` remains the fallback path. The
 *   stream is an improvement on that path, not a requirement for it.
 */

import { projectWorkspace, projectWorkspaces, readSessionIds, type WorkspaceView } from './adapter.js'

/** One complete workspace list, as the stream's opening frame carries it. */
export interface WorkspaceBaseline {
  /** Every workspace, in the registry's display order. */
  readonly items: readonly WorkspaceView[]
  /** Conversations hidden from the Harness's grouping surfaces. */
  readonly archivedSessionIds: readonly string[]
  /** The Harness's own session pins, which are **not** this panel's workspace pin. */
  readonly pinnedSessionIds: readonly string[]
}

/**
 * One workspace change, in this plugin's own vocabulary.
 *
 * The shape deliberately mirrors the upstream frame it comes from — `baseline` carries a
 * `value`, the increments carry their own field — so the two are recognisably the same protocol
 * without the sidecar having to know upstream's wrappers.
 */
export type WorkspaceChangeFrame =
  | { readonly type: 'baseline'; readonly value: WorkspaceBaseline }
  | { readonly type: 'upsert'; readonly workspace: WorkspaceView }
  | { readonly type: 'remove'; readonly workspaceId: string }
  | { readonly type: 'order'; readonly workspaceIds: readonly string[] }
  | { readonly type: 'archived'; readonly archivedSessionIds: readonly string[] }
  | { readonly type: 'pinned'; readonly pinnedSessionIds: readonly string[] }

/**
 * The one upstream method this module calls on the workspace controller.
 *
 * `follow` is a typed push stream rather than a poll: it opens with a complete
 * `baseline` and then yields ordered increments for the life of the `signal`.
 */
interface WorkspaceFollowSource {
  follow(signal: AbortSignal): AsyncIterable<unknown>
}

/** Minimal logger shape this module needs. */
export interface WorkspaceStreamLogger {
  warn(message: string, detail?: unknown): void
  debug(message: string, detail?: unknown): void
}

/** Everything one subscription needs from the plugin around it. */
export interface WorkspaceStreamDeps {
  /** The `workspaceController` service, or `undefined` when this profile has none. */
  readonly controller: unknown
  /** Plugin lifetime. Aborting it ends the subscription. */
  readonly signal: AbortSignal
  /** Called for every recognised frame, in order; failures are logged and survived. */
  readonly onFrame: (frame: WorkspaceChangeFrame) => void | Promise<void>
  readonly log: WorkspaceStreamLogger
}

/** A live subscription. `close` ends it; `done` settles when it has. */
export interface WorkspaceStreamHandle {
  /** Resolves once the subscription has ended, for any reason. */
  readonly done: Promise<void>
  /** Stop following. Safe to call more than once. */
  close(): void
}

/** Read an array of non-empty strings, dropping anything else. */
function readIds(value: unknown): readonly string[] {
  if (!Array.isArray(value)) return []
  return value.filter((id): id is string => typeof id === 'string' && id !== '')
}

/**
 * Project one upstream frame into this plugin's own vocabulary.
 *
 * Every arm is defensive: a nightly build that adds a frame type or changes a field yields
 * `undefined` (dropped and reported) rather than a half-applied list. `archived` and `pinned` are
 * carried through unchanged even though the panel ignores them today — dropping them here would
 * make a future consumer's work a protocol change instead of a parser change.
 *
 * @param frame - one frame from the upstream stream.
 * @returns the frame to forward, or `undefined` when this build does not recognise it.
 */
export function projectWorkspaceFrame(frame: unknown): WorkspaceChangeFrame | undefined {
  if (typeof frame !== 'object' || frame === null) return undefined
  const record = frame as Record<string, unknown>
  switch (record['type']) {
    case 'baseline': {
      const value = record['value']
      if (typeof value !== 'object' || value === null) return undefined
      const baseline = value as Record<string, unknown>
      return {
        type: 'baseline',
        value: {
          items: Array.isArray(baseline['items']) ? projectWorkspaces(baseline['items']) : [],
          archivedSessionIds: readSessionIds(baseline['archivedSessionIds']),
          pinnedSessionIds: readSessionIds(baseline['pinnedSessionIds']),
        },
      }
    }
    case 'upsert': {
      const workspace = projectWorkspace(record['workspace'])
      return workspace === undefined ? undefined : { type: 'upsert', workspace }
    }
    case 'remove': {
      const workspaceId = record['workspaceId']
      if (typeof workspaceId !== 'string' || workspaceId === '') return undefined
      return { type: 'remove', workspaceId }
    }
    case 'order':
      // An order frame is an ordering, not a census, and an empty one is still meaningful:
      // "nothing is ordered" leaves the list as it is rather than clearing it.
      if (!Array.isArray(record['workspaceIds'])) return undefined
      return { type: 'order', workspaceIds: readIds(record['workspaceIds']) }
    case 'archived':
      return { type: 'archived', archivedSessionIds: readSessionIds(record['archivedSessionIds']) }
    case 'pinned':
      return { type: 'pinned', pinnedSessionIds: readSessionIds(record['pinnedSessionIds']) }
    default:
      return undefined
  }
}

/**
 * Subscribe to the Harness's workspace stream.
 *
 * The iteration is driven here rather than handed to the caller, for the same reason the session
 * follow is: the only failure modes are "the stream ended" and "the stream threw", and both are
 * reported through the log rather than escaping as an unhandled rejection.
 *
 * A missing service, a missing method, and a `follow` that throws are all the same outcome — a
 * resolved handle and one warning — because the plugin's fallback (`workspaces/list`) is already
 * in place and a panel with a stale list is worse than useless but better than a plugin that
 * refuses to load.
 *
 * @param deps - controller, lifetime signal, frame sink, and logger.
 * @returns a handle that stops the subscription.
 */
export function startWorkspaceStream(deps: WorkspaceStreamDeps): WorkspaceStreamHandle {
  if (deps.signal.aborted) return { done: Promise.resolve(), close: () => {} }

  const abort = new AbortController()
  const forwardAbort = (): void => {
    if (!abort.signal.aborted) abort.abort(new Error('dsh-quorfloat: plugin lifetime ended'))
  }
  deps.signal.addEventListener('abort', forwardAbort, { once: true })

  const source = deps.controller as Partial<WorkspaceFollowSource> | undefined
  if (source === undefined || typeof source.follow !== 'function') {
    // Said out loud, because "the panel's workspace list never changes" has two causes — a
    // Harness without the stream, and a bug in this subscription — and they are identical from
    // the panel alone.
    deps.log.warn('this Harness has no workspace change stream; the panel keeps asking workspaces/list', {
      service: 'workspaceController',
      method: 'follow',
    })
    deps.signal.removeEventListener('abort', forwardAbort)
    return { done: Promise.resolve(), close: () => {} }
  }

  let iterator: AsyncIterable<unknown>
  try {
    iterator = source.follow(abort.signal)
  } catch (error) {
    deps.log.warn('the workspace change stream could not be opened', error)
    deps.signal.removeEventListener('abort', forwardAbort)
    return { done: Promise.resolve(), close: () => {} }
  }

  const done = (async () => {
    try {
      for await (const frame of iterator) {
        if (abort.signal.aborted) break
        const projected = projectWorkspaceFrame(frame)
        if (projected === undefined) {
          deps.log.debug('ignored a workspace frame this build does not recognise', frame)
          continue
        }
        try {
          await deps.onFrame(projected)
        } catch (error) {
          // A failed forward must not end the subscription: the next change still has to reach
          // the panel, and the channel that dropped this frame reports its own failure.
          deps.log.warn('could not forward a workspace change', error)
        }
      }
    } catch (error) {
      if (!abort.signal.aborted) deps.log.warn('the workspace change stream ended with an error', error)
    } finally {
      deps.signal.removeEventListener('abort', forwardAbort)
    }
  })()

  return {
    done,
    close: () => {
      if (!abort.signal.aborted) abort.abort(new Error('dsh-quorfloat: workspace stream closed by host'))
    },
  }
}
