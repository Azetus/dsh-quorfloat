/**
 * Registration for a hand-built Cordis service that the Typert gateway routes to.
 *
 * The browser half reaches the host through the Typert gateway, and the gateway
 * only routes to a **registered service** that declares a visible `typertRemote`
 * binding plus a marker for each callable method.
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
 *
 * The Cordis service key and the wire namespace are separate on purpose. The
 * gateway addresses an endpoint as `<namespace>/<method>` and finds it by
 * scanning every registered service whose `typertRemote.namespace` matches
 * (`resolveSrcDescriptor`), so two services with different keys may serve one
 * namespace as long as their method names do not collide — only a repeated
 * endpoint is ambiguous, not a repeated namespace. That is what lets each host
 * file own its own endpoints without a second registration of the same key.
 */

import type { PluginContext } from '../cordis-types.js'

/** The marker key the gateway reads off a service prototype. */
export const REMOTE_METHOD_DESCRIPTOR = '@deepseek-ai/dsh-typert-protocol/remote-methods'

/** One callable method on a Remote service. */
export interface RemoteMethod {
  /** Wire method name; also the prototype property the gateway invokes. */
  readonly method: string
  /**
   * Implementation, whose own parameter list defines the wire fields.
   *
   * Typed as the "any function" shape on purpose: the gateway reads the field
   * names by stringifying this function (`methodParameterNames`), so a narrower
   * signature here would only fight the reflection that makes it callable.
   */
  readonly implementation: (...args: never[]) => unknown
}

/** What one service registration needs. */
export interface RemoteServiceDeps {
  readonly ctx: PluginContext
  /** Cordis service key. Must be unique in the composition. */
  readonly serviceKey: string
  /** Wire namespace; an endpoint is `<namespace>/<method>`. */
  readonly namespace: string
  /** Every method the namespace serves from this service. */
  readonly methods: readonly RemoteMethod[]
}

/**
 * Register a service exposing `methods` under `namespace`.
 *
 * @param deps - context, service key, namespace, and methods.
 * @returns a disposer that unregisters the service.
 */
export function registerRemoteService(deps: RemoteServiceDeps): () => void {
  const { ctx, serviceKey, namespace, methods } = deps

  const service: Record<string, unknown> = {}
  Object.defineProperty(service, 'ctx', { value: ctx, enumerable: false, writable: true })
  Object.defineProperty(service, 'name', { value: serviceKey, enumerable: false, writable: true })
  Object.defineProperty(service, 'typertRemote', {
    value: Object.freeze({ service, serviceKey, namespace }),
    enumerable: false,
  })

  // One parameter per wire field, deliberately, and the same reason the marker
  // lists the methods rather than a name table: the gateway's source-mode
  // descriptor reads parameter names off each signature and then requires the
  // request's `args` keys to match them exactly (`assertExactArguments`), so a
  // method declared as `method(payload)` expects `args: { payload: {...} }` —
  // while every reader (and the browser half) naturally sends the method's own
  // fields.
  const prototype: Record<string, unknown> = {}
  for (const { method, implementation } of methods) {
    Object.defineProperty(prototype, method, { value: implementation, enumerable: false, writable: false })
  }
  // `remoteMethods` reads the descriptor off `Object.getPrototypeOf(service)`
  // and the implementation off the same object, so one plain prototype carries
  // both the marker and the functions.
  Object.defineProperty(prototype, REMOTE_METHOD_DESCRIPTOR, {
    value: Object.freeze({
      version: 1,
      methods: Object.freeze(
        methods.map(({ method }) => Object.freeze({ method, invocation: Object.freeze({ kind: 'direct' }) })),
      ),
    }),
    enumerable: false,
  })
  Object.setPrototypeOf(service, prototype)

  return ctx.reflect.provide(serviceKey, service)
}
