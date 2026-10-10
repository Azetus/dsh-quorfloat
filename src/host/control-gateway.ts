/**
 * The two control endpoints the browser half calls: read supervision state, and
 * drive the process that hosts the panel.
 *
 * The panel's browser half has no direct view of the sidecar — it lives in the
 * page, the sidecar is a child process of the host — so both directions go
 * through a Typert Remote endpoint. `status` is the read side, `panel` the write
 * side, and both answer the same shape so a caller never has to merge two
 * vocabularies:
 *
 * - `state` is the supervisor's own `SupervisorState` verbatim. A parallel
 *   "display state" invented here would be a second place to keep in step with
 *   the process it describes.
 * - `pid` is `null` rather than `undefined` when nothing is running: JSON has no
 *   `undefined`, and a caller reading `status.pid` should not have to know which
 *   absence it got.
 * - `seen` is the supervisor's `handshaken`: the one fact it tracks for "this
 *   process has identified itself". `visible` is the panel's own last reported
 *   window visibility.
 * - `lastError` is reduced to the message, because the caller renders a sentence,
 *   not a code.
 *
 * The service key differs from the presence endpoint's because a Cordis service
 * key is unique; both serve the `quorfloat` wire namespace (see
 * `remote-gateway.ts` for why that is legal and intended).
 */

import type { Logger, PluginContext } from '../cordis-types.js'
import type { SupervisorSnapshot } from './supervisor.js'
import { registerRemoteService, type RemoteMethod } from './remote-gateway.js'

/** Cordis service key for the control endpoints. */
export const CONTROL_SERVICE_KEY = 'quorfloatControl'

/** Wire namespace shared with the presence endpoint: `quorfloat/<method>`. */
export const CONTROL_NAMESPACE = 'quorfloat'

/** Wire method that answers the current supervision snapshot. */
export const STATUS_ENDPOINT_METHOD = 'status'

/** Wire method that performs one lifecycle action. */
export const PANEL_ENDPOINT_METHOD = 'panel'

/** What a caller may ask the panel's process to do. */
export type PanelAction = 'start' | 'stop' | 'restart'

/** The status shape both endpoints answer with. */
export interface QuorfloatStatus {
  /** The supervisor's own state vocabulary, passed through unchanged. */
  readonly state: SupervisorSnapshot['state']
  /** Sidecar process id, or `null` when it is not running. */
  readonly pid: number | null
  readonly restarts: number
  readonly restartExhausted: boolean
  /** The sidecar completed its protocol handshake. */
  readonly seen: boolean
  /** The panel last reported its window visible. */
  readonly visible: boolean
  /** Message of the most recent recorded failure, or `null`. */
  readonly lastError: string | null
}

/**
 * The supervisor-shaped host the control endpoints act on.
 *
 * Deliberately structural rather than the class itself: it is exactly the four
 * members used here, so a test can hand over the real supervisor or a stub
 * without either side pretending to be the other.
 */
export interface ControlHost {
  /** Current supervision snapshot. */
  snapshot(): SupervisorSnapshot
  /** Start the process if it is not live; resolves once the attempt settled. */
  start(): Promise<unknown>
  /** Stop the process deliberately; resolves once it exited. */
  stop(): Promise<unknown>
  /** Stop and start again; resolves once the replacement settled. */
  restart(): Promise<unknown>
}

/**
 * Reduce one supervision snapshot to the wire shape.
 *
 * Pure so the mapping can be asserted directly against real supervisor
 * snapshots, including the two that are easiest to get subtly wrong: a stopped
 * supervisor has no `pid`, and `seen` follows the handshake rather than the
 * process existing.
 *
 * @param snapshot - a supervisor snapshot.
 * @returns the status payload.
 */
export function buildStatus(snapshot: SupervisorSnapshot): QuorfloatStatus {
  return {
    state: snapshot.state,
    pid: snapshot.pid ?? null,
    restarts: snapshot.restarts,
    restartExhausted: snapshot.restartExhausted,
    seen: snapshot.handshaken,
    visible: snapshot.panelVisible,
    lastError: snapshot.lastError?.message ?? null,
  }
}

/**
 * Validate one inbound panel action.
 *
 * A closed set, and the same shape of refusal as the presence endpoint: the
 * effect is a process-level one no caller can take back, so an unknown word must
 * stop here rather than reach the supervisor.
 *
 * @param action - the raw `action` argument from the gateway.
 * @returns the validated action.
 * @throws {TypeError} when the action is missing or unknown.
 */
export function validatePanelAction(action: unknown): PanelAction {
  if (action !== 'start' && action !== 'stop' && action !== 'restart') {
    throw new TypeError(`quorfloat/panel: action must be start, stop or restart (got ${JSON.stringify(action)})`)
  }
  return action
}

/**
 * Perform one validated lifecycle action through the host.
 *
 * The single place the action word is turned into a supervisor call, shared by
 * this endpoint and by the stdio `panel/lifecycle` method so the two can never
 * disagree about what `stop` means — only `stop()` marks the exit deliberate,
 * and a stop the restart policy read as a crash would bring the panel back.
 *
 * @param host - the supervisor-shaped host.
 * @param action - the validated action.
 */
export async function applyLifecycle(host: ControlHost, action: PanelAction): Promise<void> {
  if (action === 'stop') await host.stop()
  else if (action === 'restart') await host.restart()
  else await host.start()
}

export interface ControlGatewayDeps {
  readonly ctx: PluginContext
  readonly host: ControlHost
  readonly log: Logger
}

/**
 * Register the control endpoints as a Cordis service.
 *
 * Registered after the supervisor exists (unlike the presence endpoint, which
 * the page may reach before the first spawn): both methods read or drive that
 * object, so there is nothing honest to answer before it is constructed.
 *
 * @param deps - context, host, and logger.
 * @returns a disposer that unregisters the service.
 */
export function registerControlGateway(deps: ControlGatewayDeps): () => void {
  const { ctx, host, log } = deps

  // Zero parameters on purpose: `status` takes none, and a method declared with
  // a parameter would require the caller to send that field (see
  // `assertExactArguments`). The browser half must therefore send `args: {}`,
  // not omit `args` — the gateway rejects a missing `args` before this runs.
  const status = function status(this: unknown): QuorfloatStatus {
    return buildStatus(host.snapshot())
  }
  // The action arrives as the bare string because the gateway spreads the
  // request's `args` across the signature one field per parameter.
  const panel = async function panel(this: unknown, action: unknown): Promise<QuorfloatStatus> {
    await applyLifecycle(host, validatePanelAction(action))
    return buildStatus(host.snapshot())
  }
  const methods: readonly RemoteMethod[] = [
    { method: STATUS_ENDPOINT_METHOD, implementation: status },
    { method: PANEL_ENDPOINT_METHOD, implementation: panel },
  ]

  const dispose = registerRemoteService({
    ctx,
    serviceKey: CONTROL_SERVICE_KEY,
    namespace: CONTROL_NAMESPACE,
    methods,
  })
  log.info('control gateway registered', {
    namespace: CONTROL_NAMESPACE,
    methods: [STATUS_ENDPOINT_METHOD, PANEL_ENDPOINT_METHOD],
  })
  return dispose
}
