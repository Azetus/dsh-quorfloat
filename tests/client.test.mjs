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
 * measured separately (`docs/prototype.md` §18).
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
 * Load the built artifact through a stub module loader.
 *
 * @param dom - the stubbed DOM the artifact will observe.
 * @returns the module namespace the factory produced.
 */
function loadArtifact(dom) {
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
    module: registered.factory(() => {
      throw new Error('the browser half must not require anything')
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

/** A client context recording what the half reports. */
function fakeContext() {
  const calls = []
  const logger = { info: () => {}, warn: () => {}, debug: () => {} }
  const effects = []
  const ctx = {
    calls,
    effects,
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
