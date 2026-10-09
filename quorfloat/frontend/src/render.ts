// The panel, rendered from the shell's snapshots.
//
// One snapshot in, one DOM out: `render(state)` rebuilds every dynamic part and
// leaves the few things that are the *user's* — the draft in the composer, the
// fold open/closed memory, the selected workspace for a new conversation — in
// module state, because the snapshot knows nothing about them.
//
// Real clicks dispatch through the typed `api`; the shell is the authority on
// everything the snapshot carries, so no click here mutates conversation state.

import { api } from './api'
import { icon, ring } from './lib/icons'
import { renderMarkdown } from './lib/markdown'
import { keepOpenChecked, themeColorScheme } from './lib/settings'
import { cacheLabel, contextLabel, roundsLabel, tokensLabel } from './lib/stats'
import { foldTurns, type Turn } from './lib/turns'
import type { Conversation, Snapshot } from './lib/state'

// ── user-owned state the snapshot does not carry ────────────────────────────

/** The fold open/closed memory, keyed by the turn's identity (question + position). */
const foldOpen = new Map<string, boolean>()
/** The composer draft; the textarea is only overwritten when it is not focused. */
let draft = ''
/** The last submitted text, for the "refused → put the words back" rule. */
let lastSent: { text: string; restored: boolean } | null = null
/** The workspace a new conversation will be created in (frontend choice). */
let workspaceChoice: string | null = null
/** Which page is up: the conversation or the settings. */
let page: 'main' | 'settings' = 'main'
/** Which popover is open, if any (the trigger's id). */
let openMenu: string | null = null
/** Whether the shortcut chip is recording the next chord. */
let recordingHotkey = false
/** A transient error the settings page produced, for the status line. */
let transientError: string | null = null

/** The latest snapshot, kept for event handlers that need it. */
let lastState: Snapshot | null = null

/** The close path the page's own controls take: fade, then hide the window. */
let onCloseRequested: () => void = () => {}

const $ = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id)
  if (element === null) throw new Error(`#${id} is missing from the document`)
  return element as T
}

const windowEl = $('q-window')
const input = $<HTMLTextAreaElement>('q-input')
const threadEl = $('q-thread')
const threadScroll = $('q-thread-scroll')
const cardsEl = $('q-cards')

/** The native window is the panel plus its shadow room (top 20 + bottom 40),
 *  mirrored from `quorfloat/src/app/height.rs`. */
const SHADOW_ROOM = 60

// ── static icons, once ─────────────────────────────────────────────────────

function fillStaticIcons(): void {
  const put = (id: string, name: string) => {
    const svg = icon(name)
    if (svg !== null) $(id).replaceChildren(svg)
  }
  put('q-workspace-icon', 'folder')
  put('q-session-icon', 'message-square')
  put('q-workspace-chevron', 'chevron-down')
  put('q-session-chevron', 'chevron-down')
  put('q-settings-open-icon', 'settings-2')
  put('q-close-icon', 'x')
  put('q-search-icon', 'search')
  put('q-send-icon', 'arrow-up')
  put('q-permission-icon', 'shield-check')
  put('q-permission-chevron', 'chevron-down')
  put('q-config-chevron', 'chevron-down')
  put('q-stat-speed-icon', 'gauge')
  put('q-stat-token-icon', 'database')
  const contextIcon = $('q-stat-context-icon')
  contextIcon.innerHTML = ring(0)
}

// ── helpers ────────────────────────────────────────────────────────────────

function workspaceTitle(state: Snapshot): string {
  const following = state.session.following
  if (following !== null) {
    const byCwd = state.session.conversations.find(c => c.sessionId === following.sessionId)?.cwd
    if (byCwd !== undefined && byCwd !== null) {
      const match = state.session.workspaces.find(w => w.path === byCwd)
      if (match !== undefined) return match.title
    }
    return following.label ?? '工作区'
  }
  const chosen = state.session.workspaces.find(w => w.workspaceId === workspaceChoice)
  return chosen?.title ?? '选择工作区'
}

function sessionTitle(state: Snapshot): string {
  const following = state.session.following
  if (following === null) return '新会话'
  const conversation = state.session.conversations.find(c => c.sessionId === following.sessionId)
  return conversation?.title ?? following.sessionId.slice(0, 13)
}

function statusText(state: Snapshot): string | null {
  const prompt = state.delivery.prompt.status
  const cancel = state.delivery.cancel.status
  if (prompt !== null && prompt !== undefined) return prompt
  if (cancel !== null && cancel !== undefined) return cancel
  if (state.session.settingFailure !== null) return state.session.settingFailure
  if (state.session.createFailure !== null) return state.session.createFailure
  if (state.hotkey.reason !== null) return state.hotkey.reason
  return transientError
}

function isBusy(state: Snapshot): boolean {
  return (
    state.transcript.turnActive ||
    state.transcript.streaming ||
    state.delivery.prompt.state === 'sending'
  )
}

function relativeTime(updatedAt: number): string {
  const now = Date.now()
  const diff = now - updatedAt
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  const date = new Date(updatedAt)
  const today = new Date()
  if (date.toDateString() === today.toDateString()) {
    return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
  }
  return `${date.getMonth() + 1}月${date.getDate()}日`
}

// ── top bar ────────────────────────────────────────────────────────────────

function renderTop(state: Snapshot): void {
  $('q-workspace-name').textContent = workspaceTitle(state)
  $('q-session-name').textContent = sessionTitle(state)
  $('q-workspace-pinmark').replaceChildren()
  $('q-session-pinmark').replaceChildren()
  const pinnedSession = state.session.pinned
  if (pinnedSession !== null) {
    const mark = icon('pin')
    if (mark !== null) $('q-session-pinmark').replaceChildren(mark)
    $('q-session-pinmark').hidden = false
  } else {
    $('q-session-pinmark').hidden = true
  }
  if (state.session.following === null && workspaceChoice !== null) {
    const mark = icon('pin')
    if (mark !== null) $('q-workspace-pinmark').replaceChildren(mark)
    $('q-workspace-pinmark').hidden = false
  } else {
    $('q-workspace-pinmark').hidden = true
  }
  $('q-settings-open').setAttribute('aria-pressed', String(page === 'settings'))
}

// ── composer ───────────────────────────────────────────────────────────────

function renderComposer(state: Snapshot): void {
  const busy = isBusy(state)
  $('q-send-icon').replaceChildren(icon(busy ? 'square' : 'arrow-up') ?? '')
  $('q-send').setAttribute('aria-label', busy ? '停止生成' : '发送问题')
  input.placeholder = state.transcript.entries.length > 0 ? '继续追问…' : '问点什么…'
  if (document.activeElement !== input) input.value = draft
  resizeComposer()
}

function resizeComposer(): void {
  input.style.height = '38px'
  input.style.height = `${Math.min(input.scrollHeight || 38, 120)}px`
  input.style.overflowY = input.scrollHeight > 120 ? 'auto' : 'hidden'
}

// ── thread ─────────────────────────────────────────────────────────────────

function renderThread(state: Snapshot): void {
  const scroll = threadScroll
  const before = scroll.scrollTop
  const atBottom = before + scroll.clientHeight >= scroll.scrollHeight - 4
  const view = foldTurns(state.transcript.entries, state.transcript.live, state.transcript.turnActive)

  threadEl.replaceChildren()
  for (const line of view.loose) {
    const div = document.createElement('div')
    div.className = 'q-question'
    div.textContent = line.text
    threadEl.append(div)
  }
  for (const turn of view.turns) threadEl.append(turnElement(turn))

  // The panel's cap and the thread's internal scroll are CSS's job now (the
  // flex chain #q-main → .q-thread-scroll); this only restores the reading
  // position across rebuilds.
  if (atBottom) scroll.scrollTop = scroll.scrollHeight
  else scroll.scrollTop = before
}

function turnElement(turn: Turn): HTMLElement {
  const article = document.createElement('article')
  article.className = 'q-turn'

  const question = document.createElement('div')
  question.className = 'q-question'
  question.textContent = `你 · ${turn.question}`
  article.append(question)

  if (turn.working.length > 0 || turn.inProgress) {
    const open = foldOpen.get(turn.id) ?? false
    const disclosure = document.createElement('button')
    disclosure.type = 'button'
    disclosure.className = 'q-disclosure'
    disclosure.setAttribute('aria-expanded', String(open))
    const caret = document.createElement('span')
    caret.className = 'q-caret'
    caret.replaceChildren(icon('caret-right') ?? '')
    caret.style.transform = open ? 'rotate(90deg)' : ''
    const label = document.createElement('span')
    label.textContent = turn.inProgress ? '正在工作' : '已完成'
    disclosure.append(caret, label)
    if (turn.inProgress) {
      const spinner = document.createElement('span')
      spinner.className = 'q-spin'
      // Phosphor circle-notch (regular, MIT) — the design draws no working
      // marker; this is the panel's own, as in the egui shell.
      spinner.innerHTML =
        '<svg viewBox="0 0 256 256" width="11" height="11" aria-hidden="true"><path fill="currentColor" d="M232 128a104 104 0 0 1-208 0c0-6.2 5-11.2 11.2-11.2s11.2 5 11.2 11.2a81.6 81.6 0 0 0 163.2 0c0-6.2 5-11.2 11.2-11.2S232 121.8 232 128Z"/></svg>'
      disclosure.append(spinner)
    }
    disclosure.addEventListener('click', () => {
      const next = !(foldOpen.get(turn.id) ?? false)
      foldOpen.set(turn.id, next)
      disclosure.setAttribute('aria-expanded', String(next))
      caret.style.transform = next ? 'rotate(90deg)' : ''
      const working = disclosure.nextElementSibling
      if (working instanceof HTMLElement) working.classList.toggle('q-open', next)
    })
    article.append(disclosure)

    const working = document.createElement('div')
    working.className = open ? 'q-working q-open' : 'q-working'
    for (const line of turn.working) working.append(workingLineElement(line))
    article.append(working)
  }

  if (turn.answer !== null) {
    const answer = document.createElement('div')
    answer.className = 'q-answer'
    const raw = turn.answer.blocks
      .flatMap(b => (b.kind === 'text' ? [b.text] : []))
      .join('\n\n')
    answer.innerHTML = renderMarkdown(raw)
    article.append(answer)
    article.append(answerBar(turn, raw))
  }
  return article
}

function workingLineElement(line: Turn['working'][number]): HTMLElement {
  const div = document.createElement('div')
  switch (line.kind) {
    case 'reasoning':
      div.className = 'q-working-line q-reasoning'
      div.textContent = line.text
      break
    case 'text':
      div.className = 'q-working-line'
      div.textContent = line.text
      break
    case 'call': {
      div.className = 'q-working-line q-call'
      const name = document.createElement('span')
      name.className = 'q-tool-name'
      name.textContent = `⚙ ${line.name ?? '工具'}`
      div.append(name)
      if (line.arguments !== null) {
        const args = document.createElement('span')
        args.className = 'q-call-args'
        args.textContent = ` ${line.arguments}`
        div.append(args)
      }
      break
    }
    case 'tool': {
      div.className = line.isError ? 'q-working-line q-tool q-error' : 'q-working-line q-tool'
      const name = document.createElement('span')
      name.className = 'q-tool-name'
      name.textContent = line.name !== null ? `↳ ${line.name}` : '↳ 工具结果'
      div.append(name, document.createTextNode(` ${line.text}`))
      break
    }
  }
  return div
}

function answerBar(turn: Turn, raw: string): HTMLElement {
  const bar = document.createElement('div')
  bar.className = 'q-answerbar'
  const label = document.createElement('span')
  label.textContent = turn.inProgress ? '正在生成' : '回答完成'
  bar.append(label)
  if (!turn.inProgress && raw.trim() !== '') {
    const copy = document.createElement('button')
    copy.type = 'button'
    copy.textContent = '复制回答'
    copy.addEventListener('click', () => {
      // The answer is copied as the raw Markdown, not the rendered text.
      navigator.clipboard
        .writeText(raw)
        .then(() => {
          copy.textContent = '已复制'
        })
        .catch(() => {
          copy.textContent = '请手动选择复制'
        })
    })
    bar.append(copy)
  }
  return bar
}

// ── cards ──────────────────────────────────────────────────────────────────

function renderCards(state: Snapshot): void {
  cardsEl.replaceChildren()
  for (const interaction of state.interactions) cardsEl.append(cardElement(interaction))
  if (state.handoff !== null) cardsEl.append(handoffElement(state.handoff))
}

function cardElement(interaction: Snapshot['interactions'][number]): HTMLElement {
  const card = document.createElement('div')
  card.className = 'q-card'

  const title = document.createElement('div')
  title.className = 'q-card-title'
  const shield = icon(interaction.kind === 'approval' ? 'shield-check' : 'message-square')
  if (shield !== null) title.append(shield)
  const strong = document.createElement('strong')
  strong.textContent = interaction.kind === 'approval' ? '需要批准' : '需要你的回答'
  title.append(strong)
  card.append(title)

  if (interaction.kind === 'approval') {
    if (interaction.toolName !== null) {
      const tool = document.createElement('div')
      tool.className = 'q-card-title'
      tool.textContent = `工具 · ${interaction.toolName}`
      card.append(tool)
    }
    if (interaction.detail !== null) {
      const reason = document.createElement('div')
      reason.className = 'q-card-reason'
      reason.textContent = interaction.detail
      card.append(reason)
    }
    const stateLine = document.createElement('div')
    stateLine.className = 'q-card-state'
    if (interaction.state === 'submitting') stateLine.textContent = '提交中…'
    else if (interaction.state === 'applied') {
      stateLine.textContent = interaction.verdict === 'allowed-once' ? '已允许一次' : '已拒绝'
    } else if (interaction.state === 'refused') {
      stateLine.classList.add('q-refused')
      stateLine.textContent = interaction.refusalReason ?? '请求已失效'
    }
    card.append(stateLine)

    const actions = document.createElement('div')
    actions.className = 'q-card-actions'
    if (interaction.state === 'pending') {
      const allow = document.createElement('button')
      allow.type = 'button'
      allow.className = 'q-allow'
      allow.textContent = '允许一次'
      allow.addEventListener('click', () => void api.answerApproval(interaction.id, true))
      const reject = document.createElement('button')
      reject.type = 'button'
      reject.textContent = '拒绝'
      reject.addEventListener('click', () => void api.answerApproval(interaction.id, false))
      actions.append(allow, reject)
    } else {
      const dismiss = document.createElement('button')
      dismiss.type = 'button'
      dismiss.textContent = '关闭'
      dismiss.addEventListener('click', () => void api.dismissInteraction(interaction.id))
      actions.append(dismiss)
    }
    card.append(actions)
  } else {
    for (const [header, text] of interaction.questions) {
      const line = document.createElement('div')
      line.className = 'q-card-reason'
      line.textContent = header !== null ? `${header}：${text}` : text
      card.append(line)
    }
    const note = document.createElement('div')
    note.className = 'q-card-state'
    note.textContent = '这个问题请在 Harness 窗口回答'
    card.append(note)
    const actions = document.createElement('div')
    actions.className = 'q-card-actions'
    const dismiss = document.createElement('button')
    dismiss.type = 'button'
    dismiss.textContent = '知道了'
    dismiss.addEventListener('click', () => void api.dismissInteraction(interaction.id))
    actions.append(dismiss)
    card.append(actions)
  }
  return card
}

function handoffElement(handoff: NonNullable<Snapshot['handoff']>): HTMLElement {
  const banner = document.createElement('div')
  banner.className = 'q-handoff'
  const text = document.createElement('span')
  const hasSurface = handoff.surfaces.length > 0
  text.textContent = handoff.kind === 'approval'
    ? (hasSurface ? '审批已转交 Harness 窗口处理' : '当前没有可以处理它的 Harness 窗口')
    : (hasSurface ? '这个问题已转交 Harness 窗口回答' : '当前没有可以回答它的 Harness 窗口')
  banner.append(text)
  const dismiss = document.createElement('button')
  dismiss.type = 'button'
  dismiss.textContent = '知道了'
  dismiss.addEventListener('click', () => void api.dismissHandoff())
  banner.append(dismiss)
  return banner
}

// ── runtime row (permission + model) ──────────────────────────────────────

function renderRuntime(state: Snapshot): void {
  const options = state.session.options
  const current = options?.current
  $('q-permission-name').textContent =
    options?.permissions.find(p => p.value === options.permission)?.name ?? '—'
  $('q-model-name').textContent =
    options?.models.find(m => m.id === current?.model && m.provider === current?.provider)?.name ?? '—'
  $('q-effort-name').textContent =
    options?.models
      .find(m => m.id === current?.model && m.provider === current?.provider)
      ?.efforts.find(e => e.id === current?.reasoningEffort)?.name ?? ''
  $('q-config-menu').hidden = true
  $('q-permission-menu').hidden = true
  $('q-workspace-menu').hidden = true
  $('q-session-menu').hidden = true
  for (const id of ['q-workspace', 'q-session', 'q-config', 'q-permission']) {
    $(id).setAttribute('aria-expanded', 'false')
  }
  if (openMenu !== null) renderMenu(state, openMenu)
}

// ── menus ─────────────────────────────────────────────────────────────────

const menuFor: Record<string, string> = {
  'q-workspace': 'q-workspace-menu',
  'q-session': 'q-session-menu',
  'q-config': 'q-config-menu',
  'q-permission': 'q-permission-menu',
}

function renderMenu(state: Snapshot, id: string): void {
  const menu = $(menuFor[id] ?? '')
  if (menu === undefined || menu === null) return
  menu.replaceChildren()
  menu.hidden = false
  $(id).setAttribute('aria-expanded', 'true')
  if (id === 'q-workspace') renderWorkspaceMenu(state, menu)
  else if (id === 'q-session') renderSessionMenu(state, menu)
  else if (id === 'q-permission') renderPermissionMenu(state, menu)
  else if (id === 'q-config') renderConfigMenu(state, menu)
}

function popHead(menu: HTMLElement, title: string, detail = ''): void {
  const header = document.createElement('div')
  header.className = 'q-pophead'
  const strong = document.createElement('strong')
  strong.textContent = title
  header.append(strong)
  if (detail !== '') {
    const span = document.createElement('span')
    span.textContent = detail
    header.append(span)
  }
  menu.append(header)
}

function separator(menu: HTMLElement): void {
  menu.append(document.createElement('hr'))
}

function hint(menu: HTMLElement, text: string): void {
  const line = document.createElement('div')
  line.className = 'q-pophint'
  line.textContent = text
  menu.append(line)
}

function option(label: string, selected: boolean, onPick: () => void, description = '', symbol: string | null = null): HTMLButtonElement {
  const button = document.createElement('button')
  button.type = 'button'
  button.className = 'q-option'
  button.setAttribute('aria-pressed', String(selected))
  if (symbol !== null) {
    const svg = icon(symbol)
    if (svg !== null) button.append(svg)
  }
  const main = document.createElement('span')
  main.className = 'q-option-main'
  const text = document.createElement('span')
  text.textContent = label
  main.append(text)
  if (description !== '') {
    const small = document.createElement('small')
    small.textContent = description
    main.append(small)
  }
  button.append(main)
  if (selected) {
    const check = icon('check', 'q-check')
    if (check !== null) button.append(check)
  }
  button.addEventListener('click', onPick)
  return button
}

function pinButton(name: string, pinned: boolean, onPick: () => void): HTMLButtonElement {
  const button = document.createElement('button')
  button.type = 'button'
  button.className = 'q-pin'
  button.setAttribute('aria-label', `${pinned ? '取消固定' : '固定'}${name}`)
  button.setAttribute('aria-pressed', String(pinned))
  const svg = icon(pinned ? 'pin-off' : 'pin')
  if (svg !== null) button.append(svg)
  button.addEventListener('click', onPick)
  return button
}

function closeMenus(): void {
  if (openMenu === null) return
  $(menuFor[openMenu] ?? '').hidden = true
  $(openMenu).setAttribute('aria-expanded', 'false')
  openMenu = null
}

/** Close whichever popover is open, if any. */
export function closeMenu(): void {
  closeMenus()
}

function renderWorkspaceMenu(state: Snapshot, menu: HTMLElement): void {
  popHead(menu, '工作区')
  for (const workspace of state.session.workspaces) {
    const row = document.createElement('div')
    row.className = 'q-option-row'
    const selected = state.session.following === null && workspaceChoice === workspace.workspaceId
    row.append(option(workspace.title, selected, () => {
      workspaceChoice = workspace.workspaceId
      closeMenus()
      render(state)
    }, workspace.path, 'folder'))
    row.append(pinButton(`工作区 ${workspace.title}`, workspaceChoice === workspace.workspaceId, () => {
      workspaceChoice = workspaceChoice === workspace.workspaceId ? null : workspace.workspaceId
      closeMenus()
      render(state)
    }))
    menu.append(row)
  }
  separator(menu)
  hint(menu, '选择工作区后，新会话在此开始。')
}

function renderSessionMenu(state: Snapshot, menu: HTMLElement): void {
  popHead(menu, '会话', workspaceTitle(state))
  menu.append(option('新建会话', state.session.following === null, () => {
    closeMenus()
    void api.startNew()
  }, '', 'plus'))
  separator(menu)
  const conversations = state.session.conversations.filter(c => !c.blank)
  for (const conversation of conversations) {
    const row = document.createElement('div')
    row.className = 'q-option-row'
    const selected = state.session.following?.sessionId === conversation.sessionId
    row.append(option(conversationTitle(conversation), selected, () => {
      closeMenus()
      void api.selectSession(conversation.sessionId)
    }, relativeTime(conversation.updatedAt), 'message-square'))
    row.append(pinButton(`会话 ${conversationTitle(conversation)}`, state.session.pinned === conversation.sessionId, () => {
      closeMenus()
      void api.pin(state.session.pinned === conversation.sessionId ? null : conversation.sessionId)
    }))
    menu.append(row)
  }
  if (conversations.length === 0) hint(menu, '没有可切换的会话。')
  separator(menu)
  const pinned = state.session.pinned
  hint(menu, pinned !== null ? '呼出时继续固定会话。' : '固定会话后，每次呼出继续此会话。')
}

function conversationTitle(conversation: Conversation): string {
  return conversation.title ?? conversation.sessionId.slice(0, 13)
}

function renderPermissionMenu(state: Snapshot, menu: HTMLElement): void {
  popHead(menu, '会话权限')
  const options = state.session.options
  if (options === null) {
    hint(menu, '权限目录尚未到达。')
    return
  }
  for (const permission of options.permissions) {
    menu.append(option(permission.name, options.permission === permission.value, () => {
      closeMenus()
      void api.setPermission(permission.value)
    }, permission.description ?? '', 'shield-check'))
  }
}

function renderConfigMenu(state: Snapshot, menu: HTMLElement): void {
  const options = state.session.options
  if (options === null) {
    hint(menu, '模型目录尚未到达。')
    return
  }
  const current = options.current
  const currentModel = options.models.find(m => m.id === current?.model && m.provider === current?.provider)
  const back = document.createElement('button')
  back.type = 'button'
  back.className = 'q-config-back'
  back.textContent = '‹ 模型与推理等级'
  back.addEventListener('click', () => {
    closeMenus()
    openMenu = 'q-config'
    render(state)
  })
  menu.append(back)
  popHead(menu, '模型')
  for (const model of options.models) {
    const selected = current?.model === model.id && current?.provider === model.provider
    menu.append(option(model.name, selected, () => {
      closeMenus()
      void api.selectModel(model.provider, model.id, current?.reasoningEffort ?? model.defaultEffort ?? null)
    }, model.description ?? ''))
  }
  separator(menu)
  popHead(menu, '推理等级')
  for (const effort of currentModel?.efforts ?? []) {
    menu.append(option(effort.name, current?.reasoningEffort === effort.id, () => {
      closeMenus()
      if (current !== null) void api.selectModel(current.provider, current.model, effort.id)
    }, effort.description ?? ''))
  }
  if ((currentModel?.efforts ?? []).length === 0) hint(menu, '当前模型没有可选档位。')
}

// ── footer ────────────────────────────────────────────────────────────────

function renderFooter(state: Snapshot): void {
  const status = statusText(state)
  $('q-status').innerHTML =
    '<kbd>⏎</kbd> 发送　<kbd>⇧ ⏎</kbd> 换行　<kbd>esc</kbd> 关闭' +
    (status !== null && status !== '' ? `<span style="margin-left:10px">${escapeHtml(status)}</span>` : '')

  const stats = state.session.stats
  const statsEl = $('q-session-stats')
  if (stats === null) {
    statsEl.hidden = true
    return
  }
  statsEl.hidden = false
  $('q-stat-speed').textContent = roundsLabel(stats)
  const tokens = tokensLabel(stats)
  $('q-stat-token-count').textContent = tokens.count
  $('q-stat-cache').textContent = tokens.cache
  const context = contextLabel(stats)
  const contextStat = $('q-stat-context')
  if (context === null) contextStat.hidden = true
  else {
    contextStat.hidden = false
    $('q-stat-context-value').textContent = context
    const percent = stats.contextTokens !== null && stats.contextLimit !== null && stats.contextLimit > 0
      ? Math.min(100, (stats.contextTokens / stats.contextLimit) * 100)
      : 0
    $('q-stat-context-icon').innerHTML = ring(percent)
  }
}

function escapeHtml(text: string): string {
  return text.replace(/[&<>"]/g, ch => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[ch]!)
}

// ── settings ──────────────────────────────────────────────────────────────

function renderSettings(state: Snapshot): void {
  for (const chip of document.querySelectorAll<HTMLButtonElement>('#q-theme-chips .q-chip')) {
    chip.setAttribute('aria-pressed', String(chip.dataset.theme === state.settings.theme))
  }
  // The live DOM's own state, on the record whenever it changes: the last link
  // in the ground-truth chain for the switch — if the marker says one thing and
  // the screen shows another, the divergence is in rendering, not in binding.
  const keepOpen = $('q-keep-open')
  const checked = keepOpenChecked(state.settings.hideOnBlur)
  if (keepOpen.getAttribute('aria-checked') !== checked) {
    keepOpen.setAttribute('aria-checked', checked)
    void api.log(
      `frontend switch aria-checked=${checked} hideOnBlur=${state.settings.hideOnBlur}`,
    )
  }
  const chip = $<HTMLInputElement>('q-shortcut')
  if (recordingHotkey) {
    chip.classList.add('q-recording')
    chip.value = '按下组合键…'
  } else {
    chip.classList.remove('q-recording')
    chip.value = state.hotkey.requested ?? '—'
    chip.title = state.hotkey.reason ?? ''
  }
}

// ── the whole panel ───────────────────────────────────────────────────────

/**
 * Measure what the panel wants to be, and report it to the shell's height
 * machine — the half of the contract the machine cannot see from its side.
 *
 * The measure is bounded (the cap plus a pixel, never the full natural layout
 * of a 400-entry transcript), and the report is clamped to the configured max:
 * the panel never asks for more than the product allows.
 *
 * @param state - the snapshot the settings come from.
 */
export function measureAndReport(state: Snapshot): void {
  windowEl.style.maxHeight = `${state.settings.maxHeight + 1}px`
  const natural = windowEl.offsetHeight
  const desired = Math.max(1, Math.min(natural, state.settings.maxHeight))
  // Present at the smaller of the product cap and the room the native window
  // currently provides: content reveals itself once the shell has made room.
  const available = Math.max(80, window.innerHeight - SHADOW_ROOM)
  windowEl.style.maxHeight = `${Math.min(state.settings.maxHeight, available)}px`
  void api.reportContentHeight(Math.round(desired))
}

function render(state: Snapshot): void {
  // The design's tokens are light-dark(); one property switches both palettes.
  document.documentElement.style.colorScheme = themeColorScheme(state.settings.theme)

  if (page === 'settings') $('q-settings').hidden = false, $('q-main').hidden = true
  else $('q-settings').hidden = true, $('q-main').hidden = false

  // The refusal recovery rule: the composer clears on send, and when the shell
  // later refuses, the words come back — only into an empty input.
  if (state.delivery.prompt.state === 'failed' && lastSent !== null && !lastSent.restored && draft === '' && input.value === '') {
    draft = lastSent.text
    input.value = draft
    lastSent.restored = true
  }
  if (state.delivery.prompt.state !== 'failed' && lastSent !== null && lastSent.restored) {
    // A new send resets the guard for its own refusal.
    if (state.delivery.prompt.state === 'sending' || state.delivery.prompt.state === 'accepted') {
      lastSent = null
    }
  }

  renderTop(state)
  renderComposer(state)
  renderThread(state)
  renderCards(state)
  renderRuntime(state)
  renderFooter(state)
  renderSettings(state)
  measureAndReport(state)
}

// ── the user's events ─────────────────────────────────────────────────────

export function submitDraft(state: Snapshot): void {
  const text = input.value.trim()
  if (text === '') {
    input.focus()
    return
  }
  if (isBusy(state)) {
    void api.cancel()
    return
  }
  void api.submit(text, workspaceChoice)
  lastSent = { text, restored: false }
  draft = ''
  input.value = ''
  resizeComposer()
}

export function toggleMenu(state: Snapshot, id: string): void {
  const same = openMenu === id
  closeMenus()
  if (same) return
  openMenu = id
  render(state)
}

export function openSettings(state: Snapshot): void {
  page = 'settings'
  closeMenus()
  render(state)
}

export function backFromSettings(state: Snapshot): void {
  page = 'main'
  closeMenus()
  render(state)
  input.focus()
}

export function isSettingsPage(): boolean {
  return page === 'settings'
}

export function hasOpenMenu(): boolean {
  return openMenu !== null
}

export function setRecording(recording: boolean): void {
  recordingHotkey = recording
}

export function isRecording(): boolean {
  return recordingHotkey
}

export function setTransientError(message: string | null): void {
  transientError = message
}

export function updateDraftFromInput(): void {
  draft = input.value
  resizeComposer()
  // The composer grew with the text: the panel wants more room.
  if (lastState !== null) measureAndReport(lastState)
}

export function focusComposer(): void {
  input.focus()
}

export function bindStatic(): void {
  fillStaticIcons()
  $('q-send').addEventListener('click', () => {
    if (lastState !== null) submitDraft(lastState)
  })
  $('q-close').addEventListener('click', () => onCloseRequested())
  $('q-settings-open').addEventListener('click', () => {
    if (lastState !== null) {
      if (page === 'settings') backFromSettings(lastState)
      else openSettings(lastState)
    }
  })
  $('q-settings-back').addEventListener('click', () => {
    if (lastState !== null) backFromSettings(lastState)
  })
  $('q-workspace').addEventListener('click', () => {
    if (lastState !== null) {
      if (lastState.session.workspaces.length === 0) void api.requestWorkspaces()
      toggleMenu(lastState, 'q-workspace')
    }
  })
  $('q-session').addEventListener('click', () => {
    if (lastState !== null) toggleMenu(lastState, 'q-session')
  })
  $('q-config').addEventListener('click', () => {
    if (lastState !== null) toggleMenu(lastState, 'q-config')
  })
  $('q-permission').addEventListener('click', () => {
    if (lastState !== null) toggleMenu(lastState, 'q-permission')
  })
  $('q-shortcut').addEventListener('click', () => {
    setRecording(!recordingHotkey)
    if (lastState !== null) renderSettings(lastState)
  })
  for (const chip of document.querySelectorAll<HTMLButtonElement>('#q-theme-chips .q-chip')) {
    chip.addEventListener('click', () => {
      void api.setPreferences({ theme: chip.dataset.theme })
    })
  }
  const keepOpen = $('q-keep-open')
  keepOpen.addEventListener('click', () => {
    const next = keepOpen.getAttribute('aria-checked') !== 'true'
    void api.setPreferences({ keepOpen: next })
  })
  input.addEventListener('input', updateDraftFromInput)
  document.addEventListener('click', event => {
    if (openMenu === null) return
    const target = event.target
    if (!(target instanceof Node)) return
    const trigger = $(openMenu)
    const menu = $(menuFor[openMenu] ?? '')
    if (!trigger.contains(target) && !menu.contains(target)) {
      closeMenus()
      if (lastState !== null) render(lastState)
    }
  })
}

export function renderState(state: Snapshot): void {
  lastState = state
  render(state)
}

export function bindCloseHandler(handler: () => void): void {
  onCloseRequested = handler
}

