// The panel's English copy.
//
// One flat key per user-facing string, with `{placeholder}` for the parts that vary.
// This file is the source of truth for the key union: `MessageKey` is `keyof typeof en`,
// and `zh.ts` is annotated `Record<MessageKey, string>`. That makes both directions a
// compile error — a key added here and forgotten in `zh.ts` is a missing property, and a
// key invented in a component is not assignable to `MessageKey`. The Chinese file carries
// the same keys; see `index.ts` for the lookup and its fallback rule.
//
// Vocabulary: 悬浮窗 is "floating panel" everywhere (the same noun the DSH-side entries
// use), 会话 is "conversation", 工作区 is "workspace", 推理等级 is "reasoning effort".

export const en = {
  // Shared verbs.
  'common.close': 'Close',
  'common.gotIt': 'Got it',
  'common.submitting': 'Submitting…',
  'common.requestExpired': 'Request expired',

  // The window itself.
  'panel.windowLabel': 'Quorfloat conversation window',
  'panel.hide': 'Hide floating panel',

  // Settings page.
  'settings.title': 'Floating panel settings',
  'settings.back': 'Back to conversation',
  'settings.appearance': 'Appearance',
  'settings.appearanceDetail': 'Follow the system theme, or pin one',
  'settings.hotkey': 'Summon shortcut',
  'settings.hotkeyDetail': 'Click, then press the key combination',
  'settings.keepOpen': 'Keep open when focus is lost',
  'settings.keepOpenDetail': 'Stay on screen after switching apps',
  'settings.language': 'Language',
  'settings.languageDetail': 'The language this panel draws in',

  // Theme chips.
  'theme.system': 'Follow system',
  'theme.light': 'Light',
  'theme.dark': 'Dark',

  // Language chips: endonyms, deliberately identical in both dictionaries. A reader who
  // cannot read the current language still has to recognise 中文 (or English) in the row.
  'language.zh': '中文',
  'language.en': 'English',

  // Workspace picker.
  'workspace.heading': 'Workspace',
  'workspace.choose': 'Choose a workspace',
  'workspace.menuLabel': 'Workspace selection',
  'workspace.listPending': 'The workspace list has not arrived yet.',
  'workspace.chooseHint': 'New conversations start in the chosen workspace.',
  'workspace.pinName': 'Workspace {name}',
  // The fact that a conversation belongs to no workspace. A label, never a target: it is not
  // offered in the picker and no conversation is created in it.
  'workspace.ungrouped': 'Ungrouped',
  // The host could not provide a first-use workspace, so only the Harness can. Shown **only**
  // when the shell asked and was refused (or failed).
  'workspace.needHost': 'No workspace yet — create one in the Harness window.',

  // Conversation (session) picker.
  'session.heading': 'Conversation',
  'session.new': 'New conversation',
  'session.pickerLabel': 'Choose a conversation',
  'session.menuLabel': 'Conversation selection',
  // What the conversation list is listing when the workspace field has not resolved to a
  // workspace yet (a frame or two at startup or summon): the whole list.
  'session.scopeAll': 'All conversations',
  'session.none': 'No other conversation to switch to.',
  // The list is scoped to one workspace (or to the ungrouped rows) and it holds nothing.
  'session.scopeEmpty': 'No conversations here yet.',
  'session.pinnedHint': 'The pinned conversation opens on every summon.',
  'session.unpinnedHint': 'Pin a conversation to keep opening it on every summon.',
  'session.pinName': 'Conversation {name}',
  // Why a half of the pin was forgotten. The wire carries the reason; the sentence is here.
  'session.pinNotice.sessionGone': 'The pinned conversation is gone; the pin was cleared.',
  'session.pinNotice.workspaceGone': 'The pinned workspace is gone; the workspace pin was cleared.',

  // Pin toggles.
  'pin.pinName': 'Pin {name}',
  'pin.unpinName': 'Unpin {name}',

  // Model and reasoning effort.
  'model.pickerLabel': 'Model & reasoning effort',
  'model.menuLabel': 'Model & reasoning effort settings',
  'model.heading': 'Model',
  'model.backRow': '‹ Model & reasoning effort',
  'model.catalogPending': 'The model catalog has not arrived yet.',
  'model.effortHeading': 'Reasoning effort',
  'model.noEfforts': 'This model offers no effort levels.',

  // Permissions.
  'permission.pickerLabel': 'Choose a permission',
  'permission.menuLabel': 'Permission selection',
  'permission.heading': 'Session permission',
  'permission.catalogPending': 'The permission catalog has not arrived yet.',

  // Composer.
  'composer.inputLabel': 'Ask a question',
  'composer.placeholderAsk': 'Ask something…',
  'composer.placeholderFollowUp': 'Ask a follow-up…',
  'composer.send': 'Send question',
  'composer.stop': 'Stop generating',

  // Footer key hints.
  'status.send': 'Send',
  'status.newline': 'Newline',
  'status.close': 'Close',

  // Footer statistics. The rounds group is labels rather than "3 turns" so a count of one
  // does not read as a grammar mistake; the dictionary has no plural form.
  'stats.label': 'Session statistics',
  'stats.rounds': 'Turns {turns} · Steps {steps} · {speed} tok/s',
  'stats.cacheHit': 'Cache hit',

  // Approval card.
  'approval.title': 'Approval required',
  'approval.tool': 'Tool · {tool}',
  'approval.allowOnce': 'Allow once',
  'approval.allowedOnce': 'Allowed once',
  'approval.reject': 'Reject',
  'approval.rejected': 'Rejected',

  // Handoff banner.
  'handoff.approvalHandled': 'The approval was handed to a Harness window.',
  'handoff.approvalNoSurface': 'No Harness window can handle it right now.',
  'handoff.questionHandled': 'The question was handed to a Harness window.',
  'handoff.questionNoSurface': 'No Harness window can answer it right now.',

  // Question card.
  'question.title': 'Your answer is needed',
  'question.submit': 'Submit',
  'question.submitted': 'Submitted',
  'question.answerAll': 'Answer every question first',
  'question.notActionable': 'Answer this one in the Harness window',
  'question.hiddenCount': '{count} more questions are not shown.',
  'question.answerPlaceholder': 'Type an answer…',
  'question.otherPlaceholder': 'Other answer…',
  'question.otherAnswerLabel': '{question} — Other answer',

  // Answer bar.
  'answer.done': 'Answer complete',
  'answer.copy': 'Copy answer',
  'answer.copied': 'Copied',
  'answer.copyManually': 'Select and copy manually',

  // Resources referenced by conversation Markdown.
  'markdown.image': 'Image',
  'markdown.imageUnsupported': 'This image address is not supported yet.',
  'markdown.loadImage': 'Load image',
  'markdown.imageLoading': 'Loading image…',
  'markdown.imageFailed': 'Could not load image',
  'markdown.retryImage': 'Retry',
  'markdown.linkUnsupported': 'This link address is not supported.',
  'markdown.openFailed': 'Could not open your browser. Copy the link and open it manually.',

  // Turn fold and thread.
  'disclosure.working': 'Working',
  'disclosure.done': 'Done',
  'thread.backToBottom': 'Back to bottom',
  // The Harness's own word for this control (`chat.loadOlder`), and its own busy wording.
  'thread.loadEarlier': 'Load earlier',
  'thread.loadingEarlier': 'Loading earlier…',
  'turn.question': 'You · {question}',

  // Working lines.
  'working.tool': 'tool',
  'working.call': '⚙ {name}',
  'working.resultNamed': '↳ {name}',
  'working.result': '↳ tool result',

  // Shortcut recorder.
  'hotkey.recording': 'Press the key combination…',
  'hotkey.unreadable': 'Cannot read this shortcut: {chord}',
  'hotkey.bareKey': 'A bare key would be captured from the whole desktop; add a modifier (Ctrl / Alt / Cmd / Shift)',

  // Relative times.
  'time.now': 'Just now',
  'time.minutesAgo': '{minutes} min ago',
  'time.date': '{month}/{day}',
} as const

/** Every dictionary key: derived from {@link en}, so neither language can drift. */
export type MessageKey = keyof typeof en
