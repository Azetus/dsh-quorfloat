/**
 * Browser half: report whether the user is looking at the Harness window, and
 * put the sidecar's state — with the one control that starts or stops it — on
 * both surfaces that can show it: a page of this plugin's own inside Settings →
 * Built-in plugins, and the two contribution slots the sidebar 插件 page
 * declares on every detail view.
 *
 * Why the presence report exists at all: the approval authority needs to know
 * which surface the user is looking at, and that fact lives only here. The
 * Electron shell knows it (`window.isVisible()`) but does not expose it to
 * plugins, and the host process has no window concept. Platform behaviour this
 * file must respect:
 *
 * - `document.visibilityState` alone is not the answer. A window fully occluded
 *   by another application reports `hidden` on macOS but `visible` on Windows,
 *   and a window on another desktop reports `hidden` on macOS and `visible` on
 *   Windows. Only `visible && document.hasFocus()` means "the user is looking at
 *   it", and that pair gives the same answer on both platforms.
 * - `visibilitychange` still fires while the page is hidden, so the host learns
 *   about the change immediately instead of waiting for a report to go stale.
 *
 * Why the page half lives in this same module: the hand-rolled build
 * (`scripts/build-client.mjs`) wraps exactly one compiled module into the
 * loader factory the shell evaluates, and the browser half ships no
 * dependencies of its own — it `require`s React and the shared UI primitives
 * from the loader's module table, the way `templates/decoration/client.js` of
 * the harness's own `cordis-plugin-development` skill does. Splitting the pure
 * decisions out would mean teaching that wrapper to bundle a module graph for
 * one page; the exported pure functions below are reachable through the
 * artifact's module namespace, which is the same surface the loader reads
 * `inject` and `apply` from.
 *
 * Where the page lives: two places, and each covers a case the other cannot.
 * The Settings section that hosts `settings.plugins.tab` renders one panel per
 * registration, keyed by the registration's own `id` (`{ only: … }` in
 * `packages/client/ui-settings-plugins/src/client/PluginsSettingsSection.tsx`),
 * so a fresh id puts this half beside the shipped inventory tab with no
 * cooperation from anything else. That tab is the fallback for every
 * composition the detail slots cannot serve — a plugin injected as a profile
 * patch row, a dev install, or a detail entry that fails to render — and it is
 * the one surface with room for the state and the control side by side: one row
 * with the indicator at its start and the action at its end, and the
 * supervisor's reason on the indicator's tooltip.
 *
 * The sidebar 插件 page (`ui-plugin-manager`) declares `plugins.detail.badge`
 * and `plugins.detail.actions` on every detail view and renders each entry with
 * the open page's `subject` (`slot-contract.ts`): `{ kind: 'bundle', pkg }` for
 * a package's page, `{ kind: 'row', pkg, row }` for a row's page, and
 * `{ kind: 'item', id }` for an official plugin's page. Those two slots are how
 * an installed bundle's page — and its row's page — carry this half's state tag
 * and its start/stop control next to DSH's own chrome. Because the page renders
 * them for every plugin, both entries are gated on the subject being this
 * plugin's ({@link ownsDetailSubject}); an entry that drew on any other page
 * would be a false claim about someone else's plugin.
 *
 * Every surface's job, in order: offer *start*, and tell the truth about the
 * state. After a quit from the tray menu the hotkey is gone with the process, so
 * the page half is the only way back to the panel; the state, the restart count,
 * and the supervisor's own reason are shown exactly as `quorfloat/status`
 * reports them, and nothing is claimed that the snapshot did not say.
 * Everything else — the plugin's enable/disable switch, every configuration
 * field, the read-only inventory — stays where DSH draws it.
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

/** One translate function, as the renderer supplies it to an entry that declared `locale:`. */
type Translate = (key: string, params?: Record<string, unknown>) => string

/** One list-slot registration; `id` is the cell key and `locale` the copy namespace. */
interface SlotEntryOptions {
  readonly name: string
  readonly id: string
  readonly order?: number
  /**
   * Display text where the owner projects one — the section renders it as the
   * tab's text. A thunk is re-read on every projection, which is how the label
   * follows the active language without re-registering.
   */
  readonly label?: () => string
  readonly locale?: string
}

/** What `settings.plugins.tab` hands each entry: its locale seat, and no owner props. */
interface TabProps {
  /**
   * The `t` seat of the namespace the entry declared.
   *
   * The section supplies no owner props of its own — a tab owns its whole
   * panel — so this seat is the only prop this half reads.
   */
  readonly t: Translate
}

/** One component registered into the tab slot. */
type SlotComponent = (props: TabProps) => unknown

/**
 * One row as a detail subject lists it.
 *
 * Declared by hand for the same reason the React face is: the shape crosses the
 * plugin-manager's store into this bundle, and the fields the gate reads are the
 * only ones worth naming.
 */
interface DetailRowRef {
  /** The row's own id inside its bundle — `quorfloat` for this plugin. */
  readonly rowId: string
  /** The module the row runs — this plugin's package name. */
  readonly moduleName: string
  /** Whether the row is switched on; unread here. */
  readonly enabled?: boolean
}

/** One package as a detail subject carries it. */
interface DetailPackageRef {
  /** The npm package name, which is what decides a bundle subject. */
  readonly name: string
  /** The package version, when the page has one. */
  readonly version?: string
  /** Whether the profile installed it; unread here. */
  readonly installed?: boolean
  /** Whether the bundle is enabled; unread here. */
  readonly enabled?: boolean
  /** The bundle's rows, as its page's 包含的组件 list shows them. */
  readonly rows?: readonly DetailRowRef[]
}

/**
 * The `subject` a `plugins.detail.*` entry is rendered with.
 *
 * Deliberately loose, and read as data that crossed a boundary: the page also
 * hands `{ kind: 'item', id }` subjects, and a harness that grows another kind
 * must make this half draw nothing rather than throw. Only the two fields the
 * gate inspects are named.
 */
interface DetailSubject {
  /** Which page this is: `bundle`, `row`, `item`, or something newer. */
  readonly kind?: string
  /** The bundle's package ref, on a bundle page and on a row page. */
  readonly pkg?: DetailPackageRef
  /** The row ref, on a row page. */
  readonly row?: DetailRowRef
}

/**
 * What `plugins.detail.*` hands each entry: the open page's subject, and the
 * `t` seat of the namespace the entry declared.
 */
interface PluginDetailProps {
  /** The open page; an entry renders null for a subject it has nothing for. */
  readonly subject: DetailSubject
  /** The `t` seat, supplied because the entry declares `locale`. */
  readonly t: Translate
}

/** One component registered into a detail slot. */
type DetailComponent = (props: PluginDetailProps) => unknown

/** The slice of the slot registry this half needs. */
interface SlotService {
  /** Run `callback` once `key` is declared; returns the disposer that withdraws it. */
  inject(key: string, callback: () => () => void): () => void
  /** Contribute `component` to a declared slot; returns the entry's disposer. */
  register<P>(options: SlotEntryOptions, component: (props: P) => unknown): () => void
}

/** The slice of the locale service this half needs. */
interface LocaleService {
  /** Add this plugin's dictionaries; returns the disposer that withdraws them. */
  register(namespace: string, dictionaries: Record<string, Record<string, string>>): () => void
  /**
   * Bind one namespace to a translate function that follows the active language.
   *
   * Needed for the tab's `label`, which the section reads through a thunk on
   * every projection: binding once and calling the thunk per read is what makes
   * the tab text follow a language switch with no re-registration.
   */
  bind(namespace: string): Translate
}

/** Client context surface used here. */
interface ClientContext {
  readonly connection: ClientConnection
  /** Register cleanup owned by this plugin's fiber; runs on unload. */
  effect(body: () => () => void, label?: string): () => void
  /**
   * Run `callback` in a child scope once every named service is available.
   *
   * Used instead of adding these services to `inject` on purpose: presence
   * reporting has to keep working in a composition that has no UI renderer
   * (a headless client), and one that cannot render the plugin page has
   * nothing for these slots to show.
   */
  inject(services: string[], callback: (scope: ClientContext) => void): () => void
  readonly slots: SlotService
  readonly locale: LocaleService
  logger(name?: string): { info(...args: unknown[]): void; warn(...args: unknown[]): void; debug(...args: unknown[]): void }
}

/**
 * The slice of React this half uses, declared by hand.
 *
 * The browser half ships no dependencies and is compiled without
 * `@types/react`, so the surface it uses is spelled out here. The instance
 * itself comes from the loader's module table (`require('react')`), which the
 * shell seeds with the one React every client bundle shares.
 */
interface ReactFace {
  createElement(type: unknown, props?: object | null, ...children: unknown[]): unknown
  useState<S>(initial: S): [S, (next: S) => void]
  useEffect(effect: () => void | (() => void), deps?: readonly unknown[]): void
}

/** The two shared primitives this half renders; the rest of the module is unused here. */
interface UiPrimitives {
  /** `@deepseek-ai/dsh-client-ui-primitives`' read-only capsule badge. */
  Tag(props: { tone?: string; children?: unknown }): unknown
  /** Its button atom; `onClick`/`disabled` pass through to the native button. */
  Button(props: Record<string, unknown>): unknown
}

/**
 * The module-table `require` the client loader hands this bundle's factory.
 *
 * The artifact `scripts/build-client.mjs` emits is
 * `window.__ModuleLoader__.load({ id, factory: (require) => … })`, and that
 * factory argument is the only thing in scope here that can answer a specifier:
 * the loader resolves platform seeds and other client bundles through it, and
 * nothing else — there is no `node_modules` in the page.
 */
declare const require: (specifier: string) => unknown

/** React, from the shared module table. */
const React = require('react') as ReactFace

/** The one platform seed this half renders with, so its controls match the Harness UI. */
const primitives = require('@deepseek-ai/dsh-client-ui-primitives') as UiPrimitives

/** Element constructor, short-named because every component below is a call tree. */
const h = React.createElement

/** Required browser services. */
export const inject = ['connection']

/** Logical channel the gateway serves host Remote namespaces on. */
const CHANNEL = '/api'

/** Endpoint: `<service key>/<method>` of the presence gateway on the host. */
const PRESENCE_ENDPOINT = 'quorfloat/reportPresence'

/** Endpoint: the supervisor snapshot this half renders. */
const STATUS_ENDPOINT = 'quorfloat/status'

/** Endpoint: the start/stop transition this half offers. */
const PANEL_ENDPOINT = 'quorfloat/panel'

/** Dictionary namespace owning the page half's copy. */
const LOCALE_NS = 'quorfloat.sidecar'

/**
 * Slot key carrying one page inside the Built-in plugins settings section.
 *
 * Declared by `ui-settings-plugins` as a root-scoped list slot whose entries
 * become ordered tabs, and rendered one panel at a time with `{ only: id }` —
 * the `id` of the registration, which is why a fresh one is enough.
 */
const TAB_SLOT = 'settings.plugins.tab'

/** This half's entry id inside that slot, and the key the section filters by. */
const ENTRY_ID = 'quorfloat'

/**
 * The two slots the sidebar 插件 page declares on every detail view.
 *
 * `ui-plugin-manager` declares both while its `main` panel is mounted: this one
 * beside the page's title, after the version, beta, and problem tags the page
 * draws itself; the actions one at the head of the page, before the page's own
 * switch and uninstall. Both are root-scoped list slots, rendered with the open
 * page's `subject`, so both need an entry that declines every subject that is
 * not this plugin's.
 */
const BADGE_SLOT = 'plugins.detail.badge'
const ACTIONS_SLOT = 'plugins.detail.actions'

/**
 * Where the detail entries sit among the page's contributed entries.
 *
 * The owner draws its own tags and buttons outside these slots, so this only
 * orders against other plugins' contributions: past the default, so this
 * plugin's state reads after chrome that answers "what is this package".
 */
const DETAIL_ORDER = 20

/**
 * The package name this half answers to.
 *
 * The module table serves the artifact under this name
 * (`scripts/build-client.mjs`), and both detail subjects carry it — the bundle
 * as `pkg.name`, the row as `row.moduleName`, because the bundle's patch
 * (`cordis.patch.yml`) names the row after the package.
 */
const BUNDLE_NAME = 'dsh-quorfloat'

/**
 * Whether the open detail page is this plugin's.
 *
 * The sidebar 插件 page renders `plugins.detail.badge` and
 * `plugins.detail.actions` on every plugin's page, so the entry has to answer
 * this before it draws anything. Two subjects can be ours:
 *
 * - a bundle page whose package is this package (`dsh-quorfloat`), which is the
 *   page an installed bundle's card opens;
 * - a row page whose row runs this package's module, which is the page the
 *   bundle's 包含的组件 row opens.
 *
 * Everything else — another plugin's bundle or row, an official plugin's
 * `item`, a kind that does not exist yet, and any malformed subject — is not
 * ours, and saying otherwise would put this plugin's state on another plugin's
 * page. Read defensively: the subject crossed the page's store, so `pkg` or
 * `row` may be missing on data this half has never seen.
 *
 * @param subject - the open page's subject, as the slot owner hands it over.
 * @returns whether the page belongs to this plugin.
 */
export function ownsDetailSubject(subject: unknown): boolean {
  if (typeof subject !== 'object' || subject === null) return false
  const record = subject as Record<string, unknown>
  if (record['kind'] === 'bundle') return stringField(record['pkg'], 'name') === BUNDLE_NAME
  if (record['kind'] === 'row') return stringField(record['row'], 'moduleName') === BUNDLE_NAME
  return false
}

/**
 * Read one string field off nested data, or undefined when it is not there.
 *
 * @param value - the nested value, which may be anything at all.
 * @param field - the field to read.
 * @returns the string, or undefined when the value is not a record or the field
 *   is not a string.
 */
function stringField(value: unknown, field: string): string | undefined {
  if (typeof value !== 'object' || value === null) return undefined
  const found = (value as Record<string, unknown>)[field]
  return typeof found === 'string' ? found : undefined
}

/**
 * Where the tab sits among the section's tabs.
 *
 * After the shipped read-only inventory (`ui-settings-plugin-inventory`
 * registers `all` at order 10): the section opens on its lowest-ordered tab, and
 * this plugin should not take over the default view of a shared settings page.
 * The tab is visible chrome, one click from the list the user already knows.
 */
const ENTRY_ORDER = 20

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
 * Both are more moving parts than one timing constant justifies.
 */
const HINT_DEBOUNCE_MS = 50

/**
 * How often the current state is re-reported even though nothing changed.
 *
 * Without this the report is only a *change* notification, and the host expires
 * reports — deliberately, so that a page which dies without a `blur` cannot pin the
 * approval authority to a window nobody is looking at. The two rules together mean
 * a page looked at for longer than `presenceMaxAgeMs` (30s by default) **stops
 * counting as "the user is looking at it"**, and the floating panel starts answering
 * approvals that belong to the window in front of them.
 *
 * A heartbeat turns the report into a *liveness* claim: while the page is alive its
 * state stays fresh, and when it dies the report expires exactly as intended. It must
 * be comfortably shorter than the host's `presenceMaxAgeMs`; five seconds against
 * thirty leaves six missed beats of margin.
 *
 * A hidden page's timers are throttled by the browser, so a hidden page may still
 * expire between beats. That costs nothing where it matters — a hidden page is not a
 * surface the user is looking at, and the panel is supposed to take over there. It
 * can make the `surfaces` list of an `interaction/hint` say "no window can answer
 * this" while a hidden page exists: a wording problem, not an authority problem.
 */
const HEARTBEAT_MS = 5000

/**
 * How often the sidecar's state is re-read while a page shows it.
 *
 * Two seconds is where a start or a stop the user just triggered looks settled by
 * the time their eye returns to the badge, while the page stays a page: one small
 * JSON round trip over a connection the Harness window already holds. The badge
 * and the control each hold their own interval — two cheap reads while the page
 * is open — because neither can decide anything from the other's state: the
 * button has to know whether to offer start or stop, the tag has to know what to
 * say. Both exist only while a component is mounted; nothing here polls in the
 * background.
 */
const STATUS_POLL_MS = 2000

/**
 * The restart budget the badge counts against.
 *
 * `quorfloat/status` reports how many restarts happened but not the budget they
 * are counted against, and this half cannot read the host's config (it is not on
 * the stdio channel), so the number mirrors `restartLimit` in the bundle's patch
 * (`cordis.patch.yml`) and its default in `src/config.ts`. A deployment that
 * configures a different limit shows a wrong denominator — never a wrong
 * numerator, and never a wrong "gave up": those come from the snapshot.
 */
const RESTART_LIMIT = 3

/**
 * One supervisor snapshot, as `quorfloat/status` reports it.
 *
 * `state` is deliberately an open string: it is the supervisor's own vocabulary,
 * and a value this half has no word for is rendered as it arrived rather than
 * folded into a familiar one.
 */
export interface SidecarStatus {
  /** The supervisor's own word for where the process is. */
  readonly state: string
  /** Process id while one exists, else null. */
  readonly pid: number | null
  /** Automatic restarts since this start. */
  readonly restarts: number
  /** Whether the supervisor has used up its restart budget. */
  readonly restartExhausted: boolean
  /** Whether the sidecar has completed its handshake since this start. */
  readonly seen: boolean
  /** Whether the panel window is currently visible. */
  readonly visible: boolean
  /** The failure the supervisor tracked, so far. */
  readonly lastError: string | null
}

/** The words this half has for the supervisor's states, plus an escape hatch. */
export type StatusKey = 'unknown' | 'starting' | 'restarting' | 'running' | 'runningUnseen' | 'stopped' | 'failed' | 'other'

/** One render decision for the badge. */
export interface StatusDescription {
  /** Locale key for the badge's text. */
  readonly key: StatusKey
  /** The supervisor's own word, interpolated when `key` is `other`. */
  readonly state: string
  /** Automatic restarts so far, for the keys that count them. */
  readonly restarts: number
  /** Whether the restart budget is spent, which the badge says in words. */
  readonly exhausted: boolean
  /** The `Tag` tone the state earns. */
  readonly tone: string
  /** The reason to show on hover, when the supervisor reported one. */
  readonly title: string | null
}

/**
 * Read a snapshot out of one RPC answer.
 *
 * Returns null — "nothing to say" — rather than a half-filled snapshot, because
 * `state` is the one field that cannot be defaulted: every label is built from
 * it, and a defaulted word would be this half's guess wearing the supervisor's
 * authority. The remaining fields are read defensively, since the snapshot is
 * data that crossed a process boundary.
 *
 * @param value - the `value` of a successful `quorfloat/status` answer.
 * @returns the snapshot, or null when it carries no usable state.
 */
export function parseStatus(value: unknown): SidecarStatus | null {
  if (typeof value !== 'object' || value === null) return null
  const record = value as Record<string, unknown>
  const state = record['state']
  if (typeof state !== 'string') return null
  const pid = record['pid']
  const restarts = record['restarts']
  const lastError = record['lastError']
  return {
    state,
    pid: typeof pid === 'number' && Number.isFinite(pid) ? pid : null,
    restarts: typeof restarts === 'number' && Number.isFinite(restarts) ? restarts : 0,
    restartExhausted: record['restartExhausted'] === true,
    seen: record['seen'] === true,
    visible: record['visible'] === true,
    lastError: typeof lastError === 'string' ? lastError : null,
  }
}

/**
 * Decide what the badge says.
 *
 * The rule is that the badge never answers a question the snapshot did not
 * answer. A state this half has no word for becomes `other` with the
 * supervisor's own word in it, not the nearest familiar one; a `running`
 * process that has not answered its handshake is not presented as healthy; and
 * a null snapshot (no answer yet, or an answer that could not be read) says so
 * instead of defaulting to "stopped".
 *
 * `lastError` stays out of the text and rides the badge's tooltip: it is the
 * supervisor's reason, often long, and a row of chrome is not the place for a
 * sentence. A sidecar that gave up on restarts is coloured by that fact rather
 * than described by it — the reason is in the tracked failure anyway.
 *
 * @param status - the last snapshot, or null when none could be read.
 * @returns the badge's key, tone, tooltip, and the raw state to interpolate.
 */
export function describeStatus(status: SidecarStatus | null): StatusDescription {
  if (status === null) {
    return { key: 'unknown', state: '', restarts: 0, exhausted: false, tone: 'quiet', title: null }
  }
  const title = status.lastError
  const restarts = status.restarts
  // The budget running out is a fact about the supervisor, not the process: it
  // colours a sidecar that is not answering and leaves a live one as it is. The
  // badge repeats the count, so the colour is never the only carrier of it.
  const exhausted = status.restartExhausted && !status.seen
  const tone = (base: string): string => (exhausted ? 'danger' : base)
  switch (status.state) {
    case 'running':
      if (!status.seen) {
        return { key: 'runningUnseen', state: status.state, restarts, exhausted, tone: tone('warning'), title }
      }
      return { key: 'running', state: status.state, restarts, exhausted, tone: 'success', title }
    case 'starting':
      // A second start is a restart, and a restart is news: it says the process
      // went away and is being brought back, which the bare `starting` hides.
      return restarts > 0
        ? { key: 'restarting', state: status.state, restarts, exhausted, tone: tone('warning'), title }
        : { key: 'starting', state: status.state, restarts, exhausted, tone: tone('info'), title }
    case 'stopped':
      return { key: 'stopped', state: status.state, restarts, exhausted, tone: tone('neutral'), title }
    case 'failed':
      return { key: 'failed', state: status.state, restarts, exhausted, tone: 'danger', title }
    default:
      return { key: 'other', state: status.state, restarts, exhausted, tone: tone('quiet'), title }
  }
}

/**
 * The one action the control offers.
 *
 * Null while no snapshot has been read: an unread state is not evidence that
 * the sidecar is stopped, and offering "start" there would be a guess the user
 * pays for. `seen` takes part because a `running` process that has not answered
 * its handshake — and a `starting` one — is still a process to stop, while a
 * state word this half does not know offers the harmless direction: asking to
 * stop something that is not there does nothing, while asking to start
 * something already up is refused by the supervisor.
 *
 * @param status - the last snapshot, or null when none could be read.
 * @returns the action to offer, or null when nothing should be offered.
 */
export function controlAction(status: SidecarStatus | null): 'start' | 'stop' | null {
  if (status === null) return null
  const up = status.seen || status.state === 'running' || status.state === 'starting'
  return up ? 'stop' : 'start'
}

/**
 * Run `read` now, then every `intervalMs`, until the returned disposer runs.
 *
 * Reads that overlap are dropped rather than queued: a status call that is slow
 * must not make the page accumulate requests behind it. A read that answers with
 * a failure is consumed here so the loop survives it — the reader reports its own
 * failures, and one unanswered poll is not a reason to stop asking.
 *
 * @param read - one read; the caller does not await it.
 * @param intervalMs - gap between reads, defaulting to {@link STATUS_POLL_MS}.
 * @returns the disposer that stops the polling.
 */
export function startStatusPolling(
  read: () => Promise<void>,
  intervalMs: number = STATUS_POLL_MS,
): () => void {
  let inFlight = false
  let stopped = false
  const tick = (): void => {
    if (stopped || inFlight) return
    inFlight = true
    read().then(
      () => { inFlight = false },
      () => { inFlight = false },
    )
  }
  tick()
  const timer = setInterval(tick, intervalMs)
  return () => {
    stopped = true
    clearInterval(timer)
  }
}

/** The two gateway calls the page half makes. */
interface SidecarFace {
  /** The current snapshot, or null when the call could not be answered. */
  read(): Promise<SidecarStatus | null>
  /** Ask for one transition; the snapshot answered, or null on failure. */
  control(action: 'start' | 'stop'): Promise<SidecarStatus | null>
}

/** The logger shape this half uses; named so the face's signature stays readable. */
type ClientLogger = ReturnType<ClientContext['logger']>

/**
 * Build the two gateway calls over one connection.
 *
 * A failed call is a null snapshot, never an exception into React: the caller's
 * only sane response to "the host did not answer" is to say so, and the badge
 * has a word for exactly that.
 *
 * @param connection - the browser connection service.
 * @param log - this half's logger.
 * @returns the read and control calls.
 */
function createSidecarFace(connection: ClientConnection, log: ClientLogger): SidecarFace {
  const ask = async (endpoint: string, args: Record<string, unknown>): Promise<SidecarStatus | null> => {
    try {
      // The arguments are spread: the gateway matches `args` keys against the
      // host method's parameter names one for one, and requires the envelope to
      // hold exactly that one plain-object field.
      const result = await connection.rpc.call(CHANNEL, endpoint, { args })
      if (result.ok !== true) {
        log.debug('sidecar call was refused', { endpoint, error: result.error })
        return null
      }
      return parseStatus(result.value)
    } catch (error) {
      log.warn('could not reach the sidecar gateway', error)
      return null
    }
  }
  return {
    read: () => ask(STATUS_ENDPOINT, {}),
    control: action => ask(PANEL_ENDPOINT, { action }),
  }
}

/**
 * Keep the sidecar's state live for as long as the component asking is mounted.
 *
 * The state starts empty and returns to it if a later read fails: losing the
 * answer loses the claim, which is the honest reading of a gateway that stopped
 * answering.
 *
 * @param sidecar - the gateway calls.
 * @returns the last snapshot and its setter, so a control response can be
 *   adopted without waiting for the next poll.
 */
function useSidecarStatus(sidecar: SidecarFace): [SidecarStatus | null, (next: SidecarStatus | null) => void] {
  const [status, setStatus] = React.useState<SidecarStatus | null>(null)
  React.useEffect(
    () => startStatusPolling(async () => { setStatus(await sidecar.read()) }),
    [],
  )
  return [status, setStatus]
}

/**
 * The sidecar's state plus the one transition a control offers.
 *
 * The state, the offered direction, and the in-flight guard are one decision,
 * shared by every surface that draws the control — the tab's page-sized button
 * and the sidebar detail page's compact one — so the two can never disagree
 * about what a click does or about whether a second one may stack on the first.
 *
 * @param sidecar - the gateway calls.
 * @returns the last snapshot, the action to offer (or null), and the click.
 */
function useSidecarControl(sidecar: SidecarFace): {
  status: SidecarStatus | null
  action: 'start' | 'stop' | null
  pending: boolean
  onClick: () => void
} {
  const [status, setStatus] = useSidecarStatus(sidecar)
  const [pending, setPending] = React.useState(false)
  const action = controlAction(status)
  const onClick = (): void => {
    if (action === null) return
    setPending(true)
    void sidecar.control(action).then(
      next => {
        setPending(false)
        // The host answers a transition with the snapshot it produced, so the
        // page shows the new state at once instead of at the next poll.
        if (next !== null) setStatus(next)
      },
      () => { setPending(false) },
    )
  }
  return { status, action, pending, onClick }
}

/**
 * Compose the state indicator's text from one decision.
 *
 * The spent restart budget is appended as a clause carrying its count rather
 * than left to the tone: colour alone cannot tell "stopped" from "stopped and
 * will not be retried", and the count is the part that says how bad it got.
 *
 * @param description - the decision from {@link describeStatus}.
 * @param t - the locale seat of this half's namespace.
 * @returns the words for the state.
 */
function statusText(description: StatusDescription, t: Translate): string {
  const base = t(description.key, {
    state: description.state,
    restarts: description.restarts,
    limit: RESTART_LIMIT,
  })
  return description.exhausted ? base + t('gaveUp', { restarts: description.restarts }) : base
}

/**
 * The tab body's one row: indicator at the start, action at the end.
 *
 * DSH lays a settings row out this way itself — `ui-settings-session-log`'s
 * upload row is `justify-content: space-between; align-items: center; gap: 24px`
 * — but this hand-rolled bundle carries no CSS-module pipeline
 * (`scripts/build-client.mjs` — "no CSS means no stylesheet pipeline"), so the
 * same three declarations ride the element's own `style`. They are the whole
 * layout: no margin or padding is invented here.
 */
const TAB_ROW_STYLE = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: '24px',
} as const

/**
 * Build the tab entry: this plugin's page inside Settings → Built-in plugins.
 *
 * The returned component is the body, and it is mounted only when the section
 * mounts this tab — on its first selection, and kept mounted after that so the
 * number the user is reading does not blink out of existence when they glance at
 * the inventory.
 *
 * The body is exactly one row, in every state: the state indicator at the start,
 * the action at the end, vertically centred. The indicator leads because it is
 * the answer to the question the user opened the page with, and it carries the
 * supervisor's own reason on its tooltip — the reason is a sentence, and a
 * tooltip is where this page keeps one. The action trails, offering `start` when
 * there is nothing to stop and nothing at all while no snapshot has been read.
 *
 * @param sidecar - the gateway calls.
 * @returns the slot component.
 */
function createSidecarTab(sidecar: SidecarFace): SlotComponent {
  return function SidecarTab({ t }: TabProps): unknown {
    const { status, action, pending, onClick } = useSidecarControl(sidecar)
    const description = describeStatus(status)
    const nodes: unknown[] = [
      // `undefined` leaves the attribute off entirely rather than claiming an empty
      // reason; a state with no tracked failure has nothing to explain.
      h(
        'span',
        { title: description.title ?? undefined },
        h(primitives.Tag, { tone: description.tone }, statusText(description, t)),
      ),
    ]
    // The action is the row's last cell. A state nobody has answered offers none,
    // and the row is then the indicator alone.
    if (action !== null) {
      nodes.push(h(
        primitives.Button,
        {
          // Start is the page's reason to exist — after a quit from the tray menu
          // the hotkey is gone, and this button is the only way back — so it leads
          // as a primary, page-sized control. Stop is the quiet counterpart.
          variant: action === 'start' ? 'primary' : 'outline',
          size: 'md',
          disabled: pending,
          onClick,
        },
        t(action),
      ))
    }
    return h('div', { style: TAB_ROW_STYLE }, ...nodes)
  }
}

/**
 * Build the detail badge: this half's state tag beside a detail page's title.
 *
 * The gate is the outer component and it declares no hooks, so a page that is
 * not this plugin's mounts nothing at all — no status read, and no interval
 * quietly polling on someone else's page — and a page that moves from a foreign
 * subject to ours mounts the body fresh rather than changing its hook count.
 *
 * The body is the tab's own indicator: the same tag, the same words, the same
 * tone, and the supervisor's reason on its tooltip. It is the tag alone because
 * a row of tags beside a title is chrome, not a page: the sidebar page already
 * shows the bundle, its version, and the row that opens this plugin's settings
 * tab one click away.
 *
 * @param sidecar - the gateway calls.
 * @returns the detail-slot component.
 */
function createSidecarBadge(sidecar: SidecarFace): DetailComponent {
  function SidecarBadgeBody({ t }: PluginDetailProps): unknown {
    const [status] = useSidecarStatus(sidecar)
    const description = describeStatus(status)
    return h(
      'span',
      { title: description.title ?? undefined },
      h(primitives.Tag, { tone: description.tone }, statusText(description, t)),
    )
  }
  return function SidecarBadge(props: PluginDetailProps): unknown {
    return ownsDetailSubject(props.subject) ? h(SidecarBadgeBody, props) : null
  }
}

/**
 * Build the detail action: the one start/stop control at a detail page's head.
 *
 * Same gate, same body shape as the badge, and the same control decision as the
 * tab — only the form is smaller, because this control stands in the page's own
 * chrome beside DSH's switch and uninstall rather than on a page of its own.
 * `start` still leads as the primary direction: that is the direction this
 * plugin's state is here to offer. While no snapshot has been read the entry
 * draws nothing, exactly as the tab offers no action from an unread state.
 *
 * @param sidecar - the gateway calls.
 * @returns the detail-slot component.
 */
function createSidecarAction(sidecar: SidecarFace): DetailComponent {
  function SidecarActionBody({ t }: PluginDetailProps): unknown {
    const { action, pending, onClick } = useSidecarControl(sidecar)
    if (action === null) return null
    return h(
      primitives.Button,
      {
        variant: action === 'start' ? 'primary' : 'outline',
        size: 'sm',
        disabled: pending,
        onClick,
      },
      t(action),
    )
  }
  return function SidecarAction(props: PluginDetailProps): unknown {
    return ownsDetailSubject(props.subject) ? h(SidecarActionBody, props) : null
  }
}

/** Which surface this page is. The desktop shell and a plain browser share code. */
function surfaceOf(): 'desktop' | 'web' {
  return typeof (globalThis as { dshDesktop?: unknown }).dshDesktop === 'undefined' ? 'web' : 'desktop'
}

/**
 * Install the presence reporter, then the page half.
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
      const result = await ctx.connection.rpc.call(CHANNEL, PRESENCE_ENDPOINT, {
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

  ctx.effect(() => {
    document.addEventListener('visibilitychange', schedule)
    window.addEventListener('focus', schedule)
    window.addEventListener('blur', schedule)
    // The liveness half of the report. An unchanged state still has to be re-sent,
    // or the host's expiry turns "the user is looking at this window" into "nobody is
    // looking" after half a minute of the user doing exactly that.
    const heartbeat = setInterval(() => void send(), HEARTBEAT_MS)
    return () => {
      document.removeEventListener('visibilitychange', schedule)
      window.removeEventListener('focus', schedule)
      window.removeEventListener('blur', schedule)
      clearInterval(heartbeat)
      if (pending !== undefined) clearTimeout(pending)
    }
  }, 'quorfloat/presence: listeners')

  // Report the opening state too: without it the host would treat a freshly
  // loaded, focused window as unreported until the user first touches it.
  void send()

  const log = ctx.logger('quorfloat/client')
  const sidecar = createSidecarFace(ctx.connection, log)
  // Deferred on this half's own services: a client without the UI renderer or the
  // locale service still reports presence, and simply has nowhere to show the
  // sidecar's state.
  ctx.inject(['slots', 'locale'], scope => {
    // Registered before the entry below, so its label and its `t` seat resolve
    // real copy from the first render instead of falling back to the keys.
    scope.effect(
      () => scope.locale.register(LOCALE_NS, { en, zh }),
      'quorfloat/client: dictionaries',
    )
    // Bound once and called per projection: the section reads `label` through a
    // thunk on every projection and re-projects on a locale revision, so the tab
    // text follows a language switch without re-registering anything.
    const t = scope.locale.bind(LOCALE_NS)
    scope.effect(
      () => scope.slots.inject(TAB_SLOT, () => scope.slots.register(
        {
          name: TAB_SLOT,
          id: ENTRY_ID,
          order: ENTRY_ORDER,
          label: () => t('tab'),
          locale: LOCALE_NS,
        },
        createSidecarTab(sidecar),
      )),
      'quorfloat/client: sidecar tab',
    )
    // The sidebar 插件 page's two detail slots. Each is injected separately
    // because each is declared by the page's own `main` factory and may appear
    // (or not) independently; `label` is the entry's display text where the
    // owner projects one, and here that is the same words as the tab's — these
    // slots never project it, so nothing new needed naming.
    scope.effect(
      () => scope.slots.inject(BADGE_SLOT, () => scope.slots.register(
        {
          name: BADGE_SLOT,
          id: ENTRY_ID,
          order: DETAIL_ORDER,
          label: () => t('tab'),
          locale: LOCALE_NS,
        },
        createSidecarBadge(sidecar),
      )),
      'quorfloat/client: detail badge',
    )
    scope.effect(
      () => scope.slots.inject(ACTIONS_SLOT, () => scope.slots.register(
        {
          name: ACTIONS_SLOT,
          id: ENTRY_ID,
          order: DETAIL_ORDER,
          label: () => t('tab'),
          locale: LOCALE_NS,
        },
        createSidecarAction(sidecar),
      )),
      'quorfloat/client: detail actions',
    )
  })
}

/** Every word the page half renders: one per `StatusKey`, the verbs, the gave-up clause, and the tab. */
type MessageKey = StatusKey | 'start' | 'stop' | 'gaveUp' | 'tab'

/**
 * English copy; `other` is where an unrecognised state is interpolated.
 *
 * `gaveUp` carries its own leading separator: the indicator appends it to
 * whatever the state said, and a separator living in the clause is what keeps
 * the Chinese wording free of a space before a full-width bracket.
 *
 * `tab` is the package name, because the tab configures that package and a
 * package name is not translated. The user-facing noun for the thing the plugin
 * runs is "floating panel" everywhere, never "sidecar".
 */
const en: Record<MessageKey, string> = {
  running: 'Floating panel running',
  runningUnseen: 'Floating panel running (no reply)',
  starting: 'Floating panel starting',
  restarting: 'Floating panel restarting ({restarts}/{limit})',
  stopped: 'Floating panel stopped',
  failed: 'Floating panel failed',
  other: 'Floating panel {state}',
  unknown: 'Floating panel unknown',
  start: 'Start floating panel',
  stop: 'Stop floating panel',
  gaveUp: ' ({restarts} failures in a row)',
  tab: 'dsh-quorfloat',
}

/** Chinese copy, key for key with {@link en}. */
const zh: Record<MessageKey, string> = {
  running: '悬浮窗运行中',
  runningUnseen: '悬浮窗运行中（无应答）',
  starting: '悬浮窗启动中',
  restarting: '悬浮窗重启中 ({restarts}/{limit})',
  stopped: '悬浮窗已停止',
  failed: '悬浮窗失败',
  other: '悬浮窗 {state}',
  unknown: '悬浮窗状态未知',
  start: '启动悬浮窗',
  stop: '停止悬浮窗',
  gaveUp: '（连续 {restarts} 次失败）',
  tab: 'dsh-quorfloat',
}
