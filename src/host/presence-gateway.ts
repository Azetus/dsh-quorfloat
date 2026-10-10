/**
 * The presence endpoint the browser half calls.
 *
 * The panel's own visibility arrives over the stdio protocol, but the Harness
 * window's visibility exists only in the browser, so the browser half has to
 * report it. Reaching the host from the page means registering a Cordis service
 * the Typert gateway can route to; the mechanics of that registration live in
 * `remote-gateway.ts`, and this file owns only the presence endpoint: its wire
 * fields, their validation, and what the report means.
 */

import type { Logger, PluginContext } from '../cordis-types.js'
import { registerRemoteService, type RemoteMethod } from './remote-gateway.js'

export { REMOTE_METHOD_DESCRIPTOR } from './remote-gateway.js'

/** Cordis service key and wire namespace for this endpoint. */
export const PRESENCE_SERVICE_KEY = 'quorfloat'

/** The one method the browser half calls. */
export const PRESENCE_ENDPOINT_METHOD = 'reportPresence'

/** One validated presence report, as the browser half sends it. */
export interface InboundPresenceReport {
  readonly surface: string
  readonly visible: boolean
  readonly focused: boolean
  readonly seq: number
  readonly at?: number
}

/** What the host does with a report; supplied by the plugin. */
export type PresenceSink = (report: InboundPresenceReport) => { accepted: boolean; reason?: string }

export interface PresenceGatewayDeps {
  readonly ctx: PluginContext
  readonly sink: PresenceSink
  readonly log: Logger
}

/**
 * Validate one inbound report.
 *
 * Both booleans are required together: `visible` alone cannot tell a Windows
 * occluded window from one the user is looking at, and `focused` alone is true
 * for a window on another display.
 *
 * @param payload - the raw argument object from the gateway.
 * @returns the validated report.
 * @throws {TypeError} when a required field is missing or mistyped, which the
 *   gateway surfaces to the caller as a failed invocation.
 */
export function validatePresenceReport(payload: unknown): InboundPresenceReport {
  if (typeof payload !== 'object' || payload === null) {
    throw new TypeError('ui/reportPresence: params must be an object')
  }
  const record = payload as Record<string, unknown>
  const surface = record['surface']
  if (surface !== 'desktop' && surface !== 'web') {
    throw new TypeError('ui/reportPresence: surface must be "desktop" or "web"')
  }
  if (typeof record['visible'] !== 'boolean') {
    throw new TypeError('ui/reportPresence: visible must be a boolean')
  }
  if (typeof record['focused'] !== 'boolean') {
    throw new TypeError('ui/reportPresence: focused must be a boolean')
  }
  if (!Number.isInteger(record['seq'])) {
    throw new TypeError('ui/reportPresence: seq must be an integer')
  }
  const at = record['at']
  if (at !== undefined && !Number.isFinite(at)) {
    throw new TypeError('ui/reportPresence: at must be a finite number when present')
  }
  return {
    surface,
    visible: record['visible'],
    focused: record['focused'],
    seq: record['seq'] as number,
    ...(at === undefined ? {} : { at: at as number }),
  }
}

/**
 * Register the presence endpoint as a Cordis service.
 *
 * Called before the quorfloat process starts: the page reports its presence as
 * soon as it loads, which can precede any stdio handshake, and a report that
 * arrives before anyone listens would be lost — leaving the panel to claim an
 * approval the user is looking at.
 *
 * @param deps - context, sink, and logger.
 * @returns a disposer that unregisters the service.
 */
export function registerPresenceGateway(deps: PresenceGatewayDeps): () => void {
  const { ctx, sink, log } = deps

  // One parameter per wire field, deliberately. The gateway's source-mode
  // descriptor reads parameter names off this signature and then requires the
  // request's `args` keys to match them exactly (`assertExactArguments`), so a
  // single `payload` parameter would make the browser send
  // `args: { payload: {...} }` — and sending the report's own fields, which is
  // what a reader expects, is rejected with `gateway/arguments-invalid`.
  const reportPresence = function reportPresence(
    this: unknown,
    surface: unknown,
    visible: unknown,
    focused: unknown,
    seq: unknown,
    at?: unknown,
  ): { accepted: boolean; reason?: string } {
    return sink(validatePresenceReport({ surface, visible, focused, seq, ...(at === undefined ? {} : { at }) }))
  }
  const methods: readonly RemoteMethod[] = [{ method: PRESENCE_ENDPOINT_METHOD, implementation: reportPresence }]

  const dispose = registerRemoteService({
    ctx,
    serviceKey: PRESENCE_SERVICE_KEY,
    namespace: PRESENCE_SERVICE_KEY,
    methods,
  })
  log.info('presence gateway registered', { namespace: PRESENCE_SERVICE_KEY, method: PRESENCE_ENDPOINT_METHOD })
  return dispose
}
