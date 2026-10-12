// The panel's Simplified Chinese copy — key for key with `en.ts`, and the language
// the panel draws in.
//
// `Record<MessageKey, string>` is the enforcement: every key in `en.ts` must appear here
// (a missing one is a type error), and an extra key here is an excess property error
// because this is a fresh object literal. The two dictionaries cannot drift.

import type { MessageKey } from './en'

export const zh: Record<MessageKey, string> = {
  // Shared verbs.
  'common.close': '关闭',
  'common.gotIt': '知道了',
  'common.submitting': '提交中…',
  'common.requestExpired': '请求已失效',

  // The window itself.
  'panel.windowLabel': 'Quorfloat 对话窗口',
  'panel.hide': '收起悬浮窗',

  // Settings page.
  'settings.title': '悬浮窗设置',
  'settings.back': '返回对话',
  'settings.appearance': '外观',
  'settings.appearanceDetail': '明暗主题跟随系统或手动指定',
  'settings.hotkey': '呼出快捷键',
  'settings.hotkeyDetail': '点击后按下组合键',
  'settings.keepOpen': '失焦时保持展开',
  'settings.keepOpenDetail': '切换应用后仍保留悬浮窗',
  'settings.language': '语言',
  'settings.languageDetail': '面板界面的显示语言',

  // Theme chips.
  'theme.system': '跟随系统',
  'theme.light': '浅色',
  'theme.dark': '深色',

  // Language chips: endonyms, identical to the English dictionary on purpose.
  'language.zh': '中文',
  'language.en': 'English',

  // Workspace picker.
  'workspace.heading': '工作区',
  'workspace.choose': '选择工作区',
  'workspace.menuLabel': '工作区选择',
  'workspace.listPending': '工作区列表还没到。',
  'workspace.chooseHint': '选择工作区后，新会话在此开始。',
  'workspace.pinName': '工作区 {name}',
  // 不属于任何工作区的会话：只是一个说法，不是可选目标，也不会出现在工作区选择器里。
  'workspace.ungrouped': '未分组',
  // 宿主无法提供首个工作区，只能去 Harness 里建一个；仅在上报"请求已失败"时出现。
  'workspace.needHost': '还没有工作区，请先在 Harness 窗口里创建一个。',

  // Conversation (session) picker.
  'session.heading': '会话',
  'session.new': '新会话',
  'session.pickerLabel': '选择会话',
  'session.menuLabel': '会话选择',
  // 工作区字段还没解析出具体工作区时（启动或呼出的头一两帧），列表列的是全部会话。
  'session.scopeAll': '全部会话',
  'session.none': '没有可切换的会话。',
  // 列表已按某个工作区（或未分组）收窄，而其中没有任何会话。
  'session.scopeEmpty': '这里还没有会话。',
  'session.pinnedHint': '呼出时继续固定会话。',
  'session.unpinnedHint': '固定会话后，每次呼出继续此会话。',
  'session.pinName': '会话 {name}',
  // 固定失效的原因：线上只传 reason，句子在这里。
  'session.pinNotice.sessionGone': '固定会话已不存在，已取消固定。',
  'session.pinNotice.workspaceGone': '固定工作区已不存在，已取消该固定。',

  // Pin toggles.
  'pin.pinName': '固定{name}',
  'pin.unpinName': '取消固定{name}',

  // Model and reasoning effort.
  'model.pickerLabel': '模型与推理等级',
  'model.menuLabel': '模型与推理等级设置',
  'model.heading': '模型',
  'model.backRow': '‹ 模型与推理等级',
  'model.catalogPending': '模型目录尚未到达。',
  'model.effortHeading': '推理等级',
  'model.noEfforts': '当前模型没有可选档位。',

  // Permissions.
  'permission.pickerLabel': '选择权限',
  'permission.menuLabel': '权限选择',
  'permission.heading': '会话权限',
  'permission.catalogPending': '权限目录尚未到达。',

  // Composer.
  'composer.inputLabel': '输入问题',
  'composer.placeholderAsk': '问点什么…',
  'composer.placeholderFollowUp': '继续追问…',
  'composer.send': '发送问题',
  'composer.stop': '停止生成',

  // Footer key hints.
  'status.send': '发送',
  'status.newline': '换行',
  'status.close': '关闭',

  // Footer statistics.
  'stats.label': '会话统计',
  'stats.rounds': '{turns} 轮 {steps} 步 · {speed} tok/s',
  'stats.cacheHit': '缓存命中',

  // Approval card.
  'approval.title': '需要批准',
  'approval.tool': '工具 · {tool}',
  'approval.allowOnce': '允许一次',
  'approval.allowedOnce': '已允许一次',
  'approval.reject': '拒绝',
  'approval.rejected': '已拒绝',

  // Handoff banner.
  'handoff.approvalHandled': '审批已转交 Harness 窗口处理',
  'handoff.approvalNoSurface': '当前没有可以处理它的 Harness 窗口',
  'handoff.questionHandled': '这个问题已转交 Harness 窗口回答',
  'handoff.questionNoSurface': '当前没有可以回答它的 Harness 窗口',

  // Question card.
  'question.title': '需要你的回答',
  'question.submit': '提交',
  'question.submitted': '已提交',
  'question.answerAll': '请先回答所有问题',
  'question.notActionable': '这个问题请在 Harness 窗口回答',
  'question.hiddenCount': '还有 {count} 个问题没有显示。',
  'question.answerPlaceholder': '输入答案…',
  'question.otherPlaceholder': '其他答案…',
  'question.otherAnswerLabel': '{question} — 其他答案',

  // Answer bar.
  'answer.done': '回答完成',
  'answer.copy': '复制回答',
  'answer.copied': '已复制',
  'answer.copyManually': '请手动选择复制',

  // Turn fold and thread.
  'disclosure.working': '正在工作',
  'disclosure.done': '已完成',
  'thread.backToBottom': '回到底部',
  // 与 Harness 自己的用词一致（`chat.loadOlder` / 加载中的说法）。
  'thread.loadEarlier': '加载更早',
  'thread.loadingEarlier': '正在加载更早…',
  'turn.question': '你 · {question}',

  // Working lines.
  'working.tool': '工具',
  'working.call': '⚙ {name}',
  'working.resultNamed': '↳ {name}',
  'working.result': '↳ 工具结果',

  // Shortcut recorder.
  'hotkey.recording': '按下组合键…',
  'hotkey.unreadable': '读不懂这个快捷键：{chord}',
  'hotkey.bareKey': '裸按键会被整个桌面占用，请加上修饰键（Ctrl / Alt / Cmd / Shift）',

  // Relative times.
  'time.now': '刚刚',
  'time.minutesAgo': '{minutes} 分钟前',
  'time.date': '{month}月{day}日',
}
