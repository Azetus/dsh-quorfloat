/**
 * Process supervision for the quorfloat subproject.
 *
 * The supervisor owns exactly one child process at a time and answers four
 * questions the rest of the plugin must not guess at:
 * - is a process supposed to be running, and is one actually running;
 * - did the protocol handshake complete, and did the peer register its hotkey;
 * - should an exit be restarted, or is it the expected end of a stop;
 * - when the host goes away, is the child really gone (not just signalled).
 *
 * Every "the child is fine" claim in the rest of the plugin is derived from a
 * snapshot of this object, not from the fact that `spawn` once returned a pid.
 */

import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { homedir } from 'node:os'

import type { QuorfloatConfig } from '../config.js'
import { PROTOCOL_VERSION } from '../protocol.js'
import { ChannelError } from '../bridge/errors.js'
import { QuorfloatChannel } from '../bridge/channel.js'
import { HostRouter, METHODS, type RouterHost } from '../bridge/router.js'
import type { ResolvedBinary } from './binary.js'

/** Coarse lifecycle phase; a snapshot always reports exactly one. */
export type SupervisorState =
  | 'stopped'
  | 'starting'
  | 'running'
  | 'degraded'
  | 'failed'
  | 'stopping'

/** Structured snapshot used by the settings/status surface and by diagnostics. */
export interface SupervisorSnapshot {
  readonly state: SupervisorState
  /** True only between a successful handshake and channel loss. */
  readonly handshaken: boolean
  /** What the peer reported about hotkey registration; `null` before handshake. */
  readonly hotkey: { readonly requested: string; readonly registered: boolean } | null
  readonly pid: number | undefined
  readonly binaryPath: string | undefined
  readonly binarySource: ResolvedBinary['source'] | undefined
  readonly startedAt: number | undefined
  readonly restarts: number
  readonly consecutiveMissedHeartbeats: number
  readonly rejectedFrames: number
  readonly lastFrameAt: number
  /** Human-readable reason for `failed`, or for the most recent `degraded` phase. */
  readonly lastError: { readonly code: string; readonly message: string } | undefined
  /** True when automatic restarts are exhausted and a manual retry is required. */
  readonly restartExhausted: boolean
  /**
   * Whether the panel last reported itself visible.
   *
   * `false` before the first report and after a restart: an unseen panel must
   * never be assumed visible, because that would make the plugin defer approvals
   * to a window nobody is looking at.
   */
  readonly panelVisible: boolean
  /** Capabilities the panel process has measured and reported. */
  readonly panelCapabilities: readonly string[]
  /**
   * Capabilities the panel declared in `hello` — what its build implements.
   *
   * Distinct from `panelCapabilities` on purpose: that one is measured at runtime
   * (a window that was really created), this one is the build inventory written
   * before any window exists. It is what decides whether a claimed request can be
   * rendered: claiming is exclusive, so claiming what the panel cannot answer hides
   * the request from every other answerer for the whole claim deadline.
   *
   * Empty before the handshake, which defers everything — the safe direction.
   */
  readonly peerCapabilities: readonly string[]
}

/** Event delivered to listeners on every meaningful supervision change. */
export interface SupervisorEvent {
  readonly kind: 'state' | 'handshake' | 'frame-rejected' | 'stderr' | 'exit' | 'restart-given-up'
  readonly snapshot: SupervisorSnapshot
  readonly detail?: unknown
}

/** Inputs the supervisor needs from the plugin around it. */
export interface SupervisorDeps {
  /** Current effective configuration. Read on every transition. */
  config(): QuorfloatConfig
  /** Build the method router for a fresh channel. */
  createRouter(channelSessionId: string): HostRouter
  /** Run one step of the binary resolution. Injectable for tests. */
  resolveBinary(): ResolvedBinary
  /** Structured logger. */
  log: {
    info(message: string, detail?: unknown): void
    warn(message: string, detail?: unknown): void
    error(message: string, detail?: unknown): void
    debug(message: string, detail?: unknown): void
  }
  /** Observation hook; must not throw. */
  onEvent?(event: SupervisorEvent): void
  /**
   * Forward one inbound peer notification to the session layer.
   *
   * The supervisor observes these frames for liveness and logs them, but it
   * deliberately owns no subscription state: forwarding keeps "which session is
   * being followed" in the session layer, where switching and cursors live.
   */
  onPeerNotification?(method: string, params: unknown): void
}

/** How many peer stderr lines are forwarded before suppression kicks in. */
/** Exponential-ish backoff for automatic restarts: 0.5s, 1s, 2s, capped at the window. */
function backoffMs(attempt: number, windowMs: number): number {
  return Math.min(500 * 2 ** Math.max(0, attempt - 1), Math.max(500, Math.floor(windowMs / 4)))
}

export class QuorfloatSupervisor {
  readonly #deps: SupervisorDeps
  #channel: QuorfloatChannel | undefined
  #router: HostRouter | undefined
  #child: ChildProcessWithoutNullStreams | undefined
  #state: SupervisorState = 'stopped'
  #binary: ResolvedBinary | undefined
  #startedAt: number | undefined
  #lastFrameAt = 0
  #restarts = 0
  #restartTimestamps: number[] = []
  #consecutiveMissed = 0
  #restartExhausted = false
  #lastError: { code: string; message: string } | undefined
  #heartbeatTimer: NodeJS.Timeout | undefined
  #restartTimer: NodeJS.Timeout | undefined
  #generation = 0
  /** Peer stderr lines already forwarded for the current channel. */
  #stderrDelivered = 0
  /** Last visibility the panel reported; `false` until it says otherwise. */
  #panelVisible = false
  /**
   * What the panel process reported it can do, measured on its side.
   *
   * Empty until it reports, which is the honest default: an unreported panel must
   * not be assumed capable of showing anything.
   */
  #panelCapabilities: readonly string[] = []
  /** True while a stop was requested, so the exit handler must not restart. */
  #stopping = false

  /**
   * @param deps - configuration access, router factory, resolver, and logging.
   */
  constructor(deps: SupervisorDeps) {
    this.#deps = deps
  }

  /** Current supervisor snapshot. */
  snapshot(): SupervisorSnapshot {
    const channel = this.#channel
    return {
      state: this.#state,
      handshaken: this.#router?.handshaken === true && channel?.state === 'started',
      hotkey: this.#router?.handshake?.hotkey ?? null,
      pid: this.#child?.pid,
      binaryPath: this.#binary?.path,
      binarySource: this.#binary?.source,
      startedAt: this.#startedAt,
      restarts: this.#restarts,
      consecutiveMissedHeartbeats: this.#consecutiveMissed,
      rejectedFrames: channel?.rejectedFrames ?? 0,
      lastFrameAt: Math.max(this.#lastFrameAt, channel?.lastFrameAt ?? 0),
      lastError: this.#lastError,
      restartExhausted: this.#restartExhausted,
      panelVisible: this.#panelVisible,
      panelCapabilities: this.#panelCapabilities,
      peerCapabilities: this.#router?.handshake?.capabilities ?? [],
    }
  }

  /**
   * Start the child process. Idempotent while a process is starting or running.
   *
   * @returns the snapshot after the spawn attempt was made.
   */
  async start(): Promise<SupervisorSnapshot> {
    if (this.#state === 'starting' || this.#state === 'running' || this.#state === 'degraded') {
      return this.snapshot()
    }
    this.#stopping = false
    this.#restartExhausted = false
    const config = this.#deps.config()
    if (!config.enabled) {
      this.#transition('stopped', { code: 'disabled', message: 'plugin disabled by configuration' })
      return this.snapshot()
    }

    let binary: ResolvedBinary
    try {
      binary = this.#deps.resolveBinary()
    } catch (error) {
      const failure = error instanceof ChannelError
        ? error
        : new ChannelError('unavailable', error instanceof Error ? error.message : String(error))
      this.#binary = undefined
      this.#lastError = { code: failure.code, message: failure.message }
      this.#transition('failed', this.#lastError)
      return this.snapshot()
    }
    this.#binary = binary
    return await this.#spawn(binary, config)
  }

  /**
   * Stop the child process and wait for the observed exit.
   *
   * @returns how the process ended; `exited: true` means it is gone.
   */
  async stop(): Promise<{ exited: boolean; code: number | null; signal: NodeJS.Signals | null; escalated: boolean }> {
    this.#stopping = true
    this.#clearTimers()
    const channel = this.#channel
    if (channel === undefined) {
      this.#transition('stopped', undefined)
      return { exited: true, code: null, signal: null, escalated: false }
    }
    this.#transition('stopping', undefined)
    const result = await channel.close(this.#deps.config().shutdownGraceMs)
    this.#channel = undefined
    this.#router = undefined
    this.#child = undefined
    this.#transition('stopped', result.exited ? undefined : { code: 'channel-closed', message: 'process did not exit in time' })
    return result
  }

  /**
   * Stop and start again, used by configuration reload and by a manual retry.
   *
   * @returns the snapshot after the restart settled.
   */
  async restart(): Promise<SupervisorSnapshot> {
    await this.stop()
    this.#restartExhausted = false
    this.#restartTimestamps = []
    this.#restarts = 0
    return await this.start()
  }

  /**
   * Ask the peer to refresh its view after a configuration change that does not
   * require a new process (window size, theme, hotkey).
   *
   * @returns `true` when the peer acknowledged.
   */
  async pushConfig(): Promise<boolean> {
    const channel = this.#channel
    if (channel === undefined || channel.state !== 'started' || this.#router?.handshaken !== true) {
      return false
    }
    const config = this.#deps.config()
    try {
      await channel.notify('host/config', {
        window: config.window,
        hotkey: config.hotkey,
        defaultWorkspaceId: config.defaultWorkspaceId,
        logLevel: config.logLevel,
      })
      return true
    } catch (error) {
      this.#deps.log.warn('failed to push configuration to quorfloat', error)
      return false
    }
  }

  /**
   * Ask the peer to show or hide its window.
   *
   * The missing direction of a two-way fact: the peer tells the host where its
   * window is, so the host can route approvals; this tells the peer where the host
   * wants it. Without it the panel would be summoned by the global hotkey only,
   * and a host-side action such as opening a pending approval could not bring it
   * into view.
   *
   * Sent as a notification rather than a request: the peer publishes its resulting
   * visibility back through `window/visibility`, so a reply would be a second way
   * to learn the same thing — and one that has to be timed out when the peer is
   * slow to draw.
   *
   * @param visible - `true` to show the window, `false` to hide it.
   * @returns `true` when the frame was written; `false` when there is no live peer,
   *   which is not an error — the panel is optional and may not exist at all.
   */
  async setPanelVisible(visible: boolean): Promise<boolean> {
    const channel = this.#channel
    if (channel === undefined || channel.state !== 'started') return false
    try {
      await channel.notify('window/visibility', { visible })
      return true
    } catch (error) {
      this.#deps.log.warn('failed to ask quorfloat to change window visibility', error)
      return false
    }
  }

  /**
   * Send one notification to the peer.
   *
   * Used by the session layer (frame forwarding) and by the interaction layer
   * (pending approvals). A missing or un-handshaken channel is not an error:
   * the caller is projecting state that the peer will rebuild from a snapshot
   * when it reconnects.
   *
   * @param method - protocol notification method.
   * @param params - notification payload.
   * @returns `true` when the frame was written.
   */
  async notify(method: string, params: unknown): Promise<boolean> {
    const channel = this.#channel
    if (channel === undefined || channel.state !== 'started') return false
    try {
      await channel.notify(method, params)
      return true
    } catch (error) {
      this.#deps.log.debug('dropping a notification to quorfloat', {
        method,
        error: error instanceof Error ? error.message : String(error),
      })
      return false
    }
  }

  /** Diagnostic view of the live channel, when one exists. */
  channelDiagnostics(): Record<string, unknown> {
    const channel = this.#channel
    if (channel === undefined) return { channel: null }
    return {
      channel: {
        state: channel.state,
        pid: channel.pid,
        readBytes: channel.readBytes,
        writtenBytes: channel.writtenBytes,
        rejectedFrames: channel.rejectedFrames,
        exitResult: channel.exitResult ?? null,
      },
    }
  }

  /**
   * Spawn one child and wire the channel to it.
   *
   * @param binary - resolved executable.
   * @param config - effective configuration.
   * @returns the snapshot once the spawn was attempted.
   */
  async #spawn(binary: ResolvedBinary, config: QuorfloatConfig): Promise<SupervisorSnapshot> {
    this.#generation += 1
    const generation = this.#generation
    this.#stderrDelivered = 0
    // A fresh process has not reported anything yet. Both facts are cleared
    // together: a capability established by the process that just died says
    // nothing about its replacement.
    this.#panelVisible = false
    this.#panelCapabilities = []
    // A new attempt invalidates the previous failure reason: keeping it would
    // make a live process report the error that ended its predecessor.
    this.#lastError = undefined
    this.#transition('starting', undefined)
    this.#consecutiveMissed = 0

    let child: ChildProcessWithoutNullStreams
    try {
      child = spawn(binary.path, [...binary.args], {
        // Absolute path, no shell: nothing here re-parses a path through a
        // shell, and stdio stays a dedicated pipe the supervisor owns.
        cwd: safeWorkingDirectory(),
        env: quorfloatEnvironment(config),
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
      }) as ChildProcessWithoutNullStreams
    } catch (error) {
      const failure = new ChannelError('spawn-failed', error instanceof Error ? error.message : String(error))
      this.#lastError = { code: failure.code, message: failure.message }
      this.#transition('failed', this.#lastError)
      return this.snapshot()
    }
    this.#child = child

    const channelSessionId = `quorfloat-${generation}-${Date.now().toString(36)}`
    const router = this.#deps.createRouter(channelSessionId)
    const channel = new QuorfloatChannel(child, {
      requestTimeoutMs: config.requestTimeoutMs,
      maxWriteBufferBytes: config.maxWriteBufferBytes,
      onNotification: (method, params) => {
        if (method === '$stderr') {
          // The peer's own narration is the only window into what it is doing,
          // and the peer library we ship (the Rust subproject, or the probe)
          // reports its failures there. Losing it would make a stalled handshake
          // indistinguishable from a process that never started.
          const line = typeof params === 'string' ? params : String(params)
          const limit = this.#deps.config().maxStderrLines
          if (this.#stderrDelivered < limit) {
            this.#deps.log.info('quorfloat:', line)
            this.#stderrDelivered += 1
          } else if (this.#stderrDelivered === limit) {
            this.#deps.log.info('quorfloat: (further stderr lines are suppressed)', { limit })
            this.#stderrDelivered += 1
          }
          this.#emit({ kind: 'stderr', snapshot: this.snapshot(), detail: line })
          return
        }
        this.#lastFrameAt = Date.now()
        this.#onNotification(method, params)
      },
      onRejectedFrame: rejection => {
        this.#deps.log.warn('rejected a frame from quorfloat', rejection)
        this.#emit({ kind: 'frame-rejected', snapshot: this.snapshot(), detail: rejection })
      },
      onClosed: reason => {
        // A channel that closes while we are stopping is expected; anything
        // else is a loss that the restart policy has to judge.
        if (generation !== this.#generation) return
        if (this.#stopping) {
          this.#deps.log.info('quorfloat channel closed during shutdown', reason.message)
          return
        }
        this.#lastError = { code: reason.code, message: reason.message }
        this.#deps.log.warn('quorfloat channel lost', reason.message)
        this.#handleUnexpectedExit(generation, reason)
      },
    })
    for (const [method, handler] of routerMethods(router)) channel.handle(method, handler)
    this.#channel = channel
    this.#router = router
    channel.start()

    // The peer sends `hello` as its first frame. The host waits for that frame
    // instead of racing a request of its own, and treats silence as a start
    // failure rather than as "probably still booting".
    const handshakeTimer = setTimeout(() => {
      if (generation !== this.#generation) return
      if (router.handshaken) return
      const failure = new ChannelError('handshake-timeout', `quorfloat did not complete the handshake within ${config.startupTimeoutMs}ms`, {
        startupTimeoutMs: config.startupTimeoutMs,
      })
      this.#lastError = { code: failure.code, message: failure.message }
      this.#deps.log.error('quorfloat handshake timed out', failure.message)
      void this.#restartAfterFailure(generation, failure)
    }, config.startupTimeoutMs)
    handshakeTimer.unref?.()

    // Wrap `hello` so a successful handshake cancels the startup budget, marks
    // the process as running, pushes the effective config, and starts the
    // heartbeat loop — the router itself stays unaware of all four.
    channel.handle('hello', async (params: unknown) => {
      const result = await router.handle('hello', params)
      clearTimeout(handshakeTimer)
      if (generation === this.#generation) {
        this.#startedAt = this.#startedAt ?? Date.now()
        this.#transition('running', undefined)
        this.#emit({ kind: 'handshake', snapshot: this.snapshot(), detail: result })
        await this.#sendReady(config)
        this.#startHeartbeat(generation)
      }
      return result
    })

    child.on('exit', (code, signal) => {
      if (generation !== this.#generation) return
      clearTimeout(handshakeTimer)
      this.#emit({ kind: 'exit', snapshot: this.snapshot(), detail: { code, signal } })
      if (this.#stopping) return
      this.#handleUnexpectedExit(
        generation,
        new ChannelError('helper-exited', `quorfloat exited (code=${String(code)}, signal=${String(signal)})`),
      )
    })

    this.#deps.log.info('spawned quorfloat', { path: binary.path, args: binary.args.length, source: binary.source, pid: child.pid })
    return this.snapshot()
  }

  /**
   * Send the post-handshake `ready` notification carrying the effective config.
   *
   * @param config - effective configuration.
   */
  async #sendReady(config: QuorfloatConfig): Promise<void> {
    const channel = this.#channel
    if (channel === undefined) return
    try {
      await channel.notify(
        'ready',
        {
          protocol: PROTOCOL_VERSION,
          config: {
            window: config.window,
            hotkey: config.hotkey,
            defaultWorkspaceId: config.defaultWorkspaceId,
            heartbeatMs: config.heartbeatMs,
            logLevel: config.logLevel,
          },
        },
      )
    } catch (error) {
      this.#deps.log.warn('failed to send ready to quorfloat', error)
    }
  }

  /**
   * Start the heartbeat loop for one generation.
   *
   * @param generation - the spawn generation this loop belongs to.
   */
  #startHeartbeat(generation: number): void {
    this.#clearHeartbeat()
    const config = this.#deps.config()
    if (config.heartbeatMs <= 0) return
    this.#heartbeatTimer = setInterval(() => {
      void this.#beat(generation)
    }, config.heartbeatMs)
    this.#heartbeatTimer.unref?.()
  }

  /**
   * Run one heartbeat: notify the peer, and confirm it is still answering.
   *
   * @param generation - the spawn generation this beat belongs to.
   */
  async #beat(generation: number): Promise<void> {
    if (generation !== this.#generation) return
    const channel = this.#channel
    if (channel === undefined || channel.state !== 'started') return
    try {
      await channel.notify('host/heartbeat', { t: Date.now() })
      // `ping` is deliberately a request: a notification would only prove that
      // the pipe accepts bytes, not that the peer is still running its loop.
      await channel.request('ping', {}, Math.max(1000, this.#deps.config().heartbeatMs))
      this.#consecutiveMissed = 0
      return
    } catch (error) {
      this.#consecutiveMissed += 1
      this.#deps.log.warn('quorfloat missed a heartbeat', {
        missed: this.#consecutiveMissed,
        limit: this.#deps.config().heartbeatMissLimit,
        error: error instanceof Error ? error.message : String(error),
      })
      if (this.#consecutiveMissed >= this.#deps.config().heartbeatMissLimit) {
        await this.#restartAfterFailure(
          generation,
          new ChannelError('request-timeout', `quorfloat stopped answering heartbeats (${this.#consecutiveMissed} missed)`, {
            missed: this.#consecutiveMissed,
          }),
        )
      }
    }
  }

  /**
   * Apply the restart policy to an unexpected loss.
   *
   * @param generation - the generation that was lost.
   * @param reason - why it was lost.
   */
  async #handleUnexpectedExit(generation: number, reason: ChannelError): Promise<void> {
    if (generation !== this.#generation) return
    if (this.#stopping) return
    // A single loss produces several signals (the 'exit' event, the channel's
    // own close, and possibly a failed heartbeat). Only the first one drives the
    // policy; the rest are absorbed here.
    // A single loss produces several signals (the 'exit' event, the channel's own
    // close, and possibly a failed heartbeat). Only the first one reaches the
    // policy; the rest are absorbed here, including the case where the process
    // died while a retry was already in flight.
    if (this.#state === 'degraded' || this.#state === 'failed') return
    if (this.#restartTimer !== undefined || this.#restartExhausted) return
    this.#transition('degraded', { code: reason.code, message: reason.message })
    await this.#scheduleRestart(generation)
  }

  /**
   * Tear down a broken channel and schedule a restart inside the policy.
   *
   * @param generation - the generation to abandon.
   * @param reason - the failure that triggered the restart.
   */
  async #restartAfterFailure(generation: number, reason: ChannelError): Promise<void> {
    if (generation !== this.#generation) return
    this.#lastError = { code: reason.code, message: reason.message }
    await this.stop()
    // `stop()` marks the supervisor as deliberately stopping, which suppresses
    // the exit handler; the retry path has to clear that flag before it asks for
    // a new process, or the restart would be treated as an unwanted one.
    this.#stopping = false
    await this.#scheduleRestart(generation)
  }

  /**
   * Enforce the restart budget and schedule the next spawn.
   *
   * @param generation - the generation that was lost.
   */
  async #scheduleRestart(generation: number): Promise<void> {
    if (generation !== this.#generation) return
    const config = this.#deps.config()
    const now = Date.now()
    this.#restartTimestamps = this.#restartTimestamps.filter(at => now - at < config.restartWindowMs)
    if (this.#restartExhausted) {
      // Possible when a previous attempt failed while a retry was still in
      // flight: the budget is already spent, so settle on the terminal state
      // instead of leaving the supervisor reporting a retry it will not make.
      this.#transition('failed', this.#lastError)
      return
    }
    if (this.#restartTimestamps.length >= config.restartLimit) {
      this.#restartExhausted = true
      this.#lastError = {
        code: 'unavailable',
        message: `automatic restart gave up after ${config.restartLimit} attempts inside ${config.restartWindowMs}ms; use the manual retry entry`,
      }
      this.#deps.log.error('quorfloat restart budget exhausted', this.#lastError.message)
      this.#transition('failed', this.#lastError)
      this.#emit({ kind: 'restart-given-up', snapshot: this.snapshot() })
      return
    }
    this.#restartTimestamps.push(now)
    this.#restarts += 1
    const delay = backoffMs(this.#restarts, config.restartWindowMs)
    this.#deps.log.info('restarting quorfloat', { attempt: this.#restarts, delayMs: delay })
    // `start()` refuses to run while a process is considered live, so the
    // degraded state is released here: from this point the supervisor is
    // deliberately stopped and waiting for the retry timer.
    this.#transition('stopped', undefined)
    this.#restartTimer = setTimeout(() => {
      this.#restartTimer = undefined
      void this.start().then(() => this.pushConfig())
    }, delay)
    this.#restartTimer.unref?.()
  }

  /**
   * Track inbound notifications that affect supervision.
   *
   * @param method - notification method.
   * @param params - notification payload.
   */
  #onNotification(method: string, params: unknown): void {
    switch (method) {
      case 'window/visibility': {
        // The panel reports whether it is on screen. It is a peer-emitted fact, so
        // it is carried in the snapshot rather than only passed to one observer:
        // the approval authority reads it on every decision, not just when the
        // notification happens to arrive.
        const record = (params ?? {}) as { visible?: unknown; capabilities?: unknown }
        const visible = record.visible === true
        // What the peer has established it can do, measured on its side. Optional
        // and additive, so an older peer that omits it simply leaves the last
        // measurement standing rather than clearing it.
        const reported = Array.isArray(record.capabilities)
          ? record.capabilities.filter((entry): entry is string => typeof entry === 'string')
          : undefined
        if (reported !== undefined) this.#panelCapabilities = reported
        this.#deps.log.debug('quorfloat visibility changed', { visible, capabilities: reported })
        this.#panelVisible = visible
        this.#emit({ kind: 'state', snapshot: this.snapshot(), detail: { visible } })
        return
      }
      default: {
        // Session traffic (`session/*`, `interaction/*`, and anything a future
        // protocol version adds) is the session layer's business. Supervision
        // only needs the fact that a frame arrived, which `#lastFrameAt` records.
        try {
          this.#deps.onPeerNotification?.(method, params)
        } catch (error) {
          this.#deps.log.warn('peer notification observer threw', error)
        }
      }
    }
  }

  /**
   * Move to a new state and publish the change.
   *
   * @param state - the new state.
   * @param error - optional failure detail to record.
   */
  #transition(state: SupervisorState, error: { code: string; message: string } | undefined): void {
    if (error !== undefined) this.#lastError = error
    if (this.#state === state) return
    this.#state = state
    this.#emit({ kind: 'state', snapshot: this.snapshot() })
  }

  /**
   * Publish one event without letting an observer break supervision.
   *
   * @param event - the event to publish.
   */
  #emit(event: SupervisorEvent): void {
    try {
      this.#deps.onEvent?.(event)
    } catch (error) {
      this.#deps.log.warn('supervisor event observer threw', error)
    }
  }

  /** Stop the heartbeat timer. */
  #clearHeartbeat(): void {
    if (this.#heartbeatTimer !== undefined) {
      clearInterval(this.#heartbeatTimer)
      this.#heartbeatTimer = undefined
    }
  }

  /** Stop every timer owned by the supervisor. */
  #clearTimers(): void {
    this.#clearHeartbeat()
    if (this.#restartTimer !== undefined) {
      clearTimeout(this.#restartTimer)
      this.#restartTimer = undefined
    }
  }
}

/**
 * Bind router methods to a plain function map for channel registration.
 *
 * The list of methods comes from the router itself: a second copy here would drift
 * from the dispatch and answer "unsupported method" for a method that is implemented.
 *
 * @param router - the router to expose.
 * @returns method name → handler.
 */
function routerMethods(router: HostRouter): Map<string, (params: unknown, method: string) => Promise<unknown>> {
  const map = new Map<string, (params: unknown, method: string) => Promise<unknown>>()
  for (const method of METHODS) {
    map.set(method, async (params: unknown) => await router.handle(method, params))
  }
  return map
}

/**
 * Choose a working directory that can never become a session workspace by accident.
 *
 * The child is given the user's home directory: the Harness project directory
 * would be indistinguishable from a real workspace, and the plugin's own
 * installation directory would put session data next to code.
 *
 * @returns an absolute directory path.
 */
function safeWorkingDirectory(): string {
  // `HOME` first so an explicit override wins, then the platform's own answer.
  // On Windows `HOME` is usually absent and `homedir()` resolves `USERPROFILE` or
  // `HOMEDRIVE`+`HOMEPATH`, which is the only way to get this right there.
  return process.env['HOME'] ?? homedir()
}

/**
 * Build the child environment.
 *
 * Only what quorfloat needs to start, draw, and be located is forwarded; model
 * credentials and unrelated Harness internals stay in the host process.
 *
 * @param config - effective configuration.
 * @returns the environment for the child.
 */
function quorfloatEnvironment(config: QuorfloatConfig): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    // The child must be able to find its own bundled tools and system libraries,
    // so PATH survives; nothing else that could carry credentials does.
    PATH: process.env['PATH'] ?? '',
    DSH_QUORFLOAT_PROTOCOL: PROTOCOL_VERSION,
    DSH_QUORFLOAT_PLATFORM: `${process.platform}-${process.arch}`,
    DSH_QUORFLOAT_PARENT_PID: String(process.pid),
    DSH_QUORFLOAT_LOG_LEVEL: config.logLevel,
    // The accelerator the user configured, so the child can attempt to register
    // it without reading the host's config files. It reports back in `hello`
    // whether the grab succeeded, which is how a conflict becomes visible in the
    // settings surface instead of the key silently doing nothing.
    DSH_QUORFLOAT_HOTKEY: config.hotkey,
    // Window geometry at spawn time. The same values also arrive in the `ready`
    // payload, but by then the first frame has already been drawn: without these
    // the panel would appear at its built-in default and visibly resize a moment
    // later, which is exactly the kind of flicker a summoned panel must not have.
    DSH_QUORFLOAT_WINDOW_WIDTH: String(config.window.width),
    DSH_QUORFLOAT_WINDOW_MAX_HEIGHT: String(config.window.maxHeight),
    DSH_QUORFLOAT_WINDOW_ALWAYS_ON_TOP: String(config.window.alwaysOnTop),
    DSH_QUORFLOAT_WINDOW_REDUCE_MOTION: String(config.window.reduceMotion),
    // Where the panel opens when there is no remembered position to restore (and where it falls
    // back to when the remembered one is on no attached display). It has to be here and not only
    // in the `ready` payload for the same reason as the numbers above: the placement happens the
    // moment the window is created, which is before any frame from the host has been processed.
    DSH_QUORFLOAT_WINDOW_ANCHOR: config.window.anchor,
    // The language the panel draws and labels its menu bar in. `''` is the schema's
    // "unset" and reaches the peer as it is; the plugin resolves it from the Harness's
    // locale preference before constructing this supervisor, so in a real profile this
    // is always `zh` or `en`. It is here as well as in the `ready` payload because the
    // sidecar persists the first-launch value as its own setting, and that write
    // happens before any frame arrives.
    DSH_QUORFLOAT_WINDOW_LANGUAGE: config.window.language,
    // And whether that value is a decision or only the `en` fallback. It has to reach the
    // sidecar *before* it writes anything down: the fallback is also what a launch nobody
    // decided resolves to, and persisting it would freeze the panel in English without
    // ever letting its webview answer. Only an explicit `false` says "fallback"; a
    // configuration that leaves it unset is treated as a decision.
    DSH_QUORFLOAT_WINDOW_LANGUAGE_DECIDED: config.window.languageDecided === false ? '0' : '1',
  }
  // POSIX and Windows names for the same facts, both forwarded. The Windows
  // entries are not decoration: `HOME` is normally unset there, and while the host
  // can fall back to `os.homedir()` (which consults the passwd database on POSIX),
  // a native child cannot — the GUI stack locates its caches through `USERPROFILE`
  // and `LOCALAPPDATA`, so withholding them makes the window fail to appear for a
  // reason nothing in the log explains.
  for (const key of [
    // POSIX
    'HOME', 'USER', 'TMPDIR', 'LANG', 'LC_ALL', 'LC_CTYPE',
    'XDG_RUNTIME_DIR', 'DISPLAY', 'WAYLAND_DISPLAY',
    // Windows
    'USERPROFILE', 'LOCALAPPDATA', 'APPDATA', 'TEMP', 'TMP', 'USERNAME', 'SystemRoot',
  ]) {
    const value = process.env[key]
    if (value !== undefined) env[key] = value
  }
  // Explicit, namespaced passthrough. Nothing else crosses over, so a credential
  // that happens to sit in the host environment cannot leak into the child by
  // accident, while scripted runs keep one supported way to configure it.
  for (const [key, value] of Object.entries(process.env)) {
    if (value !== undefined && key.startsWith('DSH_QUORFLOAT_')) env[key] = value
  }
  return env
}
