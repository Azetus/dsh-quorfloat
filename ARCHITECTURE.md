# dsh-quorfloat — 产品与架构规范

> 本文描述**当前系统是什么、必须满足哪些约束**。
> 开发操作见 `AGENT.md`；版本与工程记录见 `CHANGELOG.md`；安装与使用见 `README.md`。
>
> 本文不复制会漂移的数值：版本以根 `package.json` 与 `quorfloat/Cargo.toml` 为准，界面令牌以
> `quorfloat/frontend/src/styles/tokens.css` 为准，测试数量以测试命令的输出为准。

## 1. 定位与边界

### 1.1 定位

`dsh-quorfloat` 是 DeepSeek Harness 的**系统级悬浮会话窗口**：一个 TypeScript 宿主插件
（`src/`，编译到 `lib/`）＋ 一个 Tauri 侧车（`quorfloat/`，Rust）＋ 一个 React 面板前端
（`quorfloat/frontend/`）。宿主与侧车通过 stdio 上的 NDJSON JSON-RPC 通信。

### 1.2 用户可见能力

- 全局快捷键呼出/收起；拖动窗口并记住位置；失焦收起或保持展开（可配置）；明暗主题与中英文界面。
- 浏览工作区与会话；新建会话（首次提交才创建）；固定会话或默认工作区；浏览未分组会话。
- 发送、停止生成；流式回答；Markdown、代码块、表格；一轮一层的工作过程折叠；加载更早的分页。
- 选择 Harness 提供的模型、推理档位与权限预设；查看当前会话的轮次/步数/速度/消耗/上下文统计。
- 在当前会话上处理工具审批与用户追问。
- 本地与远程 Markdown 图片自动显示（受大小、格式、路径与网络策略约束）。
- 通过设置页调整主题、语言、呼出快捷键与失焦行为；设置跨重启保存。
- 菜单栏/通知区托盘入口：显示、隐藏、重启、退出。

### 1.3 非目标

- **不修改 Harness 本体**。`src/harness/` 是唯一允许接触 Harness 服务细节的模块。
- **不是第二个 Harness 桌面客户端**。不另建 agent runtime，不复制 Harness 的会话存储，不持有模型凭证，
  不自行执行工具。
- 不提供多悬浮窗、语音、会话删除、文件上传、独立模型客户端。
- 不把剪贴板读取、工具专属折叠、后台事件时间线作为产品能力；这些方向已经明确否决，见 §11.2。
- 不承诺 Linux；平台支持见 §10。

### 1.4 分发形态

一个 Harness bundle，两种安装入口：

- **离线 bundle**：自包含目录/tgz，内部 `cordis.patch.yml` 指向自带侧车可执行文件；
- **npm 包**：主包 `dsh-quorfloat` 通过可选平台依赖获取当前平台的侧车，不在安装脚本里复制二进制。

两种入口共用同一套源码、协议与版本号。

## 2. 系统组成

### 2.1 进程与调用链

```text
React 面板 (WebView)
  │  Tauri invoke / event
  ▼
Rust 侧车 (quorfloat)
  │  stdin/stdout: NDJSON JSON-RPC 2.0
  ▼
TS 宿主插件 (src/bridge → src/harness)
  │  Cordis 服务调用
  ▼
Harness 的 sessionController / workspace / permission / interaction 等服务
```

结果与事件沿相反方向回到面板。侧车只负责窗口、输入与展示状态；会话执行、持久化、权限与工具结果
全部由 Harness 决定。

### 2.2 仓库职责

| 位置 | 职责 |
|---|---|
| `src/index.ts` | 插件装配、服务等待、激活/停用顺序、生命周期接线 |
| `src/config.ts` | 插件配置 schema、默认值、校验与归一化 |
| `src/host/` | 进程监管、二进制解析、presence/control 网关、语言种子 |
| `src/harness/` | Harness 边界：adapter、session-layer、审批/追问、presence 判定、工作区/会话列表推送、图片读取 |
| `src/bridge/` | stdio 通道、分帧、JSON-RPC 路由、图片传输、错误码 |
| `src/client/` | 运行在 Harness 网页中的浏览器半边：presence 上报、插件状态页与启动/停止入口 |
| `quorfloat/src/app/` | 侧车状态机：会话跟随、转录、交互卡片、窗口设置、偏好、pin、几何、高度、语言 |
| `quorfloat/src/ipc/` | 协议常量、分帧、RPC 形状、stdio 传输、平台命名 |
| `quorfloat/src/runtime/` | 全局热键、托盘、marker 与原始帧诊断 |
| `quorfloat/frontend/` | React 视图、hooks、纯逻辑、i18n 字典、设计令牌 |
| `scripts/` | 开发、构建、发布、CI 入口 |
| `tests/` | 协议/上游绊线/健壮性测试；`tests/desktop/` 为真实侧车桌面集成 |

### 2.3 权威边界

- **Harness** 拥有会话内容、执行状态、模型目录、权限预设与工具结果。
- **宿主** 拥有侧车进程生命周期、Harness presence 的聚合判定、平台包解析与配置基线。
- **侧车** 拥有窗口、热键、托盘、当前展示状态与本地偏好文件。
- **前端** 只是渲染器：快照是权威，组件不修改快照，所有写入经 `quorfloat/frontend/src/api.ts`。
- **转录归约在侧车**：前端拿到的是已经折叠好的视图模型，不重新解释原始 Harness 事件。

### 2.4 状态归属

| 状态 | 存放处 | 说明 |
|---|---|---|
| 会话、消息、工具结果、权限、模型 | Harness | 侧车不落盘，不复制 |
| 进程状态、重启计数、最后错误 | 宿主 supervisor（内存） | 通过 `quorfloat/status` 暴露给页面 |
| 窗口位置 | `~/.dsh-quorfloat/window.json` | 可被 `DSH_QUORFLOAT_WINDOW_STATE` 重定向 |
| 固定项（会话/工作区） | `~/.dsh-quorfloat/pinned.json` | 可被 `DSH_QUORFLOAT_PINNED` 重定向 |
| 面板偏好（主题/语言/快捷键/keepOpen） | `~/.dsh-quorfloat/preferences.json` | 可被 `DSH_QUORFLOAT_PREFERENCES` 重定向 |
| 当前页面、打开的菜单、草稿 | 前端内存 | 呼出时按规则复位；草稿跨呼出保留 |

用户真实状态目录只读是硬约束：测试必须重定向到临时目录，不得写入 `~/.dsh-quorfloat/`。

## 3. 生命周期与进程监管

### 3.1 激活顺序

1. 读取并校验插件配置；配置错误在插件状态中报告，不拖垮共享 composition。
2. 通过 Cordis `inject` 等待 `sessionController` 可用。**不能在 `apply` 时一次性探测**：服务按可用性
   顺序装配，早探测必然得到“缺失”。
3. 组装 adapter、session layer、审批/追问 answerer、presence 与 control 网关。
4. 解析侧车路径，启动进程，交给 supervisor 监管。
5. 侧车先发 `hello`，宿主接受后回 `ready` 并下发有效配置。

停用按相反顺序：停止接受新工作、撤回待决交互、释放订阅、停止进程并确认退出。

### 3.2 启动与就绪

- 侧车的第一帧是 `hello`，**在窗口创建之前发出**；宿主用启动预算判断“二进制坏了”还是“起得慢”。
- `hello` 携带协议版本、产品版本、平台、架构、能力表与热键状态。
- 宿主回复后发送 `ready` 通知，携带有效配置（窗口尺寸/锚点/主题/语言/心跳等）。
- 只有握手成功才进入 `running`；进程存在但未握手是 `starting`，不是可用。

### 3.3 心跳与故障

- 宿主按 `heartbeatMs` 发送 `ping` **请求**并等待答复；同时发送 `host/heartbeat` 通知。
- `ping` 是请求而不是通知：通知只能证明“写成功了”，请求才能证明对端还在处理。
- 连续丢失达到阈值进入 `degraded`；仍未恢复则按重启预算处理。
- 侧车通过 stdin EOF 与宿主消息活性判断失联；stdout 损坏时不能继续维持一个看似可操作的界面。

### 3.4 重启与停止

- 崩溃/意外退出按有上限的退避重启；达到阈值后停止自动重启，保留手动重试入口。
- `supervisor.stop()` 是唯一把退出标记为“故意”的路径；任何其他退出都会被重启策略当成崩溃。
- 用户从托盘选择“退出”或面板请求 `panel/lifecycle {action:"stop"}` 时，必须走故意停止路径。
- 每个宿主实例最多一个侧车；多 profile 可以各自运行，但必须使用不同热键或关闭自动注册。

### 3.5 退出

1. 宿主发送 `shutdown` 请求，侧车答复后退出；
2. 超过 `shutdownGraceMs` 升级到终止信号；
3. 观察实际进程退出结果；调用 kill 或收到“正在退出”的回复都不等于清理完成；
4. 宿主退出时清理热键、托盘、窗口；侧车 EOF 后立即进入退出流程。

### 3.6 托盘

macOS 菜单栏 / Windows 通知区提供：显示/隐藏、重置窗口位置、重启、退出。重置只移动窗口到当前
配置的 anchor，不改变显示状态或焦点；重启/退出的真正执行在宿主侧（`panel/lifecycle` 或
`quorfloat/panel`），托盘只发意图。托盘创建失败不应阻止热键呼出。

### 3.7 面板自重启

面板托盘菜单通过 `panel/lifecycle` 请求宿主重启或停止自己。该请求的答复通常送不到（进程正在被
替换/杀掉），调用方按 fire-and-forget 处理；宿主仍先完成动作再返回，且不因对端已消失而抛错。

## 4. 跨进程协议

### 4.1 传输与分帧

- JSON-RPC 2.0，UTF-8，**每行一个 JSON 值**（NDJSON）；字符串内的换行由 JSON 转义。
- 子进程 **stdout 只承载协议帧**；日志一律走 stderr。宿主会吞掉 stderr，因此事后可查的是 marker。
- 读取按字节流处理：一帧可分多次到达，多帧可合并在一个 chunk。
- 单帧上限 **1 MiB**；超限帧被丢弃并计数，不拖垮连接。
- 协议版本 `quorfloat/1`，与产品版本、Harness 的 host protocol 版本相互独立。

### 4.2 握手与能力

`hello` 的 `capabilities` 是**二进制属性**：要么整类都能处理，要么不声明。当前为
`window`、`hotkey`、`tauri`、`approval`、`question`。宿主只把对端声明能渲染的交互类别交给它。
能力同时会在 `window/visibility` 上报中复述一次；`hello` 发生在窗口存在之前，那里的 `window`
只能算声明，创建成功后的上报才是实测。

### 4.3 方法总表

**侧车 → 宿主（请求）**

| 方法 | 作用 |
|---|---|
| `hello` | 握手；协议/产品版本、平台、架构、能力、热键状态 |
| `workspaces/list` | 工作区列表 `{items:[{workspaceId,title,path}]}` |
| `workspace/default` | 请宿主解析/创建首次使用的默认工作区；回答与 `workspaces/list` 同形 |
| `sessions/list` | 会话列表；行内可带 `cwd`、`updatedAt`、`running`、`blank`、`title` |
| `session/create` | `{workspaceId}`；`workspaceId` 必须是非空字符串 |
| `session/attach` | `{sessionId,fromSeq?}` → `{sessionId,generation}`；认领审批的前提 |
| `session/prompt` | `{sessionId,requestId,text}`；`requestId` 是幂等键 |
| `session/cancel` | 停止当前回合 |
| `session/history` | `{sessionId,beforeSeq}` → 一页更早的记录 |
| `session/snapshot/request` | 请求重发某个未收齐的分块快照 |
| `session/image` | 有界读取本地图片，按 256 KiB 原始字节分块返回 |
| `session/options` | 模型目录、权限预设、当前选择；`sessionId` 可选 |
| `session/select` | `{sessionId,provider,model,reasoningEffort?}`，模型与档位一起送 |
| `session/permission` | `{sessionId,value}`，写权限**预设** |
| `interaction/answer` | `{interactionId,answer}`，审批/追问的统一答复入口 |
| `panel/lifecycle` | `{action:"restart"\|"stop"}`，面板请求宿主处理自己的进程 |
| `diag/snapshot` | 诊断快照；宿主侧注册，正常 UI 不调用 |
| `presence/report` | stdio 的 presence 上报，供测试或非浏览器 peer 使用 |

**侧车 → 宿主（通知）**

| 方法 | 作用 |
|---|---|
| `window/visibility` | `{visible,capabilities}`；面板每次显隐都上报，首次隐藏也报 |
| `session/detach` | 离开当前会话；不通知会让宿主继续按旧会话判定审批归属 |

**宿主 → 侧车（请求）**

| 方法 | 作用 |
|---|---|
| `ping` | 心跳；必须答复 |
| `shutdown` | 请求退出；侧车答复后以 0 退出 |

**宿主 → 侧车（通知）**

| 方法 | 作用 |
|---|---|
| `ready` | 握手后的有效配置（窗口/主题/语言/心跳等） |
| `host/config` | 运行中推送配置变更，走与 `ready` 相同的解释路径 |
| `host/heartbeat` | 宿主活性信号；侧车不回复 |
| `window/visibility` | 显示/隐藏命令；接受 `{visible}` 或 `show`/`hide`/`toggle` |
| `session/snapshot` | 单帧会话快照 |
| `session/snapshot/begin` / `part` / `end` | 大快照的运输分块 |
| `session/event` | 持久事件（按 seq 恢复/去重） |
| `session/stream` | 临时 assistant 流 |
| `session/resync` | 历史出现空洞；侧车应重新订阅 |
| `session/history/begin` / `part` / `end` | 大历史页的运输分块 |
| `session/stats` | 当前会话统计；附着时与每回合结束后推送 |
| `workspaces/changed` | 工作区新增/改名/删除/排序的推送 |
| `sessions/changed` | 会话列表推送：`upsert`（整行）/ `status` / `activity` / `title` / `refresh` |
| `interaction/open` | 审批或追问到达 |
| `interaction/hint` | 该请求没有交给面板，附带仍活跃的界面列表 |

**浏览器 → 宿主（Cordis 远程端点）**

| 端点 | 作用 |
|---|---|
| `quorfloat/status` | 只读监督状态：state、pid、restarts、restartExhausted、seen、visible、lastError |
| `quorfloat/panel` | 写：`{action:"start"\|"stop"\|"restart"}`；与 `panel/lifecycle` 共用同一执行路径 |
| `quorfloat/reportPresence` | `{surface,visible,focused,seq,at?}`；浏览器半边的 presence 上报 |
| `quorfloat/diagnostics` | 零参数；把当前状态快照与最近的宿主事件/侧车 stderr 写成 `.log`，回答 `{path,filename}` |

### 4.4 请求语义

- 请求带整数 `id`，回答 `result` 或 `error`；通知没有 `id`，也永远不回答。
- 错误码：`-32700` 解析、`-32600` 非法请求、`-32601` 未知方法、`-32602` 非法参数、`-32603` 内部、
  `-32001` 协议不匹配、`-32002` 超时、`-32003` 通道关闭、`-32004` 不可用、`-32005` 视图过期、
  `-32006` 协议违例。
- 超时**不等于远端没有执行**；调用方必须按幂等语义处理。
- 迟到、重复、已取消或过期的交互答复不得重新执行；第二次答复返回 `accepted:false`。
- 侧车断连不能自动批准待决操作；重连后重新获取仍有效的交互。
- `panel/lifecycle` 的答复允许丢失；其余请求不允许把“没答复”解释成“没执行”。

### 4.5 幂等键

`session/prompt` 的 `requestId` 是 Harness 的幂等键：Harness 拿它去会话历史里比对
`user/message.source.rpcId`，命中就返回 `{accepted:true}` 而**不受理**。因此键必须
**每次尝试不同、跨进程也不同**：`{sessionId}:prompt-{pid}-{毫秒}-{进程内全局序号}`。

宿主另有一道闸：从 follow 帧里攒下该会话用过的键，收到“已花掉的键”时拒绝，并把一条行动在前的
中文原因放进错误消息。侧车把原因显示在状态行并写 marker，不转给 Harness 静默丢弃。侧车在发送时
清空输入框；若拒绝晚于清空到达，且输入框为空，则把原文放回。

### 4.6 快照、事件与转录

- `session/snapshot` 是单帧全量快照；超过运输预算时改为 `begin` + `part` + `end`。
- 分块快照按 `(sessionId,generation)` 装配，**收齐才折叠**；缺片时发 `session/snapshot/request`，
  重发次数有上限（2 次），装配超时 10 秒，最多同时打开 4 个装配。
- 丢弃必须可见：超时、被新的 `begin` 取代、打开的装配过多都写 marker，不静默变空白。
- `session/event` 是持久事件，按 seq 恢复与去重；`session/stream` 是临时流，不落历史。
- `session/resync` 表示历史有洞；侧车记录并重新订阅，不能把不完整状态当完整。
- 当通知与分片共用一条通道时，发送端必须**串行化**（逐帧 await），否则后来的事件会越过分片，
  对端把事件判为过期而不渲染。

转录归约的输入形状以实测为准：

- 消息是 Anthropic 风格内容块：`text` / `reasoning` / `tool-call`；
- `user/message` 的 content **不套** `message`，其他记录类型套；按统一形状读会丢掉每条用户消息；
- `tool-call` 的 `arguments` 是 JSON **字符串**，不是对象；
- 每条记录文本有 32 KiB 上限，截断会写在文本里；
- 用户消息按 `source.kind` 判别：`user` 才是用户说的，`runtime-context` 是 Harness 注入；
  `system/message` 是系统提示词；没有 `source` 的消息按用户消息处理；
- 被审批拦下的工具调用表现为普通失败，不能读作“用户拒绝”；
- 后台事件（`approval/*`、`session/resync`、未识别的 kind 等）不产生对话行，只计数并把
  kind 名字写进 marker。

### 4.7 历史分页

- 初始快照与后续页都按**整轮**对齐：`maxMessages:200`、`turnWindow:{minMessages:50,minTurns:2}`。
- Harness 达到消息上限时仍可能切在轮中；adapter 继续向前读，补齐到同一轮的 `turn/start`，
  不向前端暴露半轮。
- 整页超过 **512 KiB** 时拆成约 256 KiB 的运输片，通过 `session/history/begin|part|end` 发送；
  侧车收齐整页后一次前插。
- 响应形状：`{records,hasMore,cursor}`；分块时先发通知，再答
  `{records:[],hasMore,cursor,chunked:true,generation}`。
- 前端只在前插后按**行锚点**补偿滚动位置：取目标行的真实起点差，不用总高度差；用户主动滚动
  或点“回到底部”时释放锚点。

### 4.8 图片传输

- `session/image` 首次请求 `{sessionId,path}`，宿主验证会话、按会话 cwd 解析相对路径，
  通过 Harness 的文件系统服务有界读取并识别格式；返回 `{token,offset,nextOffset,totalBytes,mimeType,data,done}`。
- 后续请求带 `token` 与 `offset`，每次最多 256 KiB 原始字节的 Base64；`release:true` 主动释放。
- 单图上限 `min(Harness maxImageBytes, 8 MiB)`；支持 PNG/JPEG/WebP/GIF，按内容头识别。
- 每通道最多 2 个在读/快照，30 秒过期；完成/取消/失败主动释放；侧车独立在途表最多 4 个，
  12 秒超时清理，不占 follow/prompt 槽。
- 绝对路径允许在会话 cwd 之外；拒绝 `file://`、UNC、`~`、控制字符；路径只 URL 解码一次。
- 图片数据不进入会话快照，也不写用户状态文件。

### 4.9 模型、权限与统计

- `session/options` 的回答是**宿主投影过的形状**：模型按 `provider` 分组（不是 `group.id`），
  档位来自当前模型的 `efforts`；权限给 `value/name/description`；当前值可缺省。
- `session/select` 同时送 `provider`、`model`、`reasoningEffort?`：上游把三者放在同一个
  `ModelSelection` 里，分两次送会短暂登记模型可能不接受的档位。
- `session/permission` 写的是**权限预设**，同时决定沙箱范围与审批策略；不是裸的 sandbox mode。
- `session/stats` 由宿主主动推：附着时一次，此后每个 `step/end` / `turn/end` 后一次。侧车只接受
  当前会话的推送，换会话即清零。
- 统计值算不出来时不伪造 0；格式规则见 §8.9。

### 4.10 限额汇总

| 项 | 限额 |
|---|---|
| 单帧 | 1 MiB |
| 历史页运输预算 | 512 KiB |
| 快照/历史分片 | 约 256 KiB |
| 单条记录文本 | 32 KiB |
| 转录窗口 | 4 MiB 文本（按整轮回收） |
| 单图 | min(Harness 上限, 8 MiB) |
| 图片分块 | 256 KiB 原始字节 |
| 图片并发 | 前端 2 个读取、侧车 4 个在途 |
| 快照装配 | 10 秒、最多 4 个、重发 2 次 |
| 单页消息 | 200 条 / 最少 50 条、2 轮 |

## 5. 会话、工作区与标题

### 5.1 一个 pin，两个投影

- **会话 pin 是唯一的“下次呼出打开哪个”**；工作区字段是它的投影。pin 一个会话时，工作区字段
  同步改写为该会话所属工作区；匹配不到就置空，不猜。
- **工作区 pin 只在新会话态存在**：它表示默认创建位置，不是“继续某个会话”。
- 只有用户手势能设置/更改/取消 pin；程序只在失效时清理失效的那一半。
- 失效判定必须保守：列表未应答 = 无证据；会话不在列表需连续两次已应答；工作区不在列表一次即可；
  attach 被拒只认明确错误码且每次访问至多一次。
- 清理必须能解释：侧车给结构化原因，前端字典渲染文案，侧车不产出面向用户的句子。

### 5.2 新会话的工作区阶梯

| 顺位 | 取值 | 条件 |
|---|---|---|
| P0 | 本窗上次显式选择的目录（持久化 `workspaceId`） | 已校验有效 |
| P1 | 当前活动会话的目录 | 该会话仍在运行 |
| P2 | first-use 默认工作区 | 工作区注册表为空时由宿主决定 |
| P3 | 最近活跃的工作区 | 按 `updatedAt` 聚合 |
| P4 | 打开选择器让用户选 | 以上皆空；协议不允许空 `workspaceId` |

阶梯由 Rust 在真正创建时裁定（`app/workspace.rs`），面板送来的选择只要仍在宿主列表里就优先。
快照的 `session.createWorkspace` 是同一解析在“面板自己没有选择”时的结果，顶栏读它，因此字段与
创建不会各说各话。

### 5.3 呼出与本次访问

- 热键呼出默认落在**新会话态**：不立即创建，首次提交才创建；有可用会话 pin 时打开它。
- 面板内的选择只在本次可见期有效；隐藏后回到上述规则。输入框草稿跨呼出保留。
- 页面、打开的菜单、录制状态、本地错误消息在 `App.onPanelShown` 统一清零；线程阅读位置与折叠
  状态属于对话本身，不在复位范围。
- 召唤用 `window.summon = {token,count}` 判定：token 标识侧车生命周期，同一 token 内 count 递增
  才算召唤；面板重载/侧车重启不能被误判成召唤。

### 5.4 未分组浏览

- 工作区选择器在真实工作区之后追加“未分组”，标签来自前端字典；它没有 pin 按钮。
- “未分组”是 `{kind:'ungrouped'}` 浏览范围，不是工作区实体，不能作为新建目标。
- 会话归属只按工作区注册表的 `sessionIds` ∩ 可见会话列表推导；不比较 cwd。
- 未分组范围隐藏“新会话”入口；已有会话仍可选择、pin。
- 选择真实工作区、点击具体会话、跟随会话变化或再次呼出时，显式浏览结束，恢复自动范围推导。

### 5.5 标题

`sessions/list` 的行没有顶层标题；标题在 `summary.projections.values.title`。宿主直接读它；
只有“投影缺失或为 null”且该行 `updatedAt` 相对上次读取已移动时，才用 cold-safe 的 `inspect`
折叠 `session/title`（最后一条胜出）。每次列表最多读 1 条，读不到就不显示名字，不猜字段。

标题的实时更新走 `sessions/changed` 的 `title` 帧：宿主订阅 `sessionProjections.onChanged`，
只取 key 为 `title` 的变更；投影服务缺席时该帧不产生，标题随列表对账刷新。

### 5.6 列表刷新

- 会话列表由宿主推送维护：宿主订阅 Harness 的 `api-session/added|removed|status|activity`
  与标题投影，投影成 `sessions/changed` 的 `upsert` / `status` / `activity` / `title` / `refresh`。
- 侧车把推送合并进同一份列表，字段只按不会错的方向移动：名字出现但不消失、`cwd` 只补不换、
  `updatedAt` 只前进、`blank` 只降不升（`running=true` 即不再为空）。排序始终按 `updatedAt` 降序。
- `sessions/list` 答复在途时到达的推送先立即应用、再缓存，答复落地后按序重放；`refresh`
  （`removed`：离开活注册表，不等于从持久列表消失）不删行，而是立即请求权威列表。
- 每 30 秒做一次全量对账作为兜底；握手完成与面板呼出（`begin_visit`）也各请求一次。
  答复超时或帧写入失败按 3 秒重试；断线/重连由新进程的首帧全量覆盖。
- 工作区列表同样不靠轮询：宿主跟随 Harness 的 `workspaceController.follow`，把新增/改名/删除/排序
  作为 `workspaces/changed` 推给侧车，并保留一次连接期基线与回退路径。每次变更写一行低频 marker。

## 6. 审批与追问归属

### 6.1 两类请求

| | 审批 `approval/request` | 追问 `user-questions/request` |
|---|---|---|
| 触发者 | 沙箱升级：工具调用要求更宽权限 | 模型主动提问 |
| 答案形状 | `allowed-once` / `rejected` / `cancelled` / `unavailable` | `{answers:[{id,selected,custom?}]}` |
| 取消词项 | 有 | 无；只能交还给下一个答复者 |
| 默认可用性 | 默认审批策略下可用 | `standard` preset 插入提问工具 |

### 6.2 presence 判据

归属只认：**界面可见且 `document.hasFocus()`**。`document.visibilityState` 单独使用有平台差异：
被完全遮挡时 macOS 报 `hidden`、Windows 报 `visible`；别的桌面两平台都可能报 `hidden`。

| 情形 | 归属 | 面板行为 |
|---|---|---|
| Harness 界面可见且有焦点 | Harness 本体 | 不领取；对追问发指向性提示 |
| 界面可见但无焦点 | 面板 | 领取审批/追问卡片 |
| 界面隐藏 | 面板 | 同上 |
| 两个界面都不可见 | 不领取 | 不呈现；记录原因 |

浏览器的报告是**存活声明**：变化时立即上报，另外每 5 秒重发一次；宿主按
`presenceMaxAgeMs`（默认 30 秒）让过期报告失效。缺了心跳，活着的页面会在 30 秒后被判为
“没在看”，归属翻给面板。

### 6.3 认领条件

面板认领一个交互必须同时满足：

1. 该会话是本宿主在跟踪的；
2. presence 判定为面板；
3. 面板当前显示的正是这个会话（最近一次 `session/attach`，`session/detach` 时清空）；
4. 对端在 `hello.capabilities` 里声明了该类别的能力。

追问还有形状边界：一次请求 1–4 个问题、每题有非空 `id` 与题干、每题选项 ≤12 个。超出边界的卡片
仍可显示，但 `actionable:false`，不声明可答；答案必须覆盖用户被展示过的内容，宁可整单交还。

### 6.4 排他性

审批是 agent 作用域的 waterfall，`dsh-api-remotes` 会注册一个把请求转发给已连接浏览器、且不调用
`next()` 的监听器。本插件必须用 `{prepend:true}` 排到它之前才能拿到请求；一旦领取，该请求**不再
转发给 Harness 界面**，两者不会同时显示。这是刻意取舍：本插件是 Harness 的附属窗口，不是第二个
桌面客户端。

### 6.5 答案形状

- 一张卡片一次提交：`interaction/answer` 的 `params.answer = {answers:[…]}`。
- `selected` 装选项的 **label**（不是 id），顺序按选项给出的顺序，不按点击顺序。
- `custom` 只在用户真的输入了自由文本时出现；空白不算。
- Rust 侧再校验：id 属于本卡片、label 是本题提供过的、每题至少选了或写了；不合格拒绝并写 marker。
- 迟到/重复/过期答复不重新执行；第二次返回 `accepted:false`。

### 6.6 提示与失效

`interaction/hint` 只表示“这个请求没有交给面板”，因此不会伴随领取出现。它的 `surfaces` 字段
说明宿主认为哪些界面仍活跃：有活跃界面才谈得上“请切到 Harness 窗口”，否则只能如实说“当前没有
可以回答它的 Harness 窗口”。

## 7. 设置与本地状态

### 7.1 基线

宿主配置是基线：`config.window`（宽、最大高、锚点、置顶、减少动画、主题、语言）与
`config.hotkey` 等。配置经 `cordis.patch.yml` 的 `quorfloat` 行进入，不读私有配置文件；
patch 整体替换 `config`，未知字段报错而不是静默取默认值。

### 7.2 偏好覆盖

面板设置页写入 `~/.dsh-quorfloat/preferences.json`，叠加在宿主配置之上：

| 字段 | 含义 |
|---|---|
| `theme` | `system` / `light` / `dark` |
| `keepOpen` | 失焦时保持展开；与配置的 `hideOnBlur` 是同一事实的两种措辞，在边界处换算一次 |
| `hotkey` | 快捷键 spec 字符串，与宿主配置、`hello` 报告同一个拼写 |
| `language` | `zh` / `en`；首次启动可从 Harness locale 种子，之后不再跟随 |

规则：文件缺失/损坏/读不懂 = 用户没决定过，宿主配置继续生效；认不出的值不是值；一条都没设就删文件，
而不是写 `{}`。写入走同目录临时文件 + `sync_all` + 原子 `rename`；失败只写 marker，不阻止面板运行。
已有符号链接会解析到真实目标后替换，链接本身保留；Unix 权限随旧文件保留。

### 7.3 其他状态文件

- `window.json`：窗口位置；启动时若记忆位置不在任何已连接屏幕内，回退到主屏工作区的锚点。
  每次显示前还检查顶部拖动条：它在某个显示器的 **work area**（已排除菜单栏、程序坞和任务栏）中
  至少有 **64×24 logical px** 可见才保持原位；不足则回退。不增加显示器轮询。
- `pinned.json`：会话 pin 与工作区 pin；失效清理必须能解释，且只清失效的那一半。
- `logs/`：手动导出的诊断 `.log`；每次导出一个文件，不自动清理。
- 三个状态文件只由侧车读写；诊断日志由宿主写入；测试必须重定向到临时目录。

### 7.4 多语言

- 面板只支持中文/英文，沿用 Harness 的 `zh` / `en` 标识，不引入第三套词汇。
- 所有用户可见文案来自 `quorfloat/frontend/src/logic/i18n/` 的两种字典；key 类型由字典推导，
  漏 key 是编译错误。
- 用户可见的“侧车”一律说“悬浮窗”；`侧车`/sidecar 只留在代码、标识符、注释、locale 命名空间与日志。
- 面板与 Harness 客户端半是两套字典、两个构建管线，但“运行中 / 已停止 / 失败 / 悬浮窗”等状态词
  必须同口径，改一处要同时改另一处。

### 7.5 开发隔离

`npm run dev` 把偏好、pin、窗口状态与 marker 指向 `.dev-home/`，避免一次开发运行污染开发者真实的
`~/.dsh-quorfloat/`。开发 URL 由 `DSH_QUORFLOAT_DEV_URL` 指定时，WebView 只允许加载同一文档。

## 8. 界面契约

### 8.1 窗口

- 宽度固定 640；高度随内容变化，默认最大 560，可由配置调整。
- 透明、无边框、无系统阴影、置顶、不占任务栏；macOS 为 accessory 应用（无 Dock 图标）。
- 呼出动画 180ms：透明度 0↔1、上移 6px、缩放 .985；关闭过渡完成才隐藏原生窗口。
- `reduceMotion` 立即切换，支持中途反向；隐藏后不再动画重绘。
- 高度协调：前端测量内容 → 申请原生空间 → 等待兑现（有期限）→ 收缩时先缩内容再缩窗口。
  平台始终不给够高度时，改用窗口实际高度并在内部滚动，写一次 marker；窗口之后变大再长回去。
- 收起时把激活交还上一个应用：accessory 应用会 deactivate 用户原来的应用，只 `window.hide()`
  不够；macOS 上还必须 `[NSApp hide:nil]`。

### 8.2 布局

- 顶部：工作区、会话、齿轮；可拖动区域与可点控件分离。
- 对话区：问题、折叠的工作过程、最终回答、回答栏；线程只挂载视口内的行。
- 底部三组只属于对话页：权限（左下）、模型与档位（右下），统计（右下）。
- 面板自己的一行（键位提示 + 状态行）在设置页也保留：`esc 关闭` 在设置页同样成立，
  “设置没生效”也需要被看到。
- 设置页**替换**对话，不是覆盖；进入设置再返回保留该会话的阅读位置。
- 菜单按入口下方的可用高度限高，标题固定，条目内部滚动；不为显示列表把空对话面板撑高。

### 8.3 输入

- 输入区是一行且顶对齐；多行时自动增高到上限；输入字号与回答正文一致（14px），
  发送键 34×34，放大镜对准第一行视觉中心。
- Enter 发送，Shift+Enter 换行，Esc 先关闭菜单/返回对话页，再收起面板。
- IME 组合输入期间 Enter 不得发送。
- 生成中发送键变为停止；停止后保持“取消中”直到宿主报告结果。
- 发送即清空输入框；被拒绝时若输入框为空，把原文放回，原因留在状态行。
- 输入框长高时兑现“读者在看哪儿”的承诺：跟随底部就钉在底部，滚上去读就补回滚动偏移。

### 8.4 对话线程与折叠

一轮由三段组成：**问题**、**一个折叠项**、**最终回答**。

- 折叠项收纳所有“不是最终回答”的 LLM 产出：推理、工具调用与结果、中间叙述。
- 最终回答 = 本回合**最后一条**有正文、且不含 tool-call 的 assistant 消息；以工具调用结尾的消息
  还在工作，不是回答。
- 回答消息自带的推理进折叠，正文留在外面；一轮只有一层折叠，不再有第二个“思考”折叠。
- 折叠默认收起；**只有读者能开合**，程序从不替读者展开/收起。进行中显示 `▸ 正在工作 ◌`，
  结束后 `▸ 已完成`；两种状态都可点开。
- 折叠状态的键 = 这一轮的问题 + 它在“相同问题的轮次”中的序号（从最新往前数）。不能用位置当键：
  旧页前插会让位置整体前移；同一句话问两次也不能共用一个键。
- 后台事件不产生行；未识别 kind 只计数并写 marker。
- 回答栏（状态 + 复制回答）属于回答，不属于折叠；复制的是原始 Markdown。
- 流式期间不显示“正在生成”：正文在长高，下面那行只会抖；“正在工作”由折叠行表达。

### 8.5 分页与滚动

- 初始快照按整轮请求，后续页用 `beforeSeq` 向前；页请求形状与预算见 §4.7。
- “加载更早”只在 `hasMore` 且滚动区位于顶部（`scrollTop ≤ 1px`）时出现；底部/中段隐藏；
  短会话无滚动也算到顶。加载中显示忙碌文案并禁用，离开顶部即隐藏。
- 零高度槽里的元素不会自己得到高度：槽必须不参与内容测量，子项要么有确定尺寸，要么在槽上
  使用 `align-items: flex-start` 并钉住 `overflow: visible`；否则 `stretch` 会把按钮压成负向高度。
- 前插后用**行锚点**补偿：取保留行的真实起点差，不用总高度差；测量在补偿之前完成，
  保留行继续挂载，直到用户主动滚动。
- “回到底部”释放锚点；它自己的补偿滚动不能被当成用户跟随尾部的手势。
- 线程窗口化：只挂载视口内的行；每帧重读已挂载行的高度，避免未挂载行变矮后留下空白。
- 读者主动向上滚动后停止强制跟随；流式内容增长不能把读者按尾部增长量推走。

### 8.6 Markdown、链接与图片

- Markdown 由 markdown-it 渲染，DOMPurify 在注入前消毒；原始 HTML 按文本展示。
- 表格在回答区域内横向滚动，不撑破面板；长 ASCII 串软换行。
- 网页链接只接受完整 HTTP(S) 地址，交给系统默认浏览器；页内锚点保留；点击与中键都拦截窗口内导航。
- 图片：
  - 远程 HTTP(S) 图片与本地图片都自动显示，不再需要点击确认；用户已接受浏览会话可能触发第三方请求。
  - 远程图片 `loading=lazy`、`decoding=async`、`no-referrer`；失败显示字典提示并可重试。
  - 本地图片按会话 cwd 解析相对路径；绝对路径允许在 cwd 外；不支持 `file://`、UNC、`~`；
    路径只 URL 解码一次，文件名里的 `%`/`#`/`?` 不被二次解释。
  - 前端全局最多 2 个活跃读取，等待项取消后不再读取；完成的 Blob URL 在组件离开时释放；
    旧会话结果不得进入新会话。
  - 图片不超过回答区域宽度；无放大浮层；点击图片或加载按钮不激活外层 Markdown 链接。
- CSP 限制脚本、字体、对象、框架、表单与 base；`img-src` 允许 self、HTTP(S) 与 Blob；
  保留 Tauri IPC、布局内联样式与开发环境的 Vite WebSocket。
- 主 WebView 只允许内置主页面及其页内锚点；指定开发 URL 时只允许同一文档；其他导航和新窗口拒绝。
  内置首页是 `tauri://localhost`，其 path 为空串，白名单必须接受这一形式。

### 8.7 审批与追问卡片

- 审批卡：工具名 + 理由 + `允许一次` / `拒绝`；提交中/已裁定/已失效三态；只有一个实心按钮。
- 追问卡：单选、多选、自由文本；一张卡片一次提交；答不了的请求保持“请在 Harness 窗口回答”，
  不声明 `actionable`。
- 卡片区可滚动（`flex: 0 1 auto; min-height: 0; overflow-y: auto`）；动作行 sticky 在底部，
  卡片再高也按得到提交。
- 卡片使用与面板相同的设计令牌，不引入第二套按钮/颜色词汇。

### 8.8 设置页

替换对话的独立页面，每行一条设置：标签与说明在左、控制在右、行间 1px 分隔线。

| 项 | 行为 |
|---|---|
| 外观 | 跟随系统/浅色/深色；立即生效并持久化 |
| 界面语言 | 中文/英文；立即生效并持久化 |
| 呼出快捷键 | 设计稿的框本身就是控件：点一下开始录制；Esc/Enter 在录制期间让位；裸字母/数字/标点被拒，功能键例外 |
| 失焦时保持展开 | 开关；与 `hideOnBlur` 在边界处换算一次 |
| 导出诊断日志 | 把当前状态快照与最近的宿主事件/侧车 stderr 写到 `~/.dsh-quorfloat/logs/`，成功后显示绝对路径；每次手动导出一个文件，不自动清理 |

导出包含当前状态与最近日志：supervisor、配置、服务探测、会话/交互摘要、审批归属，以及
宿主 supervisor 事件环和侧车 stderr 环。会话/交互 ID 截短、home 路径替换为 `~`；不包含
对话正文、凭证、DSH 自身日志或仅 dev/CI 才有的 marker。快照区使用扁平文本，不使用 JSON：
普通对象一行一个 `key: value`，对象数组写成 `name (count):` 后每项一行
`- key=value｜...`，标量数组一行逗号分隔，空值写 `(none)`。日志环是固定上限：宿主事件最多
100 条/64 KiB，stderr 最多 200 行/32 KiB，单行 2 KiB，导出文件最多 256 KiB；超出丢最旧。
目录可用 `DSH_QUORFLOAT_LOG_DIR` 覆盖；在 `dev` 下随 `.dev-home/` 隔离。

设置页没有“附带剪贴板”行，也不会再有；该功能已取消。

### 8.9 底部三组

| 组 | 内容 | 规则 |
|---|---|---|
| 轮次 | `3 轮 9 步 · 229 tok/s` | 速度 = decodeTokens ÷ decodeMs，不含 TTFT 与工具时间；≥10 取整，<10 保留一位小数；算不出来不伪造 0 |
| 消耗 | `85.5K tok · 缓存命中 90%` | 累计 = 三个 prompt 桶 + output；`<1K` 原数、`K`/`M` 一位小数；缓存命中**绝不把部分命中四舍五入成 100%**，99.9 照实 |
| 上下文 | `◯ 1%` | `min(100, round(used/window×100))`；无上限时整组不画；环从十二点顺时针，0% 不画弧，≥100% 收口 |

三组都只接受当前会话的推送；换会话即清零；上游没报的组不画占位符。

### 8.10 主题、语言与无障碍

- 颜色、圆角、字号、间距全部来自 `tokens.css`；组件里不写死数字。
- 明暗两套令牌；`prefers-color-scheme` 跟随系统，宿主/面板偏好可覆盖。
- 开关、菜单、卡片有 `role`/`aria-checked` 等语义；可点元素用真实按钮，不用不可交互的 `<span>`。
- 用户可见文案全部来自字典；状态词在两个表面同口径。

### 8.11 实现边界

- 组件一个文件一个，纯逻辑放 `quorfloat/frontend/src/logic/`，副作用放 `quorfloat/frontend/src/hooks/`；
  组件不改快照。
- 线程窗口化是硬约束；`.q-thread-items`/`.q-thread-item` 是坐标空间，行高必须包含间距。
- 布局顺序：父组件的 `max-height` 是声明式的；子组件 layout effect 先于父组件，任何“父组件在
  effect 里设尺寸、子组件据此测量”的写法都会让子组件先看到错的盒子。
- 改了前端必须重建侧车：前端资源在编译期嵌入侧车二进制，只跑 `build:frontend` 不会更新二进制。
- UI 以实现与 `tokens.css` 为准；不要以早期设计稿作为规范或验收依据。

## 9. 构建、分发与版本

### 9.1 离线 bundle

`npm run build`（`scripts/release.mjs`）构建宿主、前端与 release 侧车，组装到
`bundles/dsh-quorfloat-<version>-<platform>/`，并自动生成同名 `.tgz`。macOS 默认用
`lipo` 合并 arm64 + x86_64 为 universal；Windows 必须在 Windows 上构建 x64。产物不签名、不公证。

bundle 内的 `cordis.patch.yml` 把 `quorfloatPath` 指向自带的 `./bin/dsh-quorfloat`；
宿主对相对路径按包根解析，因此 bundle 可以自包含。

### 9.2 npm 包

`npm run build:npm`（`scripts/release-npm.mjs`）生成主包与当前平台依赖：

- 主包：`dsh-quorfloat`，`cordis.patch.yml` 的 `quorfloatPath` 为空，宿主通过 Node 模块解析
  找到平台依赖的 `bin`；`optionalDependencies` 固定两个平台包同版本。
- 平台包：`@azetus/dsh-quorfloat-darwin`（os=darwin，cpu=arm64/x64，universal）与
  `@azetus/dsh-quorfloat-win32-x64`（os=win32，cpu=x64）。
- 先发布两个平台依赖，再发布主包；安装脚本不复制二进制，也不使用 postinstall。
- 源码 `package.json` 保持 `private`；生成的主包清单移除 `private`、开发脚本与 devDependencies。

### 9.3 二进制解析顺序

宿主按以下顺序找侧车，全部失败时诊断列出每个候选：

1. 配置 `quorfloatPath`（绝对路径或相对包根）；
2. `DSH_QUORFLOAT_PATH`；
3. 主 manifest 的 `quorfloat.platformPackages` 匹配的平台包（macOS 也接受
   `bin/<name>.app/Contents/MacOS/<name>`，裸二进制优先）；
4. 仓库内的开发构建位置。

### 9.4 版本

- 产品版本只有一个真值，声明处只有根 `package.json` 与 `quorfloat/Cargo.toml`；两者必须一致，
  `scripts/version.mjs` 在发布构建第一句校验，不一致拒绝构建。
- 侧车经 `env!("CARGO_PKG_VERSION")` 上报；`hello.quorfloatVersion` 即此值。
- `PROTOCOL_VERSION` 是接口契约号，与产品版本刻意独立，不随发版变动。
- 测试不得硬编码版本号，从真值来源推导。
- 插件在 `package.json` 通过 `engines.dsh: ">=0.2.0-rc.2"` 声明宿主范围；只写下限。DSH 0.2.1-alpha.3 之后该字段优先于 peerDependencies。
- 诊断导出的 `dshVersion` 读 `@deepseek-ai/dsh-app-boot/package.json` 的 version — 与 DSH 自己的
  `getDshRuntimeVersion()` 同源；`DSH_VERSION` 只作回退，仍不可用则 `unknown`。

### 9.5 CI 与发布

- `.github/workflows/publish-npm.yml` 支持手动运行与正式 Release 触发。
- 手动运行构建 macOS universal 与 Windows x64，执行测试并保存离线/平台/主包 tgz，不发布 npm。
- Release 触发时，两平台构建与测试都成功后，依次发布两个平台包与主包，使用 npm OIDC，无 `NPM_TOKEN`。
- 云端测试先运行 `npm run ci:install` 安装 `.tooling/ci` 的上游契约依赖，再运行
  `npm run ci:test`：Rust 测试、前端测试与 typecheck、JS 协议/上游/健壮性测试；
  **不启动真实桌面侧车**。桌面集成与 UI 实机验收在本地完成。

## 10. 平台支持与已知限制

### 10.1 平台矩阵

| 平台 | 架构 | 状态 |
|---|---|---|
| macOS | arm64 + x86_64（universal） | 支持，已实机验收 |
| Windows | x64 | 支持，已实机验收 |
| Linux | — | 不支持 |

Windows 构建需要 MSVC Build Tools + Windows SDK；macOS 需要 Xcode Command Line Tools 与两个
Rust target。普通安装用户不需要本地编译。

### 10.2 已知限制

- Windows 上状态文件的符号链接行为未专门验证；普通文件路径已跨平台覆盖。
- 悬浮窗是宿主本机窗口；通过远程 WebUI 访问时只能使用插件状态/设置页，不能显示或操作悬浮窗。
- 诊断日志写到 DSH 主机上的 `~/.dsh-quorfloat/logs/`；远程 WebUI 用户拿到的是远端路径，不是浏览器所在机器。
- 诊断日志只含宿主最近事件与侧车 stderr，不含 DSH 自身日志；marker 仅 dev/CI 或手动设置
  `DSH_QUORFLOAT_RUST_MARKER` 时存在，用户导出不读取 marker。
- 显示器热插拔与边缘拖动：macOS 与 Windows 均已实测通过。
  显示前守卫检查窗口顶部拖动条与显示器 **work area** 的可见面积，至少 **64×24 logical px** 才保持原位；
  不足或完全离屏时回退到主屏 `work_area` 的锚点。work area 已排除 macOS 菜单栏/程序坞和 Windows
  任务栏，不需要应用层计算这些区域。可见但拖动条仍可用的窗口保持原位；可见窗口拔屏后若系统不自动挪窗，
  下一次呼出/托盘切换会走同一守卫。
- 会话标题的实时更新依赖 `sessionProjections`；该服务缺席时标题最多滞后一个对账周期（30 秒）。
- 长会话只保留 4 MiB 转录窗口；更早内容通过分页加载，超出窗口的旧行会被回收并写 marker。
- 审批/追问一次只出现在一个界面；领取后不会同时出现在 Harness。
- 远程图片会触发第三方请求；这是用户已接受的取舍。
- 离线包不签名、不公证；安装与更新后必须重启 Harness 本体，只重启面板不够。
- macOS 呼出后切 Space，面板留在原 Space（普通 NSWindow 行为，已接受）。

## 11. 关键决定与已否决方向

本节只保留仍然约束实现、且未来可能被重新提出的决定。详细证据与已废止方向的历史记录属于本地
开发资料，不随仓库分发。

### 11.1 关键决定

| 决定 | 理由 / 影响 |
|---|---|
| 插件是 Harness 的附件，不是第二个桌面客户端；审批/追问领取排他 | 不复制会话存储、不另建 runtime；领取后请求不再出现在 Harness |
| 不读取剪贴板 | 任何设计都要用户二次确认，与用户自己 Ctrl+V 等价；设置页不再出现该行 |
| 一轮一层折叠；工具调用不做专属卡片；后台事件不进对话 | 过程全部收进 `已完成`；后台事件只计数、未识别 kind 写 marker |
| 图片自动显示，不点击确认 | 已接受第三方请求；远程 `lazy`/`async`/`no-referrer`，本地受限读取 |
| 会话 pin 唯一，工作区是它的投影；呼出默认新会话，首次提交才创建 | 不允许两个并列持久事实各说各话；避免留下空会话 |
| “未分组”只是浏览范围；会话归属按 `sessionIds` ∩ 可见列表 | 未分组不是工作区实体；不比较 cwd，避免会话移动后静默出错 |
| presence 判据是“可见且有焦点” | 跨平台唯一一致判据；5 秒心跳、30 秒过期，避免死页面锁住归属 |
| 追问答案的 `selected` 是选项 label，按选项展示顺序 | 一卡一次提交；`custom` 只在有非空自由文本时出现 |
| `session/prompt` 的幂等键每次尝试唯一且跨进程唯一 | Harness 用 `requestId` 比对历史；重复键会 accepted 而不受理 |
| 大负载按字节分片，分片与事件按序发送 | 1 MiB 帧上限；事件越过前面的分片会被判过期而不渲染 |
| 会话列表由 Harness 的 `api-session/*` 推送维护，慢对账兜底；removal 只重查不删行 | 官方浏览器客户端同样一次全量 + 事件；removal 是活注册表变化，不是持久列表删除 |
| 转录窗口按字节、按整轮回收 | 4 MiB 文本，单条 32 KiB；按条数会让构建输出与普通文本占用差三个数量级 |
| 版本只有一个真值；发布产物只由发布脚本产出 | 产品版本在 `package.json` + `Cargo.toml`；跨平台组装会伪造未构建的产物 |
| 测试只守协议/接口、上游绊线与少量不变量 | 产品决定会变；UI 外观交给实机验收，行为/契约仍可测 |
| UI 以实现与 `tokens.css` 为准 | 早期设计稿已经偏离实现，继续以旧稿为准会误导维护者 |

### 11.2 已否决方向

以下方向已经明确否决，不要重新提出，除非出现新的实测反例：

- egui/eframe 视图层 → 已由 Tauri + React 取代；
- 随包字体 → 系统字体栈；
- `egui_commonmark` / `pulldown-cmark` 渲染 → markdown-it + DOMPurify；
- 帧缓冲截图、图元 dump、动画抓帧工具 → 已退役；
- `tauri-nspanel` → 普通 NSWindow + Accessory 已满足；
- 无框架 vanilla TS 前端 → React；
- `protocol/schemas/` 代码生成管线 → 方法数少时是净负担。

## 12. 相关文档

| 文档 | 用途 |
|---|---|
| `AGENT.md` | 开发操作：命令、纪律、观测面、编码约定 |
| `CHANGELOG.md` / `CHANGELOG_EN.md` | 版本与工程根因（中/英） |
| `README.md` / `README_EN.md` | 安装、更新与基本使用（中/英） |
