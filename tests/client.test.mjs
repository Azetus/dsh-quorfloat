// Category 1 (cross-process interface): the client artifact the Harness loads, its slots, and the reportPresence envelope.
/**
 * Browser-half interface tests.
 *
 * The artifact under test is the real build output (`lib/client.js`), loaded the
 * way the Harness client loader loads it: through
 * `window.__ModuleLoader__.load({ id, factory })`. Testing the artifact rather
 * than the TypeScript source means the loader wrapper, the export conversion, and
 * the self-containment check are all covered — a source-level test would pass
 * while the published bundle failed to register.
 *
 * What this file keeps is the interface: the module contract the loader reads,
 * the slot names and entry identity the settings page renders, and the presence
 * fields the host authority consumes. Rendering, state vocabulary, and the
 * polling cadence are decided in code, not pinned here.
 *
 * The DOM is stubbed because the values that matter here are the two the browser
 * provides (`document.visibilityState`, `document.hasFocus()`) and the events
 * that report their change.
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
  }
}

/**
 * The two module-table entries this half reads, as inert stand-ins.
 *
 * The shell's real table is a frozen set of platform singletons; the half is only
 * allowed to ask for members of it. Nothing renders in this file, so the atoms are
 * placeholders while the factory's `React.createElement` binding stays real.
 */
function seedModules() {
  return {
    react: {
      createElement: (type, props, ...children) => ({ type, props: props ?? {}, children }),
      useState: initial => [initial, () => {}],
      useEffect: () => {},
    },
    '@deepseek-ai/dsh-client-ui-primitives': { Tag: () => null, Button: () => null },
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
function loadArtifact(dom, modules = seedModules()) {
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
    /** `{ options, component }` per `slots.register`, in registration order. */
    registered: [],
    /** `{ namespace, dictionaries }` per `locale.register`. */
    dictionaries: [],
    connection: {
      rpc: {
        async call(channel, endpoint, payload) {
          calls.push({ channel, endpoint, payload })
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
  }
  // Registered with the active load so teardown can unload it; a context built
  // outside a load is simply not tracked.
  activeLoad?.push(ctx)
  return ctx
}

/** Read the args of a report. */
function argsOf(call) {
  return call.payload.args
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
 * @param dictionary - one locale's registered copy.
 * @returns the translate function for it.
 */
function translateWith(dictionary) {
  return (key, params = {}) => (dictionary[key] ?? key)
    .replace(/\{(\w+)\}/g, (whole, name) => (name in params ? String(params[name]) : whole))
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
 * panel is mounted, and renders every entry with the open page's `subject`.
 * Registering an entry is the interface with that page; which subjects it
 * answers is a decision the component makes.
 */
const BADGE_SLOT = 'plugins.detail.badge'
const ACTIONS_SLOT = 'plugins.detail.actions'

/** Both detail slots, in registration order. */
const DETAIL_SLOTS = [BADGE_SLOT, ACTIONS_SLOT]

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
