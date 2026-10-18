/**
 * Browser-facing diagnostics endpoint.
 *
 * The settings page cannot read the host's filesystem or its in-memory state, so
 * one zero-argument Remote method asks the host to build the current snapshot
 * plus the bounded recent supervisor/stderr history and write it next to the
 * plugin's state files. The answer is the absolute path the page shows to the
 * user; no conversation content or credentials cross the endpoint.
 */

import type { Logger, PluginContext } from '../cordis-types.js'
import { registerRemoteService, type RemoteMethod } from './remote-gateway.js'
import type { SavedDiagnostics } from './diagnostics.js'

/** Cordis service key; unique among the plugin's services. */
export const DIAGNOSTICS_SERVICE_KEY = 'quorfloatDiagnostics'

/** Wire namespace shared with the status/panel endpoints. */
export const DIAGNOSTICS_NAMESPACE = 'quorfloat'

/** Wire method that writes and answers one diagnostics file. */
export const DIAGNOSTICS_ENDPOINT_METHOD = 'diagnostics'

/** What the host does when the page asks for an export. */
export interface DiagnosticsGatewayDeps {
  readonly ctx: PluginContext
  readonly run: () => SavedDiagnostics
  readonly log: Logger
}

/**
 * Register the diagnostics endpoint.
 *
 * `run` is synchronous on purpose: it formats and writes one small text file in
 * the state directory. Keeping the method zero-argument makes its gateway
 * argument expectations an empty `args: {}`, matching the page's call.
 *
 * @param deps - context, export callback and logger.
 * @returns a disposer for the service registration.
 */
export function registerDiagnosticsGateway(deps: DiagnosticsGatewayDeps): () => void {
  const diagnostics = function diagnostics(this: unknown): SavedDiagnostics {
    try {
      const saved = deps.run()
      deps.log.info('diagnostics exported', { path: saved.path })
      return saved
    } catch (error) {
      deps.log.warn('diagnostics export failed', error)
      throw error
    }
  }
  const methods: readonly RemoteMethod[] = [
    { method: DIAGNOSTICS_ENDPOINT_METHOD, implementation: diagnostics },
  ]
  const dispose = registerRemoteService({
    ctx: deps.ctx,
    serviceKey: DIAGNOSTICS_SERVICE_KEY,
    namespace: DIAGNOSTICS_NAMESPACE,
    methods,
  })
  deps.log.info('diagnostics gateway registered', {
    namespace: DIAGNOSTICS_NAMESPACE,
    methods: [DIAGNOSTICS_ENDPOINT_METHOD],
  })
  return dispose
}
