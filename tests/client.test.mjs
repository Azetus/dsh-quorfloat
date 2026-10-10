/**
 * Browser-half tests.
 *
 * The artifact under test is the real build output (`lib/client.js`), loaded the
 * way the Harness client loader loads it: through
 * `window.__ModuleLoader__.load({ id, factory })`. Testing the artifact rather
 * than the TypeScript source means the loader wrapper, the export conversion, and
 * the self-containment check are all covered — a source-level test would pass
 * while the published bundle failed to register.
 *
 * The DOM is stubbed because the values that matter here are the two the browser
 * provides (`document.visibilityState`, `document.hasFocus()`) and the events
 * that report their change. What the real values are on each platform was
 * measured separately (`docs/progress.md` §18).
 *
 * The page half is tested through the same artifact. Its React comes from the
 * loader's module table, so the stub table below stands where the shell's frozen
 * seed set stands, and the small hook runtime models the two hooks the half uses
 * (`useState` / `useEffect`) — including the cleanup-on-unmount pair that "poll
 * only while mounted" depends on. React's real semantics for an empty dep array
 * are modelled too: an effect declared with `[]` runs once per mount, not once
 * per render.
 */

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { repoRoot, waitFor } from './helpers.mjs'

const ARTIFACT = join(repoRoot, 'lib/client.js')
const PACKAGE_NAME = 'dsh-quorfloat'

/** A minimal DOM/document pair with controllable visibility and focus. */
function fakeDom({ visibility = 'visible', focused = true } = {}) {
  const documentListeners = new Map()
  const windowListeners = new Map()
  const document = {
    visibilityState: visibility,
    hasFocus: () => focused,
    addEventListener: (type, listener) => {
      const list = documentListeners.get(type) ?? []
      list.push(listener)
      documentListeners.set(type, list)
    },
    removeEventListener: (type, listener) => {
      const list = documentListeners.get(type) ?? []
      const index = list.indexOf(listener)
      if (index >= 0) list.splice(index, 1)
    },
  }
  const window = {
    addEventListener: (type, listener) => {
      const list = windowListeners.get(type) ?? []
      list.push(listener)
      windowListeners.set(type, list)
    },
    removeEventListener: (type, listener) => {
      const list = windowListeners.get(type) ?? []
      const index = list.indexOf(listener)
      if (index >= 0) list.splice(index, 1)
    },
  }
  return {
    document,
    window,
    /** Change the reported state and fire the matching event. */
    change({ visibility: nextVisibility, focused: nextFocused, event }) {
      if (nextVisibility !== undefined) document.visibilityState = nextVisibility
      if (nextFocused !== undefined) focused = nextFocused
      const target = event === 'visibilitychange' ? documentListeners : windowListeners
      for (const listener of target.get(event) ?? []) listener()
    },
    listenerCount: () => documentListeners.size + windowListeners.size,
    documentListeners,
    windowListeners,
  }
}

/**
 * A React stub with just enough runtime to mount one component of the half.
 *
 * `createElement` records the call tree instead of building DOM; `useState` and
 * `useEffect` model one component's hook scope. `begin()` resets the declaration
 * cursors before each render (React requires a stable hook order, and the half
 * declares the same hooks in the same order every time) and `commit()` runs the
 * effects that render declared — the empty dep array an effect uses here is
 * always equal to itself element-wise, which is why it runs once per mount.
 *
 * @returns the stub API plus the render/unmount controls a test drives.
 */
function createReactStub() {
  /** `useState` value per declaration position. */
  const cells = []
  /** Effects as committed: `{ deps, cleanup }` per declaration position. */
  const committed = []
  /** Effects this render declared: `{ body, deps }` per position. */
  let declared = []
  let cursor = 0
  let dirty = false

  const api = {
    createElement: (type, props, ...children) => ({ type, props: props ?? {}, children }),
    useState: (initial) => {
      const index = cursor
      cursor += 1
      if (!(index in cells)) cells[index] = initial
      return [
        cells[index],
        next => {
          cells[index] = next
          dirty = true
        },
      ]
    },
    useEffect: (body, deps) => {
      declared.push({ body, deps })
    },
  }

  return {
    api,
    /** Start one render: React validates the hook order from a clean cursor. */
    begin: () => {
      cursor = 0
      declared = []
    },
    /** Commit the render: run every effect whose deps changed since it last ran. */
    commit: () => {
      for (let index = 0; index < declared.length; index += 1) {
        const { body, deps } = declared[index]
        const previous = committed[index]
        // An empty dep array is element-wise equal to itself, which is why an
        // effect declared with `[]` runs once per mount and not once per render.
        if (previous !== undefined && sameDeps(previous.deps, deps)) continue
        previous?.cleanup?.()
        committed[index] = { deps, cleanup: body() ?? undefined }
      }
    },
    /** Whether a `setState` happened since the last {@link clearDirty}. */
    isDirty: () => dirty,
    clearDirty: () => { dirty = false },
    /** Unmount: run every committed cleanup, as React does. */
    unmount: () => { for (const effect of committed) effect?.cleanup?.() },
  }
}

/**
 * Compare two dependency arrays the way React does.
 *
 * @param a - the deps the effect last ran with.
 * @param b - the deps this render declared.
 * @returns whether the effect may be skipped.
 */
function sameDeps(a, b) {
  if (a === undefined || b === undefined) return false
  if (a.length !== b.length) return false
  return a.every((value, index) => Object.is(value, b[index]))
}

/**
 * Mount one element the way React does: give it a hook scope, then commit.
 *
 * The half's slot components return the gated body as an element rather than
 * rendering it, so the mount is two steps and both are spelled out here instead
 * of hiding behind a renderer this test does not need.
 *
 * @param react - the stub from {@link createReactStub}.
 * @param element - the element the outer component returned.
 * @returns the rendered tree and a re-render that picks up committed state.
 */
function mountElement(react, element) {
  assert.equal(typeof element.type, 'function', 'the entry returned a component to mount')
  const draw = () => {
    react.begin()
    return element.type(element.props)
  }
  const tree = draw()
  react.commit()
  return {
    tree,
    /** Re-render after a state change, then commit whatever it declared. */
    rerender: () => {
      react.clearDirty()
      const next = draw()
      react.commit()
      return next
    },
  }
}

/**
 * Mount one registered slot component the way the renderer does.
 *
 * The slot registry hands an entry the props it declared — for the Settings tab
 * that is the `t` seat of the namespace in its `locale` option, and nothing
 * else, because a `settings.plugins.tab` owner supplies no props of its own.
 *
 * @param react - the stub from {@link createReactStub}.
 * @param component - the component the entry registered.
 * @param props - the props the renderer binds.
 * @returns the rendered tree and its re-render control.
 */
function mountComponent(react, component, props) {
  return mountElement(react, react.api.createElement(component, props))
}

/**
 * The tab body's parts, in order, with the ones it decided not to render dropped.
 *
 * @param tree - the tree {@link mountComponent} returned.
 * @returns the rendered children of the body's root element.
 */
function tabParts(tree) {
  return tree.children.flat().filter(part => part !== null && part !== undefined)
}

/**
 * The state indicator of a mounted tab.
 *
 * @param tree - the tree {@link mountComponent} returned.
 * @returns the tooltip the tag carries and the tag itself.
 */
function stateOf(tree) {
  const wrapper = tabParts(tree)[0]
  return { tooltip: wrapper.props.title, tag: wrapper.children[0] }
}

/**
 * Assert the tab body's one row: the state indicator at the start, the action
 * at the end, and nothing else — no copy line and no reason paragraph.
 *
 * The layout is read off the root element the component returned, so this covers
 * the shape the settings section actually mounts. The style triple is the one
 * DSH's own settings rows use (`justify-content: space-between; align-items:
 * center`), which is also the shape the user asked for: indicator left, action
 * right, vertically centred, in every state.
 *
 * @param tree - the tree {@link mountComponent} returned.
 * @param options.label - the words the state indicator must show.
 * @param options.Tag - the module table's tag atom, to recognise it by.
 * @param options.Button - the module table's button atom, to recognise it by.
 * @param options.action - the words the action must show, or null when the
 *   state offers no action and the row is the indicator alone.
 * @param options.tip - the tooltip the indicator must carry; `undefined` when
 *   the snapshot tracked no reason.
 */
function assertTabRow(tree, { label, Tag, Button, action = null, tip }) {
  assert.equal(tree.type, 'div', 'the body is one container')
  assert.deepEqual(
    { ...tree.props.style },
    { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '24px' },
    'one flex row: the indicator and the action at opposite ends, vertically centred',
  )
  const parts = tabParts(tree)
  assert.equal(parts.length, action === null ? 1 : 2, 'exactly the indicator, and the action when one is offered')
  assert.equal(parts[0].type, 'span', 'the state indicator leads the row')
  assert.equal(parts[0].children[0].type, Tag, 'the indicator is the shared tag')
  assert.equal(parts[0].children[0].children[0], label, 'the indicator says the state the snapshot reports')
  assert.equal(parts[0].props.title, tip, 'the supervisor reason rides the indicator tooltip, not a paragraph')
  assert.ok(!parts.some(part => part.type === 'p'), 'no copy line and no reason paragraph in the body')
  if (action !== null) {
    assert.equal(parts[1].type, Button, 'the action closes the row')
    assert.equal(parts[1].children[0], action, 'the action reads the shared verb copy')
  }
}

/**
 * The action a mounted tab offers.
 *
 * @param tree - the tree {@link mountComponent} returned.
 * @param Button - the module table's button atom, to recognise it by.
 * @returns the button element, or undefined when no action is offered.
 */
function buttonOf(tree, Button) {
  return tabParts(tree).find(part => part.type === Button)
}

/**
 * Render one `plugins.detail.*` entry the way its owner does.
 *
 * Unlike the tab, a detail entry is a gate and a body: the outer component
 * answers the page's `subject` and either returns null or returns the body to
 * mount. The gate declares no hooks, so it is called directly and the body is
 * mounted as its own component — the shape React needs to keep hook order
 * stable when the page moves from someone else's subject to ours, and the shape
 * that keeps a foreign page from even starting this half's poll.
 *
 * @param react - the stub from {@link createReactStub}.
 * @param component - the component the entry registered.
 * @param props - the props the page binds: its `subject` and the locale seat.
 * @returns the mounted body, or null when the entry declined the subject.
 */
function mountDetail(react, component, props) {
  react.begin()
  const gated = component(props)
  react.commit()
  if (gated === null || gated === undefined) return null
  return mountElement(react, gated)
}

/**
 * The state tag a mounted detail badge drew.
 *
 * The badge is chrome — one tag on its own — so the root is the tag's wrapper
 * and carries the supervisor's reason as its tooltip, exactly as the tab's state
 * line does.
 *
 * @param tree - the tree {@link mountDetail} returned.
 * @returns the wrapper's tooltip and the tag inside it.
 */
function badgeOf(tree) {
  assert.ok(tree !== null, 'the badge rendered')
  return { tooltip: tree.props.title, tag: tree.children.flat().filter(part => part !== null && part !== undefined)[0] }
}

/**
 * The action a mounted detail entry drew, when it drew one at all.
 *
 * @param tree - the tree {@link mountDetail} returned.
 * @param Button - the module table's button atom, to recognise it by.
 * @returns the button element, or null when the entry drew nothing.
 */
function detailButtonOf(tree, Button) {
  if (tree === null) return null
  return tree.type === Button ? tree : null
}

/**
 * The two module-table entries this half reads, plus the stub a test mounts with.
 *
 * The shell's real table is a frozen set of platform singletons; the half is
 * only allowed to ask for members of it, which is what makes the artifact's
 * `require` calls safe in a page that has no `node_modules`.
 *
 * @returns the module table and the React stub behind it.
 */
function seedTable() {
  const react = createReactStub()
  return {
    react,
    modules: {
      react: react.api,
      '@deepseek-ai/dsh-client-ui-primitives': {
        Tag: props => ({ kind: 'Tag', props }),
        Button: props => ({ kind: 'Button', props }),
      },
    },
  }
}

/**
 * Load the built artifact through a stub module loader.
 *
 * @param dom - the stubbed DOM the artifact will observe.
 * @param modules - the module table the factory's `require` answers with; the
 *   real table is the shell's frozen seed set, and the artifact must ask for
 *   nothing outside it.
 * @returns the module namespace the factory produced.
 */
function loadArtifact(dom, modules = seedTable().modules) {
  const loaders = []
  /** Contexts created while this build is loaded, so teardown can unload them. */
  const contexts = []
  // The stub must BE the global, not a copy of it: the artifact registers its
  // listeners on `window`, and a spread would leave the test inspecting a
  // different object than the plugin used.
  Object.assign(dom.window, { __ModuleLoader__: { load: entry => { loaders.push(entry) } } })
  const previousWindow = globalThis.window
  const previousDocument = globalThis.document
  globalThis.window = dom.window
  globalThis.document = dom.document
  const source = readFileSync(ARTIFACT, 'utf8')
  // The artifact is a script that calls the loader; evaluate it as one.
  new Function(`${source}`)()
  const registered = loaders[0]
  assert.ok(registered !== undefined, 'the artifact registered a module')
  assert.equal(registered.id, PACKAGE_NAME)
  const restore = () => {
    globalThis.window = previousWindow
    globalThis.document = previousDocument
  }
  activeLoad = contexts
  return {
    module: registered.factory(specifier => {
      if (Object.hasOwn(modules, specifier)) return modules[specifier]
      throw new Error(`the browser half required "${specifier}", which the module table does not serve`)
    }),
    /**
     * Unload the plugin, then put the globals back — in that order.
     *
     * The half installs a liveness interval, and an interval nobody clears keeps the
     * process alive: the runner would sit waiting on the event loop and this file
     * would time out rather than finish. Disposing needs the DOM globals, so restoring
     * them first would make the disposer throw instead of clean up.
     */
    unload: () => {
      for (const ctx of contexts.splice(0)) {
        for (const dispose of ctx.effects) dispose()
      }
      restore()
      activeLoad = null
    },
  }
}

/** The context list of the build currently loaded, if any. */
let activeLoad = null

/** A client context recording what the half reports, serving what it renders with. */
function fakeContext() {
  const calls = []
  const logger = { info: () => {}, warn: () => {}, debug: () => {} }
  const effects = []
  const ctx = {
    calls,
    effects,
    /** Deferred service callbacks `ctx.inject` recorded. */
    injections: [],
    /** `{ key, dispose }` per `slots.inject`, so a test can withdraw one entry. */
    injected: [],
    /** `{ options, component }` per `slots.register`, in registration order. */
    registered: [],
    /** `{ namespace, dictionaries }` per `locale.register`. */
    dictionaries: [],
    /** What `quorfloat/status` answers; tests replace it to drive the tab. */
    snapshot: {
      state: 'running',
      pid: 4242,
      restarts: 0,
      restartExhausted: false,
      seen: true,
      visible: false,
      lastError: null,
    },
    /** The actions `quorfloat/panel` was asked for, in order. */
    actions: [],
    connection: {
      rpc: {
        async call(channel, endpoint, payload) {
          calls.push({ channel, endpoint, payload })
          if (endpoint === 'quorfloat/status') return { ok: true, value: ctx.snapshot }
          if (endpoint === 'quorfloat/panel') {
            const action = payload.args.action
            ctx.actions.push(action)
            // The contract: a transition answers with the snapshot it produced.
            const started = action === 'start'
            return { ok: true, value: { ...ctx.snapshot, state: started ? 'running' : 'stopped', seen: started } }
          }
          return { ok: true, value: { accepted: true } }
        },
      },
    },
    logger: () => logger,
    // Cordis runs the effect body and keeps the disposer it returns, so the fake
    // must do the same: recording the body instead would make "unload" a no-op
    // and any listener leak invisible.
    effect: (body) => {
      const dispose = body()
      effects.push(dispose)
      return () => {}
    },
    // The real `inject` runs the callback once the named services exist; keeping
    // it deferred is what lets a test prove nothing is registered before then.
    inject: (services, callback) => {
      ctx.injections.push({ services, callback })
      return () => {}
    },
    slots: {
      inject: (key, callback) => {
        const dispose = callback()
        ctx.injected.push({ key, dispose })
        return () => dispose()
      },
      register: (options, component) => {
        ctx.registered.push({ options, component })
        return () => {}
      },
    },
    locale: {
      /**
       * The language the bound `t` resolves against. A test flips it exactly
       * like the user switching language: the real service reads its active
       * snapshot per call, which is why the section re-projects tab labels on
       * a locale revision bump instead of re-registering.
       */
      language: 'en',
      /**
       * Bind a namespace the way the locale service does: the returned `t`
       * reads whatever language is active at call time, so a label thunk
       * written against it follows a locale switch with no re-registration.
       */
      bind: namespace => (key, params = {}) => {
        const entry = ctx.dictionaries.find(item => item.namespace === namespace)
        return translateWith(entry?.dictionaries[ctx.locale.language] ?? {})(key, params)
      },
      register: (namespace, dictionaries) => {
        ctx.dictionaries.push({ namespace, dictionaries })
        return () => {}
      },
    },
    /** Make the deferred services available, as Cordis does once they appear. */
    provideServices: () => {
      for (const injection of ctx.injections) injection.callback(ctx)
      ctx.injections = []
    },
    /** The registered component for one slot key, or undefined. */
    componentFor: key => ctx.registered.find(entry => entry.options.name === key)?.component,
  }
  // Registered with the active load so teardown can unload it; a context built
  // outside a load is simply not tracked.
  activeLoad?.push(ctx)
  return ctx
}

/**
 * Let queued promises run.
 *
 * Used instead of `waitFor` wherever fake timers are enabled: `waitFor` sleeps on a
 * real timer, and a mocked one would never fire.
 */
async function settle() {
  for (let turn = 0; turn < 8; turn += 1) await Promise.resolve()
}

/** Read the args of the nth report. */
function argsOf(call) {
  return call.payload.args
}

test('the artifact registers itself and exports the plugin contract', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    assert.deepEqual(loaded.module.inject, ['connection'])
    assert.equal(typeof loaded.module.apply, 'function')
  } finally {
    loaded.unload()
  }
})

/**
 * The tab: the half's own page inside Settings → Built-in plugins.
 *
 * The section renders one entry per registration, keyed by the registration's
 * own `id` (`{ only: row.id }` at
 * `ui-settings-plugins/…/PluginsSettingsSection.tsx`). It stays as the fallback
 * for a composition that has no 插件 page to show a detail entry on — a
 * patch-injected plugin never gets a subject there, and even an installed
 * bundle can fail to render a contribution.
 */
const TAB_SLOT = 'settings.plugins.tab'

/**
 * The two slots the sidebar 插件 page declares on every detail view.
 *
 * `ui-plugin-manager` declares both as root-scoped list slots while its `main`
 * panel is mounted, and renders every entry with the open page's `subject`:
 * `{ kind: 'bundle', pkg }` for a package's page, `{ kind: 'row', pkg, row }`
 * for a row's page, and `{ kind: 'item', id }` for an official plugin's page
 * (`slot-contract.ts`, and `PluginManagerPage.tsx` builds each shape). The page
 * renders these for every plugin, so an entry must return nothing for any
 * subject that is not this plugin's.
 */
const BADGE_SLOT = 'plugins.detail.badge'
const ACTIONS_SLOT = 'plugins.detail.actions'

/** Both detail slots, in registration order. */
const DETAIL_SLOTS = [BADGE_SLOT, ACTIONS_SLOT]

/**
 * The row this bundle carries, as the bundle page's 包含的组件 list shows it:
 * the row's id is `quorfloat` and its module is the package name.
 */
const OUR_ROW = { rowId: 'quorfloat', moduleName: PACKAGE_NAME, enabled: true }

/** A bundle subject for this plugin, with the page's own package shape. */
function ourBundleSubject(overrides = {}) {
  return {
    kind: 'bundle',
    pkg: {
      name: PACKAGE_NAME,
      version: '0.0.1',
      installed: true,
      enabled: true,
      rows: [OUR_ROW],
      ...overrides,
    },
  }
}

/** A row subject for this plugin's row. */
function ourRowSubject() {
  return { kind: 'row', pkg: { name: PACKAGE_NAME, rows: [OUR_ROW] }, row: OUR_ROW }
}

/** Another installed bundle's page, which must not draw this half's chrome. */
function foreignBundleSubject() {
  return {
    kind: 'bundle',
    pkg: {
      name: 'dsh-someone-else',
      version: '1.2.3',
      installed: true,
      enabled: true,
      rows: [{ rowId: 'other', moduleName: 'dsh-someone-else', enabled: true }],
    },
  }
}

/** Another bundle's row page, which must not draw this half's chrome either. */
function foreignRowSubject() {
  return {
    kind: 'row',
    pkg: { name: 'dsh-someone-else' },
    row: { rowId: 'other', moduleName: 'dsh-someone-else', enabled: true },
  }
}

/**
 * Subjects that must render nothing, whatever else they carry.
 *
 * The first group is other plugins' pages and the page shapes this half reads
 * no address out of (an official plugin's `item`, and a kind that does not
 * exist yet). The second is malformed data: the subject crossed a process and a
 * store boundary, and a missing `pkg` or `row` must not become a crash or, worse,
 * a claim on someone else's page.
 */
const NOT_OUR_SUBJECTS = [
  ['another bundle', foreignBundleSubject()],
  ['another row', foreignRowSubject()],
  ['an official plugin', { kind: 'item', id: 'quorfloat' }],
  ['an unknown kind', { kind: 'gadget', pkg: { name: PACKAGE_NAME } }],
  ['a kind with no name', { kind: 'bundle' }],
  ['a bundle with no package name', { kind: 'bundle', pkg: {} }],
  ['a bundle whose name merely starts with ours', { kind: 'bundle', pkg: { name: 'dsh-quorfloat-extra' } }],
  ['a row with no row ref', { kind: 'row', pkg: { name: PACKAGE_NAME } }],
  ['a row with no module name', { kind: 'row', row: { rowId: 'quorfloat' } }],
  [
    'a row of this bundle that is not this row',
    { kind: 'row', pkg: { name: PACKAGE_NAME }, row: { rowId: 'other', moduleName: 'dsh-other' } },
  ],
  ['no subject at all', undefined],
  ['a null subject', null],
  ['a string subject', PACKAGE_NAME],
  ['a number subject', 42],
  ['an empty object', {}],
]

/** A snapshot with every field, for the pure decisions. */
function snapshot(overrides = {}) {
  return {
    state: 'running',
    pid: 4242,
    restarts: 0,
    restartExhausted: false,
    seen: true,
    visible: false,
    lastError: null,
    ...overrides,
  }
}

/** Mount the page half's services and return the context, as an opened page does. */
function pageHalf(loaded) {
  const ctx = fakeContext()
  loaded.module.apply(ctx)
  ctx.provideServices()
  return ctx
}

/**
 * Interpolate one registered dictionary the way the locale service does.
 *
 * Asserting through this rather than through a stub `t` means the expectation is
 * the string a user reads, and the copy under test is the copy the half itself
 * registered — one source, no fixture to drift.
 *
 * @param dictionary - one locale's registered copy.
 * @returns the translate function for it.
 */
function translateWith(dictionary) {
  return (key, params = {}) => (dictionary[key] ?? key)
    .replace(/\{(\w+)\}/g, (whole, name) => (name in params ? String(params[name]) : whole))
}

test('the page half registers nothing until its services exist, then the tab and the two detail entries', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    // Deferred rather than declared in `inject`: a composition without the UI
    // renderer still reports presence, and simply has nothing to render into.
    assert.deepEqual(ctx.injections.map(entry => entry.services), [['slots', 'locale']])
    assert.equal(ctx.registered.length, 0, 'nothing is registered before those services appear')

    ctx.provideServices()
    // The tab plus one entry per detail slot: a second tab would be a second
    // section panel (the shipped inventory already owns `id: 'all'`), and the
    // detail slots are separate cells, so the same entry id is not a collision.
    assert.deepEqual(ctx.registered.map(entry => entry.options.name), [TAB_SLOT, ...DETAIL_SLOTS])
    const entry = ctx.registered[0]
    assert.equal(entry.options.id, 'quorfloat', 'the id the section filters its panel by')
    assert.equal(entry.options.order, 20, 'after the shipped inventory tab, which is order 10')
    assert.equal(entry.options.locale, 'quorfloat.sidecar', 'the copy namespace the t seat resolves')
    assert.equal(typeof entry.component, 'function')

    // Registered before the entries, so the label and the first render resolve
    // real copy. The label is a thunk — `resolveSlotLabel` calls it on every
    // projection — so re-reading it is what the section does per projection.
    assert.deepEqual(ctx.dictionaries.map(item => item.namespace), ['quorfloat.sidecar'])
    assert.deepEqual(Object.keys(ctx.dictionaries[0].dictionaries).sort(), ['en', 'zh'])
    const copy = ctx.dictionaries[0].dictionaries
    assert.equal(typeof entry.options.label, 'function', 'the tab label follows the active locale')
    // The label is the package name: it says which package the page configures,
    // and a package name is not translated, so both languages read the same.
    assert.equal(copy.zh.tab, PACKAGE_NAME, 'the Chinese label is the package name')
    assert.equal(copy.en.tab, PACKAGE_NAME, 'and so is the English one')
    ctx.locale.language = 'zh'
    assert.equal(entry.options.label(), PACKAGE_NAME)
    ctx.locale.language = 'en'
    assert.equal(entry.options.label(), PACKAGE_NAME)
  } finally {
    loaded.unload()
  }
})

test('the detail entries carry this half id, an order, and the locale seat, in both slots', () => {
  // The sidebar 插件 page renders these slots on EVERY plugin's page, so an
  // entry is never optional chrome: it is addressed by its own id (a fresh id
  // joins the shipped entries; a reused one would replace them), ordered after
  // the page's own tags, and given `t` from the namespace it declares.
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    for (const slot of DETAIL_SLOTS) {
      const entry = ctx.registered.find(item => item.options.name === slot)
      assert.ok(entry !== undefined, `${slot} has an entry`)
      assert.equal(entry.options.id, 'quorfloat', `${slot} registers under this half's own id`)
      assert.equal(entry.options.order, 20, `${slot} sits after the owner's own entries`)
      assert.equal(entry.options.locale, 'quorfloat.sidecar', `${slot} declares the copy namespace`)
      assert.equal(typeof entry.component, 'function')
      assert.equal(typeof entry.options.label, 'function', `${slot} label follows the active locale`)
      ctx.locale.language = 'zh'
      assert.equal(entry.options.label(), copy.zh.tab)
      ctx.locale.language = 'en'
      assert.equal(entry.options.label(), copy.en.tab)
    }
  } finally {
    loaded.unload()
  }
})

test('a detail entry draws on this bundle and on this row, and on nothing else', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = pageHalf(loaded)
    const t = key => `<${key}>`
    // The two subjects that are this plugin's: the bundle page (its package
    // name) and the row page (its row's module name). A bundle subject is ours
    // even without the optional facts, which is what a defensively read subject
    // looks like.
    const OURS = [
      ['the bundle page', ourBundleSubject()],
      ['the bundle page without optional facts', { kind: 'bundle', pkg: { name: PACKAGE_NAME } }],
      ['the row page', ourRowSubject()],
    ]
    for (const slot of DETAIL_SLOTS) {
      const component = ctx.componentFor(slot)
      assert.equal(typeof component, 'function', `${slot} has a component`)
      for (const [label, subject] of OURS) {
        const drawn = component({ subject, t })
        assert.notEqual(drawn, null, `${slot} draws on ${label}`)
        assert.notEqual(drawn, undefined, `${slot} draws on ${label}`)
        assert.equal(typeof drawn.type, 'function', `${slot} hands the owner a component to mount for ${label}`)
      }
      for (const [label, subject] of NOT_OUR_SUBJECTS) {
        assert.equal(component({ subject, t }), null, `${slot} draws nothing for ${label}`)
      }
    }
  } finally {
    loaded.unload()
  }
})

test('the badge on our page shows the state with its count and reason, and no panel', async (t) => {
  t.mock.timers.enable({ apis: ['setInterval'], now: 0 })
  const { react, modules } = seedTable()
  const { Tag } = modules['@deepseek-ai/dsh-client-ui-primitives']
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    // The state the tab's stopped line exists for, carrying the supervisor's
    // reason and a spent restart budget.
    ctx.snapshot = snapshot({
      state: 'stopped',
      seen: false,
      restarts: 3,
      restartExhausted: true,
      pid: null,
      lastError: 'quorfloat exited 3 times',
    })
    const mounted = mountDetail(react, ctx.componentFor(BADGE_SLOT), {
      subject: ourBundleSubject(),
      t: translateWith(copy.zh),
    })
    await settle()
    const tree = mounted.rerender()

    // The badge is one tag, and it uses the tab's vocabulary: the same state
    // word, the same appended count, the same tone, the same tooltip.
    const { tooltip, tag } = badgeOf(tree)
    assert.equal(tag.type, Tag)
    assert.equal(tag.props.tone, 'danger')
    assert.equal(tag.children[0], '悬浮窗已停止（连续 3 次失败）', 'the badge shares the state words, with the new noun')
    assert.equal(tooltip, 'quorfloat exited 3 times', 'the reason rides the tag, never invented')
    // And it stays chrome: one tag, with the reason on its tooltip, in a row of
    // tags on a page that already shows the plugin's own details.
    assert.equal(tree.children.flat().filter(part => part !== null && part !== undefined).length, 1)
  } finally {
    react.unmount()
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('the actions entry offers one start/stop control and sends the transition it offers', async () => {
  const { react, modules } = seedTable()
  const { Button } = modules['@deepseek-ai/dsh-client-ui-primitives']
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    ctx.snapshot = snapshot({ state: 'stopped', seen: false, pid: null })
    // Mounted on the row page: the second subject kind has to reach the same body.
    const mounted = mountDetail(react, ctx.componentFor(ACTIONS_SLOT), {
      subject: ourRowSubject(),
      t: translateWith(copy.zh),
    })
    await settle()
    const stopped = mounted.rerender()

    const offered = detailButtonOf(stopped, Button)
    assert.ok(offered !== null, 'the action is a single control, not a panel')
    assert.equal(offered.children[0], '启动悬浮窗', 'the detail control reads the new noun too')
    assert.equal(offered.children[0], copy.zh.start, 'and it is the shared verb copy, not a second string')
    assert.equal(offered.props.variant, 'primary', 'start is the load-bearing direction here too')
    assert.equal(offered.props.size, 'sm', 'page chrome, not a page-sized control')
    assert.equal(offered.props.disabled, false)

    offered.props.onClick()
    // In flight, the control refuses a second click — the tab's own rule.
    assert.equal(detailButtonOf(mounted.rerender(), Button).props.disabled, true)
    await settle()
    assert.deepEqual(ctx.actions, ['start'])
    const call = ctx.calls.find(entry => entry.endpoint === 'quorfloat/panel')
    assert.deepEqual(call.payload, { args: { action: 'start' } }, 'the gateway envelope, verbatim')

    const started = mounted.rerender()
    const rest = detailButtonOf(started, Button)
    assert.equal(rest.children[0], '停止悬浮窗', 'the answered snapshot becomes the offered direction, with the new noun')
    assert.equal(rest.children[0], copy.zh.stop, 'and it is the shared verb copy')
    assert.equal(rest.props.variant, 'outline')
    assert.equal(rest.props.disabled, false)
  } finally {
    react.unmount()
    loaded.unload()
  }
})

test('a detail entry that declined the subject never starts polling', async (t) => {
  // The gate runs before the body's hooks, so a foreign page's subject mounts
  // nothing: no status read, no interval, no claim on someone else's page.
  t.mock.timers.enable({ apis: ['setInterval'], now: 0 })
  const { react, modules } = seedTable()
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const statusCalls = () => ctx.calls.filter(call => call.endpoint === 'quorfloat/status')
    for (const slot of DETAIL_SLOTS) {
      assert.equal(
        mountDetail(react, ctx.componentFor(slot), { subject: foreignBundleSubject(), t: key => key }),
        null,
        `${slot} declined another bundle's page`,
      )
    }
    assert.equal(statusCalls().length, 0, 'a declined subject reads nothing')
    t.mock.timers.tick(60_000)
    await settle()
    assert.equal(statusCalls().length, 0, 'and schedules nothing')
  } finally {
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('a snapshot is read as it arrives, and an unusable one is not invented', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const { parseStatus } = loaded.module
    // Field-for-field: `pid: null` stays null and a null reason stays null.
    assert.deepEqual(parseStatus(snapshot({ pid: null, lastError: null })), snapshot({ pid: null, lastError: null }))
    // Junk fields fall back rather than throwing; `state` is the one that cannot.
    assert.deepEqual(parseStatus({ state: 'backing-off', pid: 'nope', seen: 1 }), {
      state: 'backing-off',
      pid: null,
      restarts: 0,
      restartExhausted: false,
      seen: false,
      visible: false,
      lastError: null,
    })
    assert.equal(parseStatus({ pid: 1 }), null, 'a snapshot with no state says nothing')
    assert.equal(parseStatus(null), null)
    assert.equal(parseStatus('running'), null)
  } finally {
    loaded.unload()
  }
})

test('the state indicator has a word for the states it knows and repeats the ones it does not', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const { describeStatus } = loaded.module
    assert.equal(describeStatus(null).key, 'unknown', 'no answer is not "stopped"')
    assert.equal(describeStatus(snapshot({ state: 'starting' })).key, 'starting')
    assert.equal(describeStatus(snapshot({ state: 'starting' })).tone, 'info')
    assert.equal(describeStatus(snapshot({ state: 'running', seen: true })).key, 'running')
    assert.equal(describeStatus(snapshot({ state: 'running', seen: true })).tone, 'success')
    assert.equal(describeStatus(snapshot({ state: 'running', seen: false })).key, 'runningUnseen', 'up but not answering is not healthy')
    assert.equal(describeStatus(snapshot({ state: 'running', seen: false })).tone, 'warning')
    assert.equal(describeStatus(snapshot({ state: 'stopped', seen: false })).key, 'stopped')
    assert.equal(describeStatus(snapshot({ state: 'failed', seen: false })).tone, 'danger')

    // A second start is a restart, and the count is the part that says how bad
    // it got; the first start stays a plain start.
    const firstStart = describeStatus(snapshot({ state: 'starting', seen: false, restarts: 0 }))
    assert.equal(firstStart.key, 'starting')
    assert.equal(firstStart.restarts, 0)
    const restarting = describeStatus(snapshot({ state: 'starting', seen: false, restarts: 2 }))
    assert.equal(restarting.key, 'restarting')
    assert.equal(restarting.restarts, 2)
    assert.equal(restarting.exhausted, false)

    // The open vocabulary: an unknown word is carried through, not mapped to the
    // nearest familiar one.
    const unknown = describeStatus(snapshot({ state: 'backing-off', seen: false }))
    assert.equal(unknown.key, 'other')
    assert.equal(unknown.state, 'backing-off')

    // The supervisor's reason rides the tooltip; absent stays absent.
    assert.equal(describeStatus(snapshot({ state: 'stopped', seen: false, lastError: 'spawn ENOENT' })).title, 'spawn ENOENT')
    assert.equal(describeStatus(snapshot({ state: 'stopped', seen: false })).title, null)

    // The spent restart budget is a decision of its own — carried by a count in
    // words, not by the tone — and it does not contradict a live sidecar.
    const spent = describeStatus(snapshot({ state: 'stopped', seen: false, restarts: 3, restartExhausted: true }))
    assert.equal(spent.key, 'stopped')
    assert.equal(spent.exhausted, true)
    assert.equal(spent.restarts, 3)
    assert.equal(spent.tone, 'danger')
    assert.equal(describeStatus(snapshot({ state: 'failed', seen: false, restarts: 3, restartExhausted: true })).exhausted, true)
    assert.equal(describeStatus(snapshot({ state: 'running', seen: true, restartExhausted: true })).tone, 'success')
    assert.equal(describeStatus(snapshot({ state: 'running', seen: true, restartExhausted: true })).exhausted, false)
  } finally {
    loaded.unload()
  }
})

test('the control offers the transition that applies, and nothing before a read', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const { controlAction } = loaded.module
    assert.equal(controlAction(null), null, 'an unread state is not evidence of "stopped"')
    assert.equal(controlAction(snapshot({ state: 'stopped', seen: false })), 'start')
    assert.equal(controlAction(snapshot({ state: 'failed', seen: false })), 'start')
    assert.equal(controlAction(snapshot({ state: 'running', seen: true })), 'stop')
    assert.equal(controlAction(snapshot({ state: 'running', seen: false })), 'stop', 'a process that has not answered is still a process')
    assert.equal(controlAction(snapshot({ state: 'starting', seen: false })), 'stop')
    assert.equal(controlAction(snapshot({ state: 'unheard-of', seen: false })), 'start')
  } finally {
    loaded.unload()
  }
})

test('every word the page can render resolves in both languages, and the two agree key for key', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    assert.deepEqual(
      Object.keys(copy.zh).sort(),
      Object.keys(copy.en).sort(),
      'a key in one language and not the other renders as the key itself to whoever reads that language',
    )
    // Every key the page can actually reach: each state word `describeStatus`
    // can decide on (including `other`, whose raw state is interpolated), the
    // spent-budget clause it can append, the two verbs, and the tab label. A
    // missing one renders as its own key name — the failure is invisible in code
    // and obvious on screen.
    const { describeStatus } = loaded.module
    const states = ['running', 'starting', 'stopped', 'failed', 'degraded', 'stopping', 'backing-off']
    const keys = new Set([describeStatus(null).key, 'start', 'stop', 'gaveUp', 'tab'])
    for (const state of states) {
      for (const seen of [false, true]) keys.add(describeStatus(snapshot({ state, seen })).key)
    }
    for (const language of ['en', 'zh']) {
      for (const key of keys) {
        const text = translateWith(copy[language])(key)
        assert.ok(typeof text === 'string' && text.length > 0, `${language}.${key} has copy`)
        assert.notEqual(text, key, `${language}.${key} has copy, not its own name`)
      }
    }
    // The tab names the package it configures, and a package name is not
    // translated: the label reads the same in both languages.
    assert.equal(copy.zh.tab, 'dsh-quorfloat')
    assert.equal(copy.en.tab, 'dsh-quorfloat')
    // The copy lines the tab used to render under the indicator, and the reason
    // paragraph it used to render below them, are deleted rather than left as
    // keys nobody reads: the tab body is the indicator and the action only.
    for (const language of ['en', 'zh']) {
      for (const key of ['runningLine', 'stoppedLine', 'reason']) {
        assert.ok(!(key in copy[language]), `${language}.${key} was deleted from the dictionary`)
      }
    }
    // The noun is 悬浮窗 / "floating panel" in every word the user reads about
    // the thing — the states and the two verbs.
    const named = ['running', 'runningUnseen', 'starting', 'restarting', 'stopped', 'failed', 'other', 'unknown', 'start', 'stop']
    for (const key of named) {
      assert.ok(copy.zh[key].includes('悬浮窗'), `zh.${key} names the floating panel`)
      assert.ok(/floating panel/i.test(copy.en[key]), `en.${key} names the floating panel`)
    }
    // And the retired noun is gone from every string, not only the ones named
    // above: the counts and the states stay, the name of the thing does not.
    for (const language of ['en', 'zh']) {
      for (const [key, text] of Object.entries(copy[language])) {
        assert.ok(!text.includes('侧车'), `${language}.${key} no longer says 侧车`)
        assert.ok(!/sidecar/i.test(text), `${language}.${key} no longer says sidecar`)
      }
    }
  } finally {
    loaded.unload()
  }
})

test('the copy-line decision is gone from the artifact, not left behind as dead code', () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    // The tab body is the indicator and the action only, so nothing decides on a
    // line of copy any more: the export the line was read from is deleted.
    assert.equal(loaded.module.stateLineKey, undefined, 'the artifact no longer exports the line decision')
  } finally {
    loaded.unload()
  }
})

test('polling reads at once, keeps its own cadence, and drops a read still in flight', async (t) => {
  t.mock.timers.enable({ apis: ['setInterval'] })
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const { startStatusPolling } = loaded.module
    let reads = 0
    const stop = startStatusPolling(async () => { reads += 1 }, 2000)
    await settle()
    assert.equal(reads, 1, 'the first read does not wait for the first interval')
    // One interval at a time, each followed by a microtask checkpoint: mock
    // timers fire synchronously, and a read that has not settled yet is
    // correctly dropped by the in-flight guard — real time never stacks ticks.
    t.mock.timers.tick(2000)
    await settle()
    assert.equal(reads, 2)
    t.mock.timers.tick(2000)
    await settle()
    assert.equal(reads, 3)
    t.mock.timers.tick(2000)
    await settle()
    assert.equal(reads, 4)
    stop()
    t.mock.timers.tick(20_000)
    await settle()
    assert.equal(reads, 4, 'a stopped poller is silent')

    // A slow read must not make the page accumulate requests behind it.
    const pending = []
    const slow = startStatusPolling(() => new Promise(resolve => { pending.push(resolve) }), 1000)
    await settle()
    assert.equal(pending.length, 1)
    t.mock.timers.tick(3000)
    await settle()
    assert.equal(pending.length, 1, 'the tick is dropped, not queued')
    pending[0]()
    await settle()
    t.mock.timers.tick(1000)
    await settle()
    assert.equal(pending.length, 2, 'and the cadence resumes once it answers')
    slow()
  } finally {
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('nothing polls until the tab is mounted, and unmounting stops it', async (t) => {
  t.mock.timers.enable({ apis: ['setInterval'], now: 0 })
  const { react, modules } = seedTable()
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const statusCalls = () => ctx.calls.filter(call => call.endpoint === 'quorfloat/status')
    // The section mounts a tab on its first selection, so a user who never opens
    // this tab never makes this half ask anything at all.
    assert.equal(statusCalls().length, 0, 'registering the tab does not poll')

    const component = ctx.componentFor(TAB_SLOT)
    assert.equal(typeof component, 'function', 'the tab slot has an entry')
    const mounted = mountComponent(react, component, { t: key => `<${key}>` })
    await settle()
    assert.equal(statusCalls().length, 1, 'mounting reads the state once')
    // The gateway requires exactly one plain-object `args` field, and the host
    // method takes no parameters.
    assert.deepEqual(statusCalls()[0].payload, { args: {} })
    assert.equal(statusCalls()[0].channel, '/api')

    t.mock.timers.tick(2000)
    await settle()
    assert.equal(statusCalls().length, 2, 'and keeps reading on its cadence')
    assert.ok(mounted.tree !== undefined)

    react.unmount()
    const reported = statusCalls().length
    t.mock.timers.tick(60_000)
    await settle()
    assert.equal(statusCalls().length, reported, 'an unmounted tab is silent')
  } finally {
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('the tab shows the state it read, and carries the reason on the indicator tooltip', async (t) => {
  t.mock.timers.enable({ apis: ['setInterval'], now: 0 })
  const { react, modules } = seedTable()
  const { Button, Tag } = modules['@deepseek-ai/dsh-client-ui-primitives']
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    ctx.snapshot = snapshot({ state: 'running', seen: false, lastError: 'handshake timed out' })
    const mounted = mountComponent(react, ctx.componentFor(TAB_SLOT), { t: translateWith(copy.zh) })

    // Before the first read answers there is nothing to claim and nothing to offer:
    // the row is the indicator alone, and no line of copy appears under it.
    assert.equal(stateOf(mounted.tree).tag.props.tone, 'quiet')
    assertTabRow(mounted.tree, { label: copy.zh.unknown, Tag, Button, tip: undefined })

    await settle()
    assert.equal(react.isDirty(), true, 'the answer reached the component')
    const tree = mounted.rerender()
    assert.equal(stateOf(tree).tag.props.tone, 'warning')
    // The panel is up, the indicator says so, the reason it is unhappy rides the
    // tooltip, and the action closes the same row.
    assertTabRow(tree, {
      label: copy.zh.runningUnseen,
      Tag,
      Button,
      action: copy.zh.stop,
      tip: 'handshake timed out',
    })

    // A read that stops being answered withdraws the claim instead of freezing it.
    ctx.connection.rpc.call = async () => ({ ok: false, error: { code: 'gateway', message: 'gone', details: {} } })
    t.mock.timers.tick(2000)
    await settle()
    const lost = mounted.rerender()
    assert.equal(stateOf(lost).tag.children[0], copy.zh.unknown)
    assert.equal(stateOf(lost).tooltip, undefined, 'a withdrawn snapshot carries no reason either')
    assertTabRow(lost, { label: copy.zh.unknown, Tag, Button, tip: undefined })
  } finally {
    react.unmount()
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('the tab counts restarts out loud, and says when the budget is spent', async (t) => {
  t.mock.timers.enable({ apis: ['setInterval'], now: 0 })
  const { react, modules } = seedTable()
  const { Button, Tag } = modules['@deepseek-ai/dsh-client-ui-primitives']
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    // The copy as this half registered it, interpolated the way the locale
    // service does: every expectation below is a string a user reads.
    const copy = ctx.dictionaries[0].dictionaries
    ctx.snapshot = snapshot({ state: 'starting', seen: false, restarts: 2, pid: null })
    // One hook runtime (the module table's), drawn once per language: the second
    // mount shares the cells and the one interval, exactly as a locale switch does.
    const zh = mountComponent(react, ctx.componentFor(TAB_SLOT), { t: translateWith(copy.zh) })
    await settle()

    // A start after a failure says so, with the count it is on.
    const restarting = zh.rerender()
    assert.equal(stateOf(restarting).tag.props.tone, 'warning', 'a restart is attention, not a plain start')
    assertTabRow(restarting, { label: '悬浮窗重启中 (2/3)', Tag, Button, action: copy.zh.stop })

    const en = mountComponent(react, ctx.componentFor(TAB_SLOT), { t: translateWith(copy.en) })
    assert.equal(stateOf(en.tree).tag.children[0], 'Floating panel restarting (2/3)')

    // The budget spent is said with its count, not left to the colour.
    ctx.snapshot = snapshot({
      state: 'stopped',
      seen: false,
      restarts: 3,
      restartExhausted: true,
      pid: null,
      lastError: 'quorfloat exited 3 times',
    })
    t.mock.timers.tick(2000)
    await settle()
    const spent = zh.rerender()
    assert.equal(stateOf(spent).tag.props.tone, 'danger')
    // Given up on restarts and no process answering: the same one row, with the
    // gave-up clause in the indicator and the supervisor's reason on its tooltip.
    assertTabRow(spent, {
      label: '悬浮窗已停止（连续 3 次失败）',
      Tag,
      Button,
      action: copy.zh.start,
      tip: 'quorfloat exited 3 times',
    })
    assert.equal(
      stateOf(en.rerender()).tag.children[0],
      'Floating panel stopped (3 failures in a row)',
    )
  } finally {
    react.unmount()
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('the tab body is one row — indicator at the start, action at the end — in every state', async (t) => {
  t.mock.timers.enable({ apis: ['setInterval'], now: 0 })
  const { react, modules } = seedTable()
  const { Button, Tag } = modules['@deepseek-ai/dsh-client-ui-primitives']
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    // Start from no answer at all, so the first state is the one the tab shows
    // before the host has said anything.
    ctx.snapshot = null
    const mounted = mountComponent(react, ctx.componentFor(TAB_SLOT), { t: translateWith(copy.zh) })

    // 未知: no snapshot has been read, so the indicator is the whole row and no
    // action is offered (an unread state is not evidence that the panel is down).
    assertTabRow(mounted.tree, { label: copy.zh.unknown, Tag, Button, tip: undefined })
    // Let the mount's own read settle before driving the next one: the poller
    // drops a tick that arrives while its previous read is still in flight.
    await settle()

    /** Drive the tab to the next state the host reports, and re-render it. */
    const show = async overrides => {
      ctx.snapshot = snapshot(overrides)
      t.mock.timers.tick(2000)
      await settle()
      return mounted.rerender()
    }

    // The user's two compared states, side by side in the same row shape.
    // 启动中: the panel is coming up, so the action offers stop.
    assertTabRow(
      await show({ state: 'starting', seen: false, restarts: 0, pid: null, lastError: null }),
      { label: copy.zh.starting, Tag, Button, action: copy.zh.stop },
    )
    // 重启中 (n/limit): the same row, with the count in the indicator.
    assertTabRow(
      await show({ state: 'starting', seen: false, restarts: 2, pid: null, lastError: null }),
      { label: '悬浮窗重启中 (2/3)', Tag, Button, action: copy.zh.stop },
    )
    // 运行中: a live, answering process.
    assertTabRow(
      await show({ state: 'running', seen: true, restarts: 0, pid: 4242, lastError: null }),
      { label: copy.zh.running, Tag, Button, action: copy.zh.stop },
    )
    // 运行中（无应答）: up but not answering, which the indicator says itself.
    assertTabRow(
      await show({ state: 'running', seen: false, restarts: 0, pid: 4242, lastError: null }),
      { label: copy.zh.runningUnseen, Tag, Button, action: copy.zh.stop },
    )
    // 已停止: the supervisor's own word, so start leads.
    assertTabRow(
      await show({ state: 'stopped', seen: false, restarts: 0, pid: null, lastError: null }),
      { label: copy.zh.stopped, Tag, Button, action: copy.zh.start },
    )
    // 失败: a failure the supervisor tracked, and its reason on the tooltip
    // rather than in a paragraph under the row.
    assertTabRow(
      await show({ state: 'failed', seen: false, restarts: 0, pid: null, lastError: 'spawn ENOENT' }),
      { label: copy.zh.failed, Tag, Button, action: copy.zh.start, tip: 'spawn ENOENT' },
    )
    // An unrecognised state word is carried through verbatim, in the same row.
    assertTabRow(
      await show({ state: 'backing-off', seen: false, restarts: 0, pid: null, lastError: null }),
      { label: '悬浮窗 backing-off', Tag, Button, action: copy.zh.start },
    )

    // The row shape is not a Chinese accident: the same state in English draws
    // the same one row, with the English words in it.
    const en = mountComponent(react, ctx.componentFor(TAB_SLOT), { t: translateWith(copy.en) })
    assertTabRow(en.rerender(), { label: copy.en.other.replace('{state}', 'backing-off'), Tag, Button, action: copy.en.start })
  } finally {
    react.unmount()
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('the tab leads with start when the floating panel is down, and sends the transition it offers', async () => {
  const { react, modules } = seedTable()
  const { Button, Tag } = modules['@deepseek-ai/dsh-client-ui-primitives']
  const dom = fakeDom()
  const loaded = loadArtifact(dom, modules)
  try {
    const ctx = pageHalf(loaded)
    const copy = ctx.dictionaries[0].dictionaries
    ctx.snapshot = snapshot({ state: 'stopped', seen: false, pid: null })
    const mounted = mountComponent(react, ctx.componentFor(TAB_SLOT), { t: translateWith(copy.zh) })
    await settle()
    const stopped = mounted.rerender()

    // The load-bearing half: this page is the way back to the panel, and it is
    // the only surface that offers start as a page-level action. The row is the
    // indicator and that action, nothing between them.
    assertTabRow(stopped, { label: copy.zh.stopped, Tag, Button, action: copy.zh.start })
    assert.equal(stateOf(stopped).tag.props.tone, 'neutral')
    const offered = buttonOf(stopped, Button)
    assert.equal(offered.props.variant, 'primary', 'start is a page-level action, not a quiet chip')
    assert.equal(offered.props.size, 'md')
    assert.equal(offered.props.disabled, false)

    offered.props.onClick()
    // The click is guarded while the transition is in flight, so a second one
    // cannot stack a start on top of the first.
    assert.equal(buttonOf(mounted.rerender(), Button).props.disabled, true)
    await settle()
    assert.deepEqual(ctx.actions, ['start'])
    const call = ctx.calls.find(entry => entry.endpoint === 'quorfloat/panel')
    assert.deepEqual(call.payload, { args: { action: 'start' } }, 'the gateway envelope, verbatim')

    const started = mounted.rerender()
    assert.equal(stateOf(started).tag.props.tone, 'success')
    assertTabRow(started, { label: copy.zh.running, Tag, Button, action: copy.zh.stop })
    const rest = buttonOf(started, Button)
    assert.equal(rest.props.variant, 'outline')
    assert.equal(rest.props.disabled, false)
  } finally {
    // The mount owns a poll interval; leaving it behind keeps the runner alive.
    react.unmount()
    loaded.unload()
  }
})

test('it reports the opening state so an untouched window is not treated as absent', async () => {
  const dom = fakeDom({ visibility: 'visible', focused: true })
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await waitFor('the opening report', async () => ctx.calls.length > 0)
    const args = argsOf(ctx.calls[0])
    assert.equal(ctx.calls[0].channel, '/api')
    assert.equal(ctx.calls[0].endpoint, 'quorfloat/reportPresence')
    assert.equal(args.visible, true)
    assert.equal(args.focused, true)
    assert.equal(args.seq, 1)
    assert.equal(typeof args.at, 'number')
    assert.ok(['desktop', 'web'].includes(args.surface))
  } finally {
    loaded.unload()
  }
})

test('losing focus reports visible-but-unfocused, which is what Windows needs', async () => {
  const dom = fakeDom({ visibility: 'visible', focused: true })
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await waitFor('the opening report', async () => ctx.calls.length > 0)
    dom.change({ focused: false, event: 'blur' })
    await waitFor('the blur report', async () => ctx.calls.length > 1)
    const args = argsOf(ctx.calls.at(-1))
    assert.equal(args.visible, true, 'still composited')
    assert.equal(args.focused, false, 'but the user is elsewhere')
  } finally {
    loaded.unload()
  }
})

test('being hidden reports not-visible, which is what macOS occlusion needs', async () => {
  const dom = fakeDom({ visibility: 'visible', focused: true })
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await waitFor('the opening report', async () => ctx.calls.length > 0)
    dom.change({ visibility: 'hidden', focused: false, event: 'visibilitychange' })
    await waitFor('the hidden report', async () => ctx.calls.length > 1)
    const args = argsOf(ctx.calls.at(-1))
    assert.equal(args.visible, false)
    assert.equal(args.focused, false)
  } finally {
    loaded.unload()
  }
})

test('sequence numbers increase monotonically so the host can drop reordered reports', async () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await waitFor('the opening report', async () => ctx.calls.length > 0)
    dom.change({ focused: false, event: 'blur' })
    await waitFor('a second report', async () => ctx.calls.length > 1)
    dom.change({ focused: true, event: 'focus' })
    await waitFor('a third report', async () => ctx.calls.length > 2)
    const seqs = ctx.calls.map(call => argsOf(call).seq)
    for (let index = 1; index < seqs.length; index += 1) {
      assert.ok(seqs[index] > seqs[index - 1], `seq increased: ${seqs.join(',')}`)
    }
  } finally {
    loaded.unload()
  }
})

test('a burst of focus and visibility changes coalesces into one report', async () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await waitFor('the opening report', async () => ctx.calls.length > 0)
    const before = ctx.calls.length
    // Both events fire together in practice (switching windows does both).
    dom.change({ visibility: 'hidden', focused: false, event: 'visibilitychange' })
    dom.change({ focused: false, event: 'blur' })
    dom.change({ visibility: 'hidden', event: 'visibilitychange' })
    await new Promise(resolve => setTimeout(resolve, 250))
    assert.equal(ctx.calls.length, before + 1, 'the burst produced exactly one report')
  } finally {
    loaded.unload()
  }
})

test('a failed report never throws into the client event chain', async () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    ctx.connection.rpc.call = async () => {
      throw new Error('host is gone')
    }
    loaded.module.apply(ctx)
    // The only observable requirement is that nothing rejects or throws; the
    // next focus change sends another report, and the host treats a missing
    // report as "not looking" rather than as "looking".
    await new Promise(resolve => setTimeout(resolve, 200))
  } finally {
    loaded.unload()
  }
})

test('unloading removes every listener', async () => {
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await waitFor('the opening report', async () => ctx.calls.length > 0)
    assert.equal(ctx.effects.length, 1, 'one effect owns the listeners')
    assert.equal(typeof ctx.effects[0], 'function', 'the effect returned a disposer')
    ctx.effects[0]()
    assert.equal((dom.windowListeners.get('focus') ?? []).length, 0)
    assert.equal((dom.windowListeners.get('blur') ?? []).length, 0)
    assert.equal((dom.documentListeners.get('visibilitychange') ?? []).length, 0)
  } finally {
    loaded.unload()
  }
})

test('the state is re-reported even when nothing changes', async (t) => {
  // The defect this pins: reports used to be change-only while the host expires them
  // (deliberately — a page that dies without a `blur` must not pin the authority).
  // Together those made a focused page stop counting as "the user is looking at it"
  // after 30 seconds, and the panel took over approvals belonging to the window in
  // front of the user. Verified on a real `dsh web` session: with the panel open and
  // the browser focused, the panel showed the approval card.
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'], now: 0 })
  const dom = fakeDom({ visibility: 'visible', focused: true })
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await settle()
    assert.equal(ctx.calls.length, 1, 'the opening report')

    t.mock.timers.tick(5000)
    await settle()
    assert.equal(ctx.calls.length, 2, 'the same state is reported again')
    const first = argsOf(ctx.calls[0])
    const second = argsOf(ctx.calls[1])
    assert.equal(second.visible, first.visible)
    assert.equal(second.focused, first.focused)
    assert.ok(second.seq > first.seq, 'each report is a new sequence number')
    assert.ok(second.at >= first.at, 'and carries a time the host can age against')

    t.mock.timers.tick(10_000)
    await settle()
    assert.equal(ctx.calls.length, 4, 'and keeps doing it')
  } finally {
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('unloading stops the heartbeat instead of reporting forever', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'], now: 0 })
  const dom = fakeDom()
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    loaded.module.apply(ctx)
    await settle()
    const reported = ctx.calls.length
    for (const dispose of ctx.effects) dispose()
    t.mock.timers.tick(20_000)
    await settle()
    assert.equal(ctx.calls.length, reported, 'a disposed plugin is silent')
  } finally {
    loaded.unload()
    t.mock.timers.reset()
  }
})

test('a focused page keeps the authority past the host expiry window', async (t) => {
  // The defect this pins lived in the *combination*, which is why neither half's own
  // tests caught it: the host expires reports on purpose (a crashed page sends no
  // `blur`), and the page used to report only on change — so after thirty seconds of
  // the user looking at the window its report expired and the panel took the
  // authority. Driving both halves with one clock is the only way to see it.
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval', 'Date'], now: 0 })
  const { PresenceTracker, evaluateAuthority } = await import(
    new URL('../lib/harness/presence.js', import.meta.url)
  )
  const tracker = new PresenceTracker()
  const dom = fakeDom({ visibility: 'visible', focused: true })
  const loaded = loadArtifact(dom)
  try {
    const ctx = fakeContext()
    // Feed what the half actually sends into the host's real tracker.
    ctx.connection.rpc.call = async (_channel, _endpoint, payload) => {
      const args = payload.args
      const rejection = tracker.report(args.surface, {
        visible: args.visible,
        focused: args.focused,
        seq: args.seq,
        at: args.at,
      })
      assert.equal(rejection, undefined, 'the host accepted every report')
      return { ok: true, value: { accepted: true } }
    }
    loaded.module.apply(ctx)
    await settle()

    const MAX_AGE_MS = 30_000
    for (let elapsed = 0; elapsed <= 90_000; elapsed += 1_000) {
      const verdict = evaluateAuthority({
        desktop: tracker.raw('desktop'),
        web: tracker.raw('web'),
        panelVisible: true,
        maxAgeMs: MAX_AGE_MS,
        now: Date.now(),
      })
      assert.equal(
        verdict.authority,
        'harness',
        `at ${elapsed}ms the user is still looking at the page, so it still owns the decision`,
      )
      t.mock.timers.tick(1_000)
      await settle()
    }
  } finally {
    loaded.unload()
    t.mock.timers.reset()
  }
})
