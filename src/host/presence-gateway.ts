/**
 * The host service the browser half calls.
 *
 * The panel's own visibility arrives over the stdio protocol, but the Harness
 * window's visibility exists only in the browser, so the browser half has to
 * report it. Reaching the host from the page means going through the Typert
 * gateway, and the gateway only routes to a **registered service** that declares
 * a visible `typertRemote` binding plus a marker for each callable method.
 *
 * This file builds both of those by hand, on purpose. Importing
 * `@deepseek-ai/dsh-typert-protocol` and extending `TypertRemoteService` would
 * pull the first runtime dependency into a plugin that has deliberately had none
 * (see the README: a `@deepseek-ai/dsh*` peer range is what triggers the
 * runtime's compatibility gate and its exact-version exemption flow). Everything
 * needed is either a plain string key or a globally shared symbol:
 *
 * - the marker key is the literal string
 *   `@deepseek-ai/dsh-typert-protocol/remote-methods` with `version: 1`;
 * - `Service`'s only registration step is `ctx.reflect.provide(name, this)`,
 *   and `reflect` is a documented Cordis service, not an internal;
 * - the binding is validated as a **plain object** (`validateBinding` only checks
 *   `service`/`serviceKey`/`namespace`), and the method's parameters are read by
 *   reflection on the prototype, so the wire codec is `src-json` and no schema
 *   library is involved.
 *
 * The tradeoff is coupling to a small, stable wire detail. It fails loudly and
 * legibly: if the descriptor key or version ever changes, the gateway reports
 * `gateway/invocation-unavailable`, which the browser half logs — rather than
 * silently losing the endpoint.
 */

import type { Logger, PluginContext } from '../cordis-types.js'

/** The marker key the gateway reads off a service prototype. */
export const REMOTE_METHOD_DESCRIPTOR = '@deepseek-ai/dsh-typert-protocol/remote-methods'

/** Cordis service key and wire namespace for this gateway. */
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
 * Register the presence gateway as a Cordis service.
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

  const service: Record<string, unknown> = {}
  Object.defineProperty(service, 'ctx', { value: ctx, enumerable: false, writable: true })
  Object.defineProperty(service, 'name', { value: PRESENCE_SERVICE_KEY, enumerable: false, writable: true })
  Object.defineProperty(service, 'typertRemote', {
    value: Object.freeze({ service, serviceKey: PRESENCE_SERVICE_KEY, namespace: PRESENCE_SERVICE_KEY }),
    enumerable: false,
  })

  const method = function reportPresence(this: unknown, payload: unknown): { accepted: boolean; reason?: string } {
    const report = validatePresenceReport(payload)
    return sink(report)
  }
  // `remoteMethods` reads the descriptor off `Object.getPrototypeOf(service)`
  // and the implementation off the same object, so one plain prototype carries
  // both the marker and the function.
  const prototype: Record<string, unknown> = {}
  Object.defineProperty(prototype, PRESENCE_ENDPOINT_METHOD, {
    value: method,
    enumerable: false,
    writable: false,
  })
  Object.defineProperty(prototype, REMOTE_METHOD_DESCRIPTOR, {
    value: Object.freeze({
      version: 1,
      methods: Object.freeze([
        Object.freeze({ method: PRESENCE_ENDPOINT_METHOD, invocation: Object.freeze({ kind: 'direct' }) }),
      ]),
    }),
    enumerable: false,
  })
  Object.setPrototypeOf(service, prototype)

  const dispose = ctx.reflect.provide(PRESENCE_SERVICE_KEY, service)
  log.info('presence gateway registered', { namespace: PRESENCE_SERVICE_KEY, method: PRESENCE_ENDPOINT_METHOD })
  return dispose
}
