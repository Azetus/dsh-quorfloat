// The three readers that translate the Harness's own shapes into what the panel's pickers and
// statistics row need.
//
// These are the boundary where a guess becomes a wrong number or a wrong model name, so the tests
// are about *refusing* unfamiliar shapes as much as about reading the familiar ones. The rule the
// project already follows applies here in full: an unrecognised shape yields nothing, because a
// missing statistic is an empty space and a wrong one is a lie.
import assert from 'node:assert/strict'
import test from 'node:test'

const { readStats, readModelGroups, readCurrentSelection, readCurrentPermission, createHarnessFromContext } = await import(
  new URL('../lib/harness/adapter.js', import.meta.url)
)

/** A projections answer in the shape `sessionController.projections` returns. */
function projections(values) {
  return { asOfSeq: 12, values }
}

test('statistics come from the two projections that carry them', () => {
  const stats = readStats(projections({
    sessionStats: { turns: 3, steps: 7, llmMs: 1000, toolMs: 500, ttftMs: 400, ttftSteps: 2, decodeMs: 2000, decodeTokens: 400 },
    tokenUsage: { uncachedInputTokens: 1000, outputTokens: 400, cacheReadTokens: 3000, cacheWriteTokens: 0 },
    contextPressure: { pressureTokens: 4000, capacityTokens: 128000 },
  }))
  assert.deepEqual(stats, {
    turns: 3,
    steps: 7,
    // 400 output tokens over 2000ms of decode: 200/s.
    tokensPerSecond: 200,
    // 3000 of 4000 prompt tokens were cache reads.
    cacheHitPercent: 75,
    contextTokens: 4000,
    contextLimit: 128000,
  })
})

test('a statistic that cannot be computed is absent, not zero', () => {
  // The distinction matters: "0 tok/s" reads as a stalled model, and an absent speed reads as
  // "not measured yet". A session with no completed step has no speed, which is the normal state
  // of a panel someone has just opened.
  const fresh = readStats(projections({ sessionStats: { turns: 0, steps: 0 } }))
  assert.deepEqual(fresh, { turns: 0, steps: 0 })
  assert.equal(fresh.tokensPerSecond, undefined)
  assert.equal(fresh.cacheHitPercent, undefined)

  // Zero decode time is not infinite speed.
  const instant = readStats(projections({
    sessionStats: { turns: 1, steps: 1, decodeMs: 0, decodeTokens: 10 },
  }))
  assert.equal(instant.tokensPerSecond, undefined)

  // A prompt of no tokens has no cache-hit share to report.
  const empty = readStats(projections({ tokenUsage: { cacheReadTokens: 0, uncachedInputTokens: 0, cacheWriteTokens: 0 } }))
  assert.equal(empty.cacheHitPercent, undefined)
})

test('a shape this build does not recognise yields no statistics', () => {
  for (const value of [undefined, null, 42, 'nope', {}, { values: null }, { values: [] }]) {
    assert.equal(readStats(value), undefined, `${JSON.stringify(value)} has no statistics`)
  }
  // A session whose stats projection is missing entirely, even beside a valid usage one, is still
  // no statistics: half an answer drawn as a whole one is what this refuses.
  assert.equal(readStats(projections({ unrelated: { turns: 9 } })), undefined)
})

test('the model catalog becomes picker groups, dropping what cannot be selected', () => {
  const groups = readModelGroups({
    default: { provider: 'deepseek', model: 'v41' },
    groups: [
      {
        id: 'deepseek',
        name: 'DeepSeek',
        models: [
          {
            id: 'v41-flash',
            name: 'V4.1 Flash',
            description: 'Fast',
            reasoning: { efforts: [{ id: 'low', name: '低' }, { id: 'high', name: '高', description: '深入' }], defaultEffort: 'low' },
          },
          // No id: nothing to select, so it is not offered.
          { name: 'broken' },
        ],
      },
      // A provider with no usable model: an empty header is worse than no header.
      { id: 'empty', name: 'Empty', models: [] },
      { name: 'no id' },
    ],
  })
  assert.deepEqual(groups, [
    {
      provider: 'deepseek',
      name: 'DeepSeek',
      models: [
        {
          id: 'v41-flash',
          name: 'V4.1 Flash',
          description: 'Fast',
          efforts: [
            { id: 'low', name: '低' },
            { id: 'high', name: '高', description: '深入' },
          ],
          defaultEffort: 'low',
        },
      ],
    },
  ])

  // A model with no reasoning control is still selectable, with no efforts.
  const plain = readModelGroups({ groups: [{ id: 'p', name: 'P', models: [{ id: 'm', name: 'M' }] }] })
  assert.deepEqual(plain, [{ provider: 'p', name: 'P', models: [{ id: 'm', name: 'M', efforts: [] }] }])

  // And an unreadable catalog is an empty list rather than a crash.
  for (const value of [undefined, null, 7, {}, { groups: 'no' }]) {
    assert.deepEqual(readModelGroups(value), [], `${JSON.stringify(value)} offers nothing`)
  }
})

test('the current selection reads the public next projection, not the internal pending state', () => {
  // A picker that showed `lastUsed` would snap back to the old model after every change, until the
  // next prompt happened to run: the pending selection is the one the user just made.
  const pending = readCurrentSelection(projections({
    modelSelection: {
      lastUsed: { provider: 'deepseek', model: 'v41' },
      next: { provider: 'deepseek', model: 'v41-flash', reasoningEffort: 'high' },
    },
  }))
  assert.deepEqual(pending, { provider: 'deepseek', model: 'v41-flash', reasoningEffort: 'high' })

  // With nothing pending, the last used one is what the next step will use.
  const settled = readCurrentSelection(projections({
    modelSelection: { lastUsed: { provider: 'deepseek', model: 'v41' }, next: { provider: 'deepseek', model: 'v41' } },
  }))
  assert.deepEqual(settled, { provider: 'deepseek', model: 'v41' })

  // And an unrecognised shape yields nothing rather than a model that is not selected.
  for (const value of [
    undefined,
    null,
    projections({}),
    projections({ modelSelection: {} }),
    projections({ modelSelection: { pending: { provider: 'p', model: 'm' }, lastUsed: { provider: 'p', model: 'old' } } }),
    projections({ modelSelection: { next: { provider: '', model: 'm' } } }),
    projections({ modelSelection: { next: { provider: 'p', model: '' } } }),
    projections({ modelSelection: { lastUsed: { model: 'no-provider' } } }),
  ]) {
    assert.equal(readCurrentSelection(value), undefined, `${JSON.stringify(value)} names no model`)
  }
})

// Public shapes from permission-presets/src/types.ts and session-controller's wire projection.
// Arbitrary values ensure catalogs are transported, not replaced with three built-in choices.
test('options preserve live Harness catalogs and read permissions without loading an Agent', async () => {
  let options = [{ value: 'team-audit', name: '团队审计', description: '组织配置的权限' }]
  const services = {
    sessionController: {
      modelCatalog: async () => ({ groups: [{ id: 'team', name: '团队服务', models: [{
        id: 'custom', name: '内部模型', description: '宿主描述',
        reasoning: { efforts: [{ id: 'deliberate', name: '仔细', description: '自定义档位' }], defaultEffort: 'deliberate' },
      }] }] }),
      projections: async () => projections({ permissions: { currentValue: 'team-audit' },
        modelSelection: { lastUsed: null, next: { provider: 'team', model: 'custom', reasoningEffort: 'deliberate' } } }),
    },
    permissionPresets: { catalog: () => ({ options }) },
  }
  const { harness } = createHarnessFromContext(name => services[name], { warn() {}, debug() {} })
  const result = await harness.options('cold-session')
  assert.equal(result.permission, 'team-audit')
  assert.deepEqual(result.permissions, options)
  assert.equal(result.current.model, 'custom')
  assert.equal(result.groups[0].name, '团队服务')
  assert.deepEqual(result.groups[0].models[0].efforts, [{ id: 'deliberate', name: '仔细', description: '自定义档位' }])
  options = [{ value: 'new-policy', name: '新的策略' }]
  assert.deepEqual((await harness.options('cold-session')).permissions, options)
  options = [
    { value: 'danger-full-access', name: 'Full access' },
    { value: 'read-only', name: 'read-only' },
    { value: 'workspace-write', name: 'Workspace Write' },
    { value: 'read-only', name: '组织指定名称' },
  ]
  assert.deepEqual((await harness.options('cold-session')).permissions.map(option => option.name),
    ['完全权限', '仅可查看', '工作区内修改', '组织指定名称'])
  for (const value of [undefined, {}, projections({ permissions: { current: 'read-only' } }),
    projections({ permissions: { currentValue: '' } }), projections({ permissions: { currentValue: 42 } })]) {
    assert.equal(readCurrentPermission(value), undefined)
  }
})
