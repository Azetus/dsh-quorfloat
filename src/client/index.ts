/**
 * Browser half: report whether the user is looking at the Harness window.
 *
 * Why this exists at all: the approval authority needs to know which surface the
 * user is looking at, and that fact lives only here. The Electron shell knows it
 * (`window.isVisible()`) but does not expose it to plugins, and the host process
 * has no window concept. Measured behaviour that shaped this file
 * (`docs/prototype.md` §18):
 *
 * - `document.visibilityState` alone is not the answer. A window fully occluded
 *   by another application reports `hidden` on macOS but `visible` on Windows,
 *   and a window on another desktop reports `hidden` on macOS and `visible` on
 *   Windows. Only `visible && document.hasFocus()` means "the user is looking at
 *   it", and that pair gives the same answer on both platforms.
 * - `visibilitychange` still fires while the page is hidden, so the host learns
 *   about the change immediately instead of waiting for a report to go stale.
 *
 * Nothing is rendered here, and nothing is stored: this half only forwards two
 * booleans.
 */

/** The slice of the browser connection service this half needs. */
interface ClientConnection {
  readonly rpc: {
    call(
      channel: string,
      endpoint: string,
      payload: unknown,
      signal?: AbortSignal,
    ): Promise<{ ok: boolean; value?: unknown; error?: unknown }>
  }
}

/** Client context surface used here. */
interface ClientContext {
  readonly connection: ClientConnection
  /** Register cleanup owned by this plugin's fiber; runs on unload. */
  effect(body: () => () => void, label?: string): () => void
  logger(name?: string): { info(...args: unknown[]): void; warn(...args: unknown[]): void; debug(...args: unknown[]): void }
}

/** Required browser services. */
export const inject = ['connection']

/** Logical channel the gateway serves host Remote namespaces on. */
const CHANNEL = '/api'

/** Endpoint: `<service key>/<method>` of the presence gateway on the host. */
const ENDPOINT = 'quorfloat/reportPresence'

/**
 * Delay before a focus or visibility change is reported.
 *
 * Focus and visibility settle in pairs — switching windows fires `blur` and
 * `visibilitychange` together — so this collapses a burst into one report
 * without making the host wait noticeably. Tuning it toward zero makes the
 * hand-off feel instant at the cost of more requests; toward the high end it
 * adds a perceptible lag before the approval authority moves.
 *
 * Deliberately not configurable yet: the host's config reaches quorfloat over
 * stdio, and this half is not on that channel, so exposing it means either
 * baking a default in at build time or adding a settings fetch to the gateway.
 * Both are more moving parts than one timing constant justifies; see the config
 * candidates in `docs/prototype.md` §21.
 */
const HINT_DEBOUNCE_MS = 50

/** Which surface this page is. The desktop shell and a plain browser share code. */
function surfaceOf(): 'desktop' | 'web' {
  return typeof (globalThis as { dshDesktop?: unknown }).dshDesktop === 'undefined' ? 'web' : 'desktop'
}

/**
 * Install the presence reporter.
 *
 * @param ctx - client root context.
 */
export function apply(ctx: ClientContext): void {
  const surface = surfaceOf()
  let seq = 0
  let pending: ReturnType<typeof setTimeout> | undefined
  let sending = false
  let queued = false

  const read = (): { visible: boolean; focused: boolean } => ({
    // `hidden` is meaningful on its own: it covers macOS occlusion and
    // other-desktop. `focused` is what makes Windows agree.
    visible: document.visibilityState === 'visible',
    focused: document.hasFocus(),
  })

  const send = async (): Promise<void> => {
    if (sending) {
      // Coalesce: focus and visibility often change together, and the host only
      // needs the latest pair.
      queued = true
      return
    }
    sending = true
    try {
      const { visible, focused } = read()
      seq += 1
      // The arguments are spread: the gateway matches `args` keys against the
      // host method's parameter names one for one.
      const result = await ctx.connection.rpc.call(CHANNEL, ENDPOINT, {
        args: { surface, visible, focused, seq, at: Date.now() },
      })
      if (result.ok !== true) {
        ctx.logger('quorfloat/presence').debug('presence report was refused', { seq, error: result.error })
      }
    } catch (error) {
      // Losing a report is recoverable: the next focus or visibility change sends
      // another one, and the host treats a missing report as "not looking" rather
      // than as "looking". Never let this throw into the client event chain.
      ctx.logger('quorfloat/presence').warn('could not report presence', error)
    } finally {
      sending = false
      if (queued) {
        queued = false
        void send()
      }
    }
  }

  const schedule = (): void => {
    if (pending !== undefined) clearTimeout(pending)
    pending = setTimeout(() => {
      pending = undefined
      void send()
    }, HINT_DEBOUNCE_MS)
  }

  document.addEventListener('visibilitychange', schedule)
  window.addEventListener('focus', schedule)
  window.addEventListener('blur', schedule)

  ctx.effect(() => () => {
    document.removeEventListener('visibilitychange', schedule)
    window.removeEventListener('focus', schedule)
    window.removeEventListener('blur', schedule)
    if (pending !== undefined) clearTimeout(pending)
  }, 'quorfloat/presence: listeners')

  // Report the opening state too: without it the host would treat a freshly
  // loaded, focused window as unreported until the user first touches it.
  void send()
}
