/**
 * The Harness's session list push, and the frames it becomes.
 *
 * The panel's conversation list is a view of one fact: which conversations exist, what they are
 * called, and which one is running. It used to be whatever `sessions/list` said the last time the
 * sidecar asked, which is why that list was polled. The Harness already emits the changes — the
 * `api-session/*` events declared by its session controller — and its own browser client keeps
 * its list from exactly those events, so this module is the host half of doing the same here:
 * subscribe once, project each event into this plugin's vocabulary, and let the sidecar merge it
 * into the list it draws.
 *
 * Two boundaries are drawn on purpose:
 *
 * - **Upstream shape lives here.** A row is projected through the same
 *   {@link projectSessionSummary} the `sessions/list` answer uses, so the pushed row and the
 *   polled row cannot disagree about which field carries the name or which sessions are visible.
 * - **Nothing here is load-bearing.** Events are best-effort; a dropped frame is repaired by the
 *   sidecar's slow reconcile, and a Harness without the projection feed still forwards every
 *   list event (titles then follow the reconcile instead of arriving live).
 */

import type { PluginContext } from '../cordis-types.js'
import { projectSessionSummary, type SessionSummaryView } from './adapter.js'

/**
 * One conversation-list change, in this plugin's own vocabulary.
 *
 * The shapes mirror what the sidecar can apply without re-deriving anything: `upsert` carries a
 * whole row, the other increments carry one field. `refresh` is not a row change at all — it says
 * "the cached list may be wrong, ask for the authoritative one" — which is what a removal needs,
 * because "left the live registry" is not the same fact as "gone from the persisted list".
 */
export type SessionChangeFrame =
  | { readonly type: 'upsert'; readonly session: SessionSummaryView }
  | { readonly type: 'status'; readonly sessionId: string; readonly running: boolean }
  | { readonly type: 'activity'; readonly sessionId: string; readonly updatedAt: number }
  | { readonly type: 'title'; readonly sessionId: string; readonly title: string | null }
  | { readonly type: 'refresh'; readonly reason: string }

/** Minimal logger shape this module needs. */
export interface SessionStreamLogger {
  warn(message: string, detail?: unknown): void
  debug(message: string, detail?: unknown): void
}

/** The one method of `sessionProjections` this module subscribes to. */
interface ProjectionChangeSource {
  onChanged(
    listener: (session: unknown, key: string, value: unknown, seq: unknown) => void,
  ): () => void
}

/** Everything one subscription needs from the plugin around it. */
export interface SessionStreamDeps {
  /** The plugin context: event subscriptions are owned by this fiber. */
  readonly ctx: PluginContext
  /** Called for every recognised frame, in order; failures are reported and survived. */
  readonly onFrame: (frame: SessionChangeFrame) => void | Promise<void>
  readonly log: SessionStreamLogger
}

/** A live subscription. `close` ends it; `settled` waits for the writes it started. */
export interface SessionStreamHandle {
  /** Stop listening. Safe to call more than once. */
  close(): void
  /** Resolves once every frame forwarded so far has been written. */
  settled(): Promise<void>
}

/**
 * Subscribe to the Harness's session changes.
 *
 * The five events are the session controller's declared Host events, not an inference: the
 * installed contract lists them as `emit`, which is what makes `ctx.on` the supported entry.
 * A Harness that composes no `sessionProjections` still gets every list event; its titles are
 * simply not live, and that is reported once rather than silently.
 *
 * Frames are delivered in order. A frame handler that returns a promise is awaited before the
 * next frame is handed over, so a slow write cannot let a status change overtake the upsert it
 * belongs to.
 *
 * @param deps - context, frame sink, and logger.
 * @returns a handle that stops the subscription.
 */
export function startSessionStream(deps: SessionStreamDeps): SessionStreamHandle {
  const disposers: Array<() => void> = []
  let closed = false
  let writes: Promise<void> = Promise.resolve()

  const forward = (frame: SessionChangeFrame): void => {
    if (closed) return
    writes = writes.then(async () => {
      if (closed) return
      try {
        await deps.onFrame(frame)
      } catch (error) {
        // A failed forward must not end the subscription or escape the host's synchronous
        // event dispatch: the next change still has to reach the panel, and the channel that
        // dropped this frame reports its own failure.
        deps.log.warn('could not forward a session change', error)
      }
    })
  }

  const subscribe = (name: string, listener: (...args: any[]) => void): void => {
    try {
      disposers.push(deps.ctx.on(name, listener))
    } catch (error) {
      deps.log.warn(`could not subscribe to ${name}; the list will only be reconciled`, error)
    }
  }

  // A row, complete: the list's own projection, subagents included as the filter removes them.
  subscribe('api-session/added', (summary: unknown) => {
    const session = projectSessionSummary(summary)
    if (session !== undefined) forward({ type: 'upsert', session })
  })
  // Running is a display fact, and `blank` is derived from it on the sidecar: the Harness's own
  // client does the same, so a conversation that starts working leaves the "empty" hiding rule.
  subscribe('api-session/status', (sessionId: unknown, running: unknown) => {
    if (typeof sessionId !== 'string' || sessionId === '') return
    if (typeof running !== 'boolean') return
    forward({ type: 'status', sessionId, running })
  })
  // The only source of list order: a user message is what moves a conversation to the front.
  subscribe('api-session/activity', (sessionId: unknown, updatedAt: unknown) => {
    if (typeof sessionId !== 'string' || sessionId === '') return
    if (typeof updatedAt !== 'number' || !Number.isFinite(updatedAt)) return
    forward({ type: 'activity', sessionId, updatedAt })
  })
  // "Left the live registry", which is not "gone from the persisted list" (see the frame docs).
  subscribe('api-session/removed', (sessionId: unknown) => {
    if (typeof sessionId !== 'string' || sessionId === '') return
    forward({ type: 'refresh', reason: 'removed' })
  })

  // Titles are a projection, not a list event: the Harness's own control stream is built on this
  // same feed, and the list row carries the same value under `projections.values.title`. Only
  // the name is taken — the feed also carries token usage and turn boundaries, which the panel
  // gets from the conversation itself.
  const projections = deps.ctx.get('sessionProjections') as Partial<ProjectionChangeSource> | undefined
  if (projections !== undefined && typeof projections.onChanged === 'function') {
    try {
      disposers.push(projections.onChanged((session, key, value) => {
        if (key !== 'title') return
        const record = session as { id?: unknown; header?: { origin?: unknown } } | undefined
        const sessionId = typeof record?.id === 'string' ? record.id : ''
        if (sessionId === '' || record?.header?.origin === 'subagent') return
        if (value !== null && typeof value !== 'string') return
        forward({ type: 'title', sessionId, title: value === '' ? null : value })
      }))
    } catch (error) {
      deps.log.warn('could not subscribe to session title changes; titles follow the reconcile', error)
    }
  } else {
    deps.log.warn('this Harness has no sessionProjections feed; conversation titles refresh with the list', {
      service: 'sessionProjections',
      method: 'onChanged',
    })
  }

  return {
    settled: () => writes,
    close: () => {
      if (closed) return
      closed = true
      while (disposers.length > 0) {
        const dispose = disposers.pop()
        try {
          dispose?.()
        } catch (error) {
          deps.log.warn('could not release a session change subscription', error)
        }
      }
    },
  }
}
