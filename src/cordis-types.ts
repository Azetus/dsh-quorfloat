/**
 * Structural types for the subset of the Cordis 4 plugin API this plugin uses.
 *
 * The plugin deliberately ships **zero runtime dependencies** and imports
 * nothing from `@deepseek-ai/**`: the host supplies the `Context` instance, so
 * a structural view is enough to type the code, and the plugin never has to be
 * resolved against the runtime's peer-dependency compatibility gate.
 *
 * If this file ever drifts from the real runtime, `tests/cordis-smoke.test.mjs`
 * fails: it loads the genuine `@deepseek-ai/cordis` Context and drives this
 * plugin through it.
 */

/** A reverse-order cleanup callback; async cleanup is awaited by the runtime. */
export type Disposer = () => void | Promise<void>

/** Result accepted from an effect body: a disposer, a promise, or an iterable of them. */
export type EffectResult =
  | Disposer
  | Promise<Disposer>
  | Iterable<Disposer>
  | AsyncIterable<Disposer>
  | void

/** Log methods this plugin calls. Messages are formatted by the host. */
export interface Logger {
  error(...args: unknown[]): void
  warn(...args: unknown[]): void
  info(...args: unknown[]): void
  debug(...args: unknown[]): void
}

/**
 * The host context surface this plugin relies on.
 *
 * `effect` registers cleanup that runs on plugin unload; `inject` defers a
 * callback until the named services are available; `on` subscribes to host
 * events. Everything else is optional so the type also fits the small fake
 * contexts used by focused unit tests.
 */
export interface PluginContext {
  /** Register a cleanup effect owned by this plugin's fiber. */
  effect(body: () => EffectResult, label?: string): Disposer
  /** Run `callback` once every named service is available; returns a disposer. */
  inject(services: string[], callback: (ctx: PluginContext) => EffectResult): Disposer
  /** Read one optional service without declaring a hard dependency. */
  get(name: string): unknown
  /**
   * Subscribe to one host event (used for approval / user-question waterfalls).
   *
   * `options.prepend` runs this listener before the ones already registered for
   * the same event, which is the supported way to be reached ahead of a listener
   * that never delegates (`dsh-api-remotes` forwards to the browser and does not
   * call `next()`); `options.global` bypasses context-filter checks.
   */
  on(
    name: string,
    listener: (...args: any[]) => unknown,
    options?: { prepend?: boolean; global?: boolean },
  ): Disposer
  /** Emit one host event. */
  emit(name: string, ...args: unknown[]): void
  /** Create a named child logger. */
  logger(name?: string): Logger
  /**
   * The reflection layer backing the context proxy.
   *
   * `provide` is how a service becomes visible to the rest of the composition —
   * it is the only registration step Cordis' own `Service` base performs, and
   * the Typert gateway discovers remotely-callable services through it.
   */
  reflect: {
    /** Register `value` under `name` for this fiber; returns a disposer. */
    provide(name: string, value?: unknown, check?: () => boolean): Disposer
  }
}

/** A schema accepted as plugin `Config` (Standard Schema v1, synchronous only). */
export interface StandardSchemaIssue {
  readonly message: string
  readonly path?: readonly (PropertyKey | { readonly key: PropertyKey })[] | undefined
}

export interface StandardSchemaV1<Input = unknown, Output = Input> {
  readonly '~standard': {
    readonly version: 1
    readonly vendor: string
    readonly validate: (
      value: unknown,
    ) => { readonly value: Output } | { readonly issues: readonly StandardSchemaIssue[] }
  }
}
