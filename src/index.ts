/**
 * dsh-quorfloat host plugin entry.
 *
 * Activation order, and why it is this order:
 * 1. validate configuration (the runtime has already applied the `Config`
 *    schema, so this is a re-read of the normalized value);
 * 2. wait until the Harness services we need are actually available, without
 *    blocking the host event loop — a plugin that throws during activation
 *    would take down a shared composition, so a missing service is reported,
 *    not fatal;
 * 3. build the adapter, the session layer, and the interaction answerers;
 * 4. start the quorfloat process and let the supervisor own it from there.
 *
 * Deactivation reverses it: stop accepting work, withdraw pending interactions,
 * release subscriptions, then stop the process and confirm it exited.
 */

import { randomUUID } from 'node:crypto'

import { Config, DEFAULT_CONFIG, type QuorfloatConfig } from './config.js'
import type { ChannelError } from './bridge/errors.js'
import { HostRouter, type PanelLifecycleAction } from './bridge/router.js'
import { QuorfloatSupervisor, type SupervisorEvent, type SupervisorSnapshot } from './host/supervisor.js'
import { resolveQuorfloatBinary, type ResolvedBinary } from './host/binary.js'
import { readHarnessLocalePreference, resolvePanelLanguage } from './host/language.js'
import { registerPresenceGateway, type InboundPresenceReport } from './host/presence-gateway.js'
import { applyLifecycle, registerControlGateway } from './host/control-gateway.js'
import {
  createHarnessFromContext,
  HarnessError,
  probeServices,
  type ModelChoiceView,
  type ProbeResult,
  type QuorfloatHarness,
} from './harness/adapter.js'
import { SessionLayer } from './harness/session-layer.js'
import { evaluateAuthority, PresenceTracker, type AuthorityVerdict, type PresenceSurface } from './harness/presence.js'
import { Interactions } from './harness/interactions.js'
import type { Disposer, EffectResult, Logger, PluginContext } from './cordis-types.js'

export { Config } from './config.js'
export type { QuorfloatConfig } from './config.js'

/**
 * Services that must be provided before the plugin does any work.
 *
 * Verified against a real dsh: `sessionController` is **not** available when
 * `apply` runs — the profile composes in availability order, so the controller
 * appears later. Declaring it here is what makes Cordis activate the work at the
 * right moment; probing once at `apply` time always reports it missing.
 */
const REQUIRED_SERVICES: string[] = ['sessionController']

/**
 * The declared dependency is deliberately narrow: only the service without which
 * no session call can succeed. Everything else (workspace registry, approval,
 * user questions) is used when present and reported when absent, so a profile
 * that composes fewer services still gets a diagnosable plugin rather than one
 * stuck in `PENDING` with no explanation.
 */
export const inject: string[] = REQUIRED_SERVICES

/** One entry in the plugin's activation record, exposed for diagnostics. */
interface Activation {
  readonly startedAt: number
  supervisor?: QuorfloatSupervisor | undefined
  sessionLayer?: SessionLayer | undefined
  interactions?: Interactions | undefined
  probe?: ProbeResult | undefined
  harness?: QuorfloatHarness | undefined
  /** Disposer for the deferred activation registered with `ctx.inject`. */
  injection?: Disposer | undefined
  /** Reported presence of the Harness UI surfaces; read on every decision. */
  presence?: PresenceTracker | undefined
  /** Disposer for the browser-facing presence service. */
  presenceGateway?: Disposer | undefined
  /** Disposer for the browser-facing control service. */
  controlGateway?: Disposer | undefined
  stopped: boolean
}

/**
 * Build the plugin function for one configuration.
 *
 * Exported separately from the default export so tests can construct the plugin
 * with injected seams (logger, binary resolver) instead of a live runtime.
 *
 * @param overrides - test seams; omitted in production.
 * @returns a Cordis function plugin.
 */
export function createPlugin(overrides: PluginOverrides = {}) {
  return function apply(ctx: PluginContext, rawConfig: unknown): EffectResult {
    const config = normalizeOrThrow(rawConfig)
    const logger = overrides.logger ?? ctx.logger('quorfloat')
    const log = wrapLogger(logger, config.logLevel)
    const activation: Activation = { startedAt: Date.now(), stopped: false }

    log.info('dsh-quorfloat activating', {
      enabled: config.enabled,
      hotkey: config.hotkey === '' ? '(none)' : config.hotkey,
      heartbeatMs: config.heartbeatMs,
    })

    // The activation record is a disposal effect first: no matter how far
    // activation gets, deactivation has one place that reverses all of it.
    ctx.effect(() => {
      return async () => {
        await deactivate(activation, log)
      }
    }, 'quorfloat: activation record')

    // The plugin's own view of itself. It is what the (future) settings surface
    // and the diagnostics method report, and what tests assert against — reading
    // state is strictly better than parsing log lines.
    const state = (): PluginState => ({
      enabled: config.enabled,
      // `null` until the first probe: reporting an all-false presence before
      // probing would claim "no services" and "not checked yet" as the same fact.
      services: activation.probe?.presence ?? null,
      missingServices: activation.probe?.missing ?? [],
      supervisor: activation.supervisor?.snapshot() ?? null,
      sessions: activation.sessionLayer?.describe() ?? null,
      interactions: activation.interactions?.describe() ?? null,
      stopped: activation.stopped,
    })

    // Service readiness is polled rather than awaited inline: `apply` must
    // return promptly so the host event loop is never blocked by a plugin.
    const probeNow = (): boolean => {
      const probe = probeServices(name => ctx.get(name))
      activation.probe = probe
      if (probe.missing.length > 0) return false
      const created = createHarnessFromContext(name => ctx.get(name), log)
      if (created.harness === undefined) return false
      activation.harness = created.harness
      return true
    }

    const beginWork = (): void => {
      if (activation.stopped) return
      const harness = activation.harness
      if (harness === undefined) return
      log.info('harness services are ready', harness.describe())

      // Presence is read on every interaction decision, and the panel's own
      // visibility comes from the supervision snapshot, so neither needs its own
      // subscription: both are sampled at the moment a request arrives.
      const presence = new PresenceTracker()
      activation.presence = presence
      const acceptPresence = (report: InboundPresenceReport): { accepted: boolean; reason?: string } => {
        const surface: PresenceSurface = report.surface === 'desktop' ? 'desktop' : 'web'
        const rejection = presence.report(surface, {
          visible: report.visible,
          focused: report.focused,
          seq: report.seq,
          at: report.at ?? Date.now(),
        })
        if (rejection !== undefined) {
          log.debug('ignored a presence report', { surface, rejection, seq: report.seq })
          return { accepted: false, reason: rejection }
        }
        log.debug('presence reported', { surface, visible: report.visible, focused: report.focused })
        overrides.onStateChange?.(state())
        return { accepted: true }
      }
      // Registered before the process starts: the page reports on load, which can
      // precede any stdio handshake, and a report nobody listens for is lost.
      try {
        activation.presenceGateway = registerPresenceGateway({ ctx, sink: acceptPresence, log })
      } catch (error) {
        // Without this the panel cannot know whether the user is looking at the
        // Harness window, so approvals would default to whichever surface the
        // authority rules pick for an unreported state. Say so rather than fail
        // the whole plugin.
        log.warn('could not register the presence gateway; browser halves cannot report visibility', error)
      }
      const authority = (): AuthorityVerdict =>
        evaluateAuthority({
          desktop: presence.raw('desktop'),
          web: presence.raw('web'),
          panelVisible: activation.supervisor?.snapshot().panelVisible === true,
          maxAgeMs: config.presenceMaxAgeMs,
          now: Date.now(),
        })

      const sessionLayer = new SessionLayer({
        harness,
        defaultWorkspaceId: () => config.defaultWorkspaceId,
        notify: async (method, params) => {
          const supervisor = activation.supervisor
          if (supervisor === undefined) return
          // Notifications are best-effort: a lost channel is reported by the
          // supervisor, and a dropped display frame must not fail a session call.
          await supervisor.notify(method, params)
        },
        onStateChange: (sessionId, reason, detail) => {
          log.debug('session state change', { sessionId, reason, detail })
        },
        log,
      })
      activation.sessionLayer = sessionLayer

      const interactions = new Interactions({
        ctx,
        ownedSessionIds: () => sessionLayer.ownedSessionIds(),
        authority,
        // Which conversation the panel is showing. Together with the verdict this is what
        // makes the panel answer only for its own conversation: an open panel that is
        // working on something else defers, and the Harness window answers.
        panelSession: () => sessionLayer.activeSessionId,
        // The peer declares what its build implements in `hello`, and claiming is
        // exclusive — so the panel claims only what it says it can render. Today that
        // is approvals: a question's answer is a list of selected option ids and the
        // panel has no widget for it, so claiming one would hide it from the Harness
        // window for the whole claim deadline and then hand it back.
        canAnswer: kind => activation.supervisor?.snapshot().peerCapabilities.includes(kind) === true,
        notify: async (method, params) => {
          const supervisor = activation.supervisor
          if (supervisor === undefined) return
          await supervisor.notify(method, params)
        },
        onStateChange: (sessionId, reason, detail) => {
          log.debug('interaction state change', { sessionId, reason, detail })
        },
        log,
      }, { deadlineMs: config.claimDeadlineMs })
      activation.interactions = interactions
      if (!interactions.register()) {
        // Not fatal: the text path stays usable, and the status surface reports
        // that approvals cannot be answered from the panel on this build.
        log.warn('approval/question answering from the panel is unavailable on this Harness build')
      }

      // The panel's language, resolved once, on the transition into work: it has to
      // be in the spawn environment for the very first frame, before any host frame
      // has been processed. An explicit panel setting is used as it stands; an empty
      // one is seeded from the Harness's own locale preference (namespace `locale`,
      // field `preference`, in the Host user-settings document), and anything
      // unsupported, unreadable or absent becomes `en`. The sidecar persists the
      // answer as the panel's own setting, which is what makes later launches
      // explicit instead of a second reading of the Harness.
      const harnessPreference = readHarnessLocalePreference(name => ctx.get(name))
      const language = resolvePanelLanguage(config.window.language, harnessPreference)
      log.info('panel language resolved', {
        language,
        setting: config.window.language === '' ? '(unset)' : config.window.language,
        harness: harnessPreference ?? '(unreachable)',
      })
      // Only the supervisor ever reads `window.language`, and it reads it through
      // `config()`, so the resolved value is carried on a copy rather than mutating
      // the validated configuration object the rest of activation closed over.
      const effectiveConfig: QuorfloatConfig =
        language === config.window.language ? config : { ...config, window: { ...config.window, language } }

      const supervisor = new QuorfloatSupervisor({
        config: () => effectiveConfig,
        resolveBinary: () =>
          overrides.resolveBinary?.(config.quorfloatPath) ?? resolveQuorfloatBinary({ configuredPath: config.quorfloatPath }),
        createRouter: channelSessionId => {
          const router = new HostRouter({
            config: () => config,
            hostVersion: () => hostVersion(),
            channelSessionId: () => channelSessionId,
            // The peer-facing method names belong to the router; everything that
            // touches Harness state goes through the layers built above.
            listWorkspaces: async () => sessionLayer.listWorkspaces(),
            listSessions: async () => ({ items: await sessionLayer.listSessions() }),
            createSession: async workspaceId => await sessionLayer.createSession(workspaceId),
            attachSession: async sessionId => await sessionLayer.attach(sessionId),
            readHistory: async (sessionId, beforeSeq) => await sessionLayer.readHistory(sessionId, beforeSeq),
            readOptions: async sessionId => await sessionLayer.readOptions(sessionId),
            selectModel: async (sessionId, selection) =>
              await sessionLayer.selectModel(sessionId, selection as ModelChoiceView),
            setPermission: async (sessionId, value) => await sessionLayer.setPermission(sessionId, value),
            prompt: async (sessionId, requestId, text) => await sessionLayer.prompt(sessionId, requestId, text),
            cancel: async sessionId => await sessionLayer.cancel(sessionId),
            answerInteraction: async (interactionId, answer) => interactions.answer(interactionId, answer),
            // The stdio path exists for tests and for a peer that reports the
            // panel's own presence; the browser half goes through the gateway
            // service instead. Both funnel into the same tracker.
            reportPresence: async (surface, report) => acceptPresence({ ...report, surface }),
            // The panel's tray menu can replace or deliberately stop the process
            // that hosts it. `supervisor` is declared below this closure, but the
            // closure only runs after it is assigned: `createRouter` is called
            // from inside `supervisor.start()`, never while the supervisor is
            // still being constructed. `stop` must go through the supervisor's
            // own entry — that is what marks the exit as deliberate, and a stop
            // the policy read as a crash would be undone by an automatic restart.
            // The dispatch itself is shared with the browser-facing `panel`
            // endpoint so the two cannot disagree about what an action means.
            panelLifecycle: async (action: PanelLifecycleAction) => {
              log.info('panel requested a lifecycle action', { action })
              await applyLifecycle(supervisor, action)
              return { action, accepted: true as const }
            },
            diagnostics: () => ({
              hostVersion: hostVersion(),
              services: activation.probe?.presence ?? {},
              sessions: sessionLayer.describe(),
              interactions: interactions.describe(),
              authority: authority(),
            }),
          })
          overrides.onRouter?.(router)
          return router
        },
        log,
        // Supervision narration is deliberately visible at the default level.
        // A plugin whose only symptom is "nothing happens" is undiagnosable: the
        // lifecycle of the process it owns is exactly what an operator needs, and
        // failures are escalated so no log level can hide them.
        onEvent: event => {
          reportSupervisorEvent(event, log)
          overrides.onStateChange?.(state())
        },
        onPeerNotification: (method, params) => {
          // The peer may ask for a resync instead of accepting a gap; the session
          // layer owns the subscription, so the request is routed there.
          if (method === 'session/attach') {
            const sessionId = typeof (params as { sessionId?: unknown })?.sessionId === 'string'
              ? (params as { sessionId: string }).sessionId
              : undefined
            if (sessionId !== undefined) void sessionLayer.attach(sessionId)
          }
          // The panel has stopped showing a conversation — it started a new one, or the
          // pinned one turned out to be gone. This is what keeps approval routing honest:
          // the panel answers only for the conversation it is showing, so a subscription
          // nobody is looking at must not keep claiming requests for it.
          if (method === 'session/detach') {
            const sessionId = typeof (params as { sessionId?: unknown })?.sessionId === 'string'
              ? (params as { sessionId: string }).sessionId
              : undefined
            if (sessionId !== undefined) sessionLayer.detach(sessionId)
          }
        },
      })
      activation.supervisor = supervisor
      try {
        // Registered once the supervisor exists, but before the first spawn: a
        // status read during startup must answer "starting", not "unavailable".
        activation.controlGateway = registerControlGateway({ ctx, host: supervisor, log })
      } catch (error) {
        // The browser half loses its status and control surface, but the plugin
        // still owns the process: report rather than fail activation.
        log.warn('could not register the panel control gateway; the browser half cannot read or change supervision state', error)
      }
      void supervisor
        .start()
        .then(snapshot => {
          log.info('quorfloat supervision started', summarize(snapshot))
        })
        .catch(error => {
          log.error('failed to start quorfloat', error)
        })
    }

    // Readiness uses the framework's own deferred-activation mechanism rather
    // than polling. Two reasons, both learned the hard way on a real dsh:
    //
    // 1. Services appear *after* this plugin's `apply` runs — the profile boots
    //    in availability order, so `sessionController` does not exist yet when
    //    we get here. A single probe would always say "missing".
    // 2. A polling timer had to be `unref`'d to avoid holding the host event
    //    loop open, which also made it possible for the poll to never fire.
    //
    // `ctx.inject` is the supported answer: Cordis activates the callback when
    // every named service becomes available, and tears it down if they go away.
    const servicesReady = probeNow()
    const missing = servicesReady ? [] : (overrides.missingServices?.() ?? activation.probe?.missing ?? [])
    if (missing.length === 0) {
      beginWork()
      overrides.onStateChange?.(state())
      return undefined
    }
    log.warn('quorfloat is loaded but waiting for Harness services', {
      missing,
      hint: 'the window cannot appear until these are provided; a profile without the Web/API bundles provides none of them',
    })
    overrides.onStateChange?.(state())

    const inject = [...(overrides.injectServices ?? REQUIRED_SERVICES)]
    const disposeInjection = ctx.inject(inject, () => {
      if (activation.stopped) return undefined
      if (!probeNow()) {
        // The callback fired but the probe still disagrees: report rather than
        // start a process whose session calls would all fail.
        log.error('Harness services were announced but are not resolvable', {
          missing: activation.probe?.missing ?? [],
        })
        overrides.onStateChange?.(state())
        return undefined
      }
      beginWork()
      overrides.onStateChange?.(state())
      return undefined
    })
    activation.injection = disposeInjection

    return undefined
  }
}

/** The plugin's own view of its activation, for diagnostics and for tests. */
export interface PluginState {
  readonly enabled: boolean
  readonly services: { readonly sessionController: boolean; readonly workspaceRegistry: boolean; readonly approval: boolean; readonly userQuestions: boolean } | null
  readonly missingServices: readonly string[]
  readonly supervisor: SupervisorSnapshot | null
  readonly sessions: Record<string, unknown> | null
  readonly interactions: Record<string, unknown> | null
  readonly stopped: boolean
}

/**
 * Report one supervision event at a level that survives the default config.
 *
 * The rule: state transitions are `info`, anything that lost or risked losing
 * the process is `warn`, and a terminal failure is `error`. Nothing that an
 * operator needs in order to answer "why is there no window?" is left at `debug`.
 *
 * @param event - the supervision event.
 * @param log - the plugin logger.
 */
function reportSupervisorEvent(event: SupervisorEvent, log: Logger): void {
  const snapshot = event.snapshot
  switch (event.kind) {
    case 'handshake': {
      const hotkey = snapshot.hotkey
      log.info('quorfloat handshake complete', {
        pid: snapshot.pid,
        hotkey: hotkey === null ? '(none)' : `${hotkey.requested}${hotkey.registered ? '' : ' (NOT registered)'}`,
        source: snapshot.binarySource,
      })
      return
    }
    case 'state': {
      if (snapshot.state === 'starting') {
        log.info('starting quorfloat', { path: snapshot.binaryPath ?? '(unresolved)', source: snapshot.binarySource ?? '-' })
        return
      }
      if (snapshot.state === 'running') return
      if (snapshot.state === 'failed') {
        log.error('quorfloat is not running and will not be retried automatically', {
          reason: snapshot.lastError?.message ?? 'unknown',
          hint: 'fix the cause, then restart the plugin or use the manual retry entry',
        })
        return
      }
      return
    }
    case 'exit':
      log.warn('quorfloat process exited', event.detail)
      return
    case 'restart-given-up':
      log.error('automatic restart gave up', { reason: snapshot.lastError?.message ?? 'unknown' })
      return
    case 'frame-rejected':
      // A peer that writes non-protocol output is the single most likely reason a
      // connection fails, so it is surfaced rather than counted silently.
      log.warn('quorfloat sent a frame that is not protocol data', event.detail)
      return
    case 'stderr':
      log.debug('quorfloat stderr', event.detail)
      return
  }
}

/** Test seams; production passes nothing. */
export interface PluginOverrides {
  /** Replace the Cordis logger. */
  readonly logger?: Logger
  /** Replace the quorfloat executable resolver. */
  readonly resolveBinary?: (configuredPath: string) => ResolvedBinary
  /** Observe activation state changes; must not throw. */
  readonly onStateChange?: (state: PluginState) => void
  /** Replace the service list the deferred activation waits for (tests only). */
  readonly injectServices?: readonly string[]
  /** Report a fixed set of missing services, bypassing the probe (tests only). */
  readonly missingServices?: () => readonly string[]
  /**
   * Observe each router as it is created (tests only).
   *
   * The router is the plugin's whole inbound protocol surface, so a test that can
   * hold it drives the same entry points a real peer and a real browser half use,
   * instead of reaching into internals.
   */
  readonly onRouter?: (router: HostRouter) => void
}

/**
 * The plugin as dsh loads it.
 *
 * `inject` is attached to the function because Cordis reads plugin metadata from
 * the plugin value itself, and it is deliberately empty — see above.
 */
const plugin = Object.assign(createPlugin(), { inject })
export default plugin

/**
 * Normalize configuration, treating a validation failure as fatal.
 *
 * The runtime validates `Config` before calling us, so reaching this with bad
 * input means the value was constructed programmatically; failing loudly is
 * better than running with a half-applied configuration.
 *
 * @param raw - raw config value.
 * @returns the normalized configuration.
 */
function normalizeOrThrow(raw: unknown): QuorfloatConfig {
  const result = Config['~standard'].validate(raw)
  if ('issues' in result) {
    const detail = result.issues.map(issue => `${issue.message}${issue.path ? ` (at ${issue.path.join('.')})` : ''}`).join('; ')
    throw new TypeError(`dsh-quorfloat: invalid configuration: ${detail}`)
  }
  return result.value
}

/**
 * Reverse activation in the documented order.
 *
 * @param activation - the activation record.
 * @param log - logger.
 */
async function deactivate(activation: Activation, log: Logger): Promise<void> {
  activation.stopped = true
  try {
    activation.presenceGateway?.()
  } catch {
    // Unregistering the browser-facing service must not block the rest of teardown.
  }
  activation.presenceGateway = undefined
  try {
    activation.controlGateway?.()
  } catch {
    // Unregistering the browser-facing control service must not block the rest of teardown.
  }
  activation.controlGateway = undefined
  try {
    activation.injection?.()
  } catch {
    // Releasing a deferred activation must not prevent the rest of teardown.
  }
  activation.injection = undefined
  // 1. stop accepting new work and withdraw requests that would otherwise hang
  //    an in-flight turn;
  activation.interactions?.abortAll('plugin deactivating')
  activation.interactions?.unregister()
  // 2. release Harness subscriptions, leaving Harness-side work untouched;
  activation.sessionLayer?.detachAll('plugin deactivating')
  // 3. stop the process and report what actually happened.
  const supervisor = activation.supervisor
  if (supervisor !== undefined) {
    const result = await supervisor.stop()
    if (!result.exited) {
      log.error('quorfloat did not exit within the shutdown budget', result)
    } else {
      log.info('quorfloat stopped', result)
    }
  }
}

/**
 * Wrap a logger so the configured level actually filters output.
 *
 * The host logger has its own level handling; this keeps plugin-side noise
 * (frame-by-frame traces) out of the log unless it was explicitly requested.
 *
 * @param logger - the host logger.
 * @param level - configured maximum verbosity.
 * @returns a logger that filters by level.
 */
function wrapLogger(logger: Logger, level: QuorfloatConfig['logLevel']): Logger {
  const order = { error: 0, warn: 1, info: 2, debug: 3 } as const
  const allowed = order[level]
  return {
    error: (...args) => logger.error(...args),
    warn: (...args) => {
      if (allowed >= order.warn) logger.warn(...args)
    },
    info: (...args) => {
      if (allowed >= order.info) logger.info(...args)
    },
    debug: (...args) => {
      if (allowed >= order.debug) logger.debug(...args)
    },
  }
}

/**
 * Read the running Harness version for diagnostics.
 *
 * Deliberately best-effort: the version is reported, never enforced, because
 * the compatibility that matters is service behaviour, which is probed directly.
 *
 * @returns a version string, or `unknown`.
 */
function hostVersion(): string {
  const version = process.env['DSH_VERSION']
  return typeof version === 'string' && version !== '' ? version : 'unknown'
}

/**
 * Reduce a supervisor snapshot to the fields worth logging.
 *
 * @param snapshot - full snapshot.
 * @returns a compact summary.
 */
function summarize(snapshot: SupervisorSnapshot): Record<string, unknown> {
  return {
    state: snapshot.state,
    handshaken: snapshot.handshaken,
    pid: snapshot.pid ?? null,
    binary: snapshot.binaryPath ?? null,
    hotkey: snapshot.hotkey,
    restarts: snapshot.restarts,
  }
}

/** Allocate a correlation id for one user submission. */
export function newPromptRequestId(): string {
  return `qf-${randomUUID()}`
}

export { HarnessError }
export type { ChannelError }
