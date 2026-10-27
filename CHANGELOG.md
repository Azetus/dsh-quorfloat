# CHANGELOG — 版本与工程记录

> 本文记录每个版本的用户可见变化、协议/工程根因。当前行为规范见 `ARCHITECTURE.md`；
> 排查“为什么现在是这样做”时阅读本文。英文版见 `CHANGELOG_EN.md`；中文为准，两份在同一提交内同步。
>
> 新条目加在最上面；每条尽量控制在 8 行以内。只记录改变行为、协议或发布的事；日常小修并入
> 相邻条目。测试数量只在能说明发布状态时保留，且写明“当时”；不记录验收结论。
> 三级标题按主题分组，正文直接说明改动；原因简短带过。

## 版本索引

| 版本 | 时间 | 主题 |
|---|---|---|
| 0.1.10 | <date> | 会话列表事件化、归档对齐与 30 秒对账 |
| 0.1.9 | 2026-10-22 | 诊断日志可读性、托盘重置窗口位置 |
| 0.1.8 | 2026-10-19 | 宿主版本范围、窗口拖动区守卫、开发脚本清理 |
| 0.1.7 | 2026-10-18 | 诊断日志导出、状态文件原子写入、Windows 构建/CI 修复 |
| 0.1.6 | 2026-10-14 | 图片自动显示、内置首页导航修正、README 版本占位符 |
| 0.1.5 | 2026-10-13 | Markdown 外部内容三阶段（链接、远程图片、本地图片） |
| 0.1.4 | 2026-10-13 | npm 平台包与 GitHub Actions、Windows 启动/心跳修复、测试分组 |
| 0.1.3 | 2026-10-12 | npm 发布包构建入口、npm 12 兼容、发布元数据 |
| 0.1.2 | 2026-10-12 | 未分组浏览、i18n、系统字体、标题投影、分页对齐 |
| 0.1.1 | 2026-10-12 | 长会话分块快照、加载更早、按钮高度、bundle 过滤 |
| 0.1.0 | 2026-10-12 | 版本一致守卫、工作区推送、bundle/tgz 自动打包 |
| pre-0.1.0 | 2026-10-07 – 10-11 | 宿主/侧车/协议、审批与追问、Tauri + React 迁移、托盘、pin |

> 0.1.3 之前没有发布到 npm，也没有 GitHub Release；这些版本只作为本地/离线构建记录。
> 0.1.10 尚未发布，日期以 `<date>` 占位，发布时替换为实际发布日期。

## 0.1.10（<date>）

### 会话列表事件化
- 会话列表改由宿主推送维护：订阅 Harness 的 `api-session/added|removed|status|activity` 与
  `sessionProjections` 的 `title`，作为 `sessions/changed` 发给侧车；选择器与相对时间不再等 3 秒轮询。
- 侧车把推送合并进同一份列表：名字只出现不消失、`cwd` 只补不换、`updatedAt` 只前进、`blank`
  只降不升；列表答复在途时的推送先应用再缓存，答复落地后重放。
- `removed` 只表示离开活注册表，不删行，而是立即请求权威列表；`sessions/list` 降为 30 秒对账，
  握手与面板呼出各刷新一次，答复超时按 3 秒重试。断线/重连仍由新进程首帧全量覆盖。
- 标题实时更新依赖 `sessionProjections`；该服务缺席时标题随对账刷新（最多滞后 30 秒）。

### 归档对齐
- 面板隐藏 Harness 已归档的会话：用 workspace 推送的完整归档集合过滤选择器、工作区菜单与“未分组”；
  取消归档后下一帧即恢复，不重拉列表。
- 当前会话被归档时离开并回到新会话态（通知宿主 `session/detach`、清转录与在途快照/历史页）；
  固定会话被归档时静默清除 pin（仅 marker，不产生 `pinNotice`）。
- 已归档会话的迟到快照被丢弃；面板不提供“显示已归档”，归档与恢复都在 Harness 本体完成。

### 注释与文档
- 修正 `follow.rs` 两处过期说明（“没有会话出现通知”“跟随最近会话”）。
- `ARCHITECTURE.md` 增补 `sessions/changed` 与列表刷新规则；`AGENT.md` 增加对应 marker 行。

## 0.1.9（2026-10-22）

### 诊断日志可读性
- 导出快照区由 JSON 片段改为扁平文本。
- 数据来源、脱敏规则、容量上限和落盘行为不变；不提供 JSON 格式导出。

### 托盘重置窗口位置
- 菜单栏/通知区新增“重置窗口位置”，按当前 `window.anchor` 和主显示器 work area 移动窗口。
- 隐藏状态下只移动，不显示也不抢焦点；无显示器或移动失败时写 marker，不改变可见状态。

## 0.1.8（2026-10-19）

### 宿主版本与 DSH 0.2.1-rc.2
- `package.json` 增加 `engines.dsh: ">=0.2.0-rc.2"`，只写下限；DSH 0.2.1-alpha.3 之后该范围优先于 peerDependencies。
- 诊断 `dshVersion` 改为读取运行中的 `@deepseek-ai/dsh-app-boot/package.json`，与 DSH 的 `getDshRuntimeVersion()` 同源；`DSH_VERSION` 仅作回退，仍不可用则为 `unknown`。

### 窗口拖动区守卫
- 旧规则只要有正面积交集就认为可见；窗口可被拖到只剩一条边，或被程序坞、菜单栏、任务栏挡住，
  顶部拖动条几乎不可点击。
- 启动、显示、热键/托盘前检查窗口顶部拖动条与显示器 **work area** 的交集，至少
  64×24 logical px 可见才保持原位；不足则回退到主屏锚点。可见但拖动条仍可用时保持原位。
- 热键/托盘在 visible 但拖动条不可用时执行救援而不隐藏；没有显示器轮询，也没有新增托盘
  重置项；可见窗口拔屏后若系统不自动挪窗，需要一次呼出触发。

### 开发脚本清理
- 删除失效的 `--perf` 与 `DSH_QUORFLOAT_PERF`；该变量来自 egui 时代，Tauri/React 侧车已无消费者。
- 测试中的产品版本与 DSH 版本占位改为中性值，避免随发版过期。

## 0.1.7（2026-10-18）

### 状态文件原子写入
- `preferences.json`、`pinned.json`、`window.json` 原先直接覆盖写入；中断、磁盘错误或
  Windows 共享冲突可能留下半截文件，让下一次启动丢掉设置、pin 或窗口位置。
- 保存改为同目录临时文件 + `sync_all` + rename；Unix 保留符号链接目标与权限，Windows 用
  `MoveFileExW` 替换式写入。失败保留旧文件，并写 `preferences save failed`、`pins save failed`、
  `window position save failed` marker。
- 测试只保留两条不变量——正常替换、强制 rename 失败不破坏旧文件。

### 诊断日志导出
- 设置页“导出诊断日志”把当前状态、最近宿主 supervisor 事件与侧车 stderr 写成 `.log`，
  写到 `~/.dsh-quorfloat/logs/` 并显示绝对路径；`DSH_QUORFLOAT_LOG_DIR` 可覆盖，dev 下隔离到
  `.dev-home/logs/`。
- 日志上限为宿主事件 100 条/64 KiB、stderr 200 行/32 KiB、单行 2 KiB、导出文件 256 KiB；超出丢最旧。
  快照 ID 截短、home 路径替换为 `~`，不含对话正文或凭证。
- 不读取 DSH 自身日志，也不读取仅 dev/CI 才有的 marker；不做剪贴板/浏览器下载；
  每次手动导出保留一个文件，不自动清理。

### Windows 构建与 CI 工具链
- 前端构建把 `TEMP` 指向工作区内，避免 dsh/esbuild 在 Windows 目录完整性约束下冲突。
- 新增 `npm run ci:install` 安装 `.tooling/ci` 的上游契约依赖；workflow 改为调用它，
  `ci:test` 缺少依赖时给出可行动错误。
- `contract:extract` 按路径段查找 asar 条目，修复 Windows 下反斜杠路径被当作一个文件名的问题。

## 0.1.6（2026-10-14）

### 图片自动显示
- 本地与远程 Markdown 图片不再需要“加载图片”确认；组件挂载即进入 loading，失败可重试。
- 远程图片使用 `lazy`/`async`/`no-referrer`；本地图片按会话 cwd 解析、绝对路径允许在 cwd 外；
  `file://`、UNC、`~` 拒绝；格式限 PNG/JPEG/WebP/GIF；上限 `min(Harness maxImageBytes, 8 MiB)`。
- 前端全局最多 2 个活跃读取；等待项取消后不再读取；离开组件释放 Blob；
  `session/image` 每次 256 KiB 原始字节，30 秒过期，侧车最多 4 个在途。

### 内置首页导航修正
- 宿主显示已启动，但面板不可见；marker 显示窗口/托盘创建后立即 `webview navigation blocked`。
- Tauri 把 `index.html` 简化为 `tauri://localhost`，path 是空串；白名单只允许 `/` 与
  `/index.html`，把启动页自己拦住了。
- 修复为在内置协议/主机限制下接受空 path；外部导航与协议限制不变。

### README 版本占位符
- 根 README 中英文的 tgz 文件名、npm 版本与构建目录示例改为 `<version>`，并说明替换为实际版本。

## 0.1.5（2026-10-13）

### Markdown 外部内容：链接
- 网页链接只接受完整 HTTP(S)，前端拦截点击/中键，Rust 通过 Tauri Opener helper 交给系统浏览器；
  页内锚点保留；其他导航与新窗口请求拒绝。
- 原始 HTML 继续按文本展示；DOMPurify 继续消毒 Markdown 输出。
- 生产与开发启用 CSP：脚本、字体、对象、框架、表单、base 受限；`img-src` 允许 self/http/https/blob；
  保留 Tauri IPC、布局内联样式与开发环境 Vite WebSocket。
- 新增 `tauri-plugin-opener`（Rust helper），不注册文件打开命令，不增加前端 npm 依赖。

### Markdown 外部内容：远程图片
- Markdown 只生成 `data-image-src`/`data-image-alt` 惰性占位；`DOMPurify` 仍禁止 `img`；
  `AnswerBody` 把受控 `MarkdownImage` 组件挂到占位上。
- 初次渲染不请求；点击后才创建 `img`，异步解码、不发送 Referer，失败可重试；图片宽度不超过回答区域，
  无放大浮层，点击图片或按钮不激活外层链接。
- 新文案全部走中英字典。

### Markdown 外部内容：本地图片
- 宿主 `src/harness/images.ts` 通过 Harness 的 `fs.resolve/readBytes` 读取；会话 cwd 来自
  `sessionController.list`；缺 cwd 只阻止相对路径，不阻止绝对路径。
- `session/image` 首次读生成随机 token，后续按 offset 取块；完成/取消/失败主动释放；
  Rust 独立在途表，12 秒超时，不占 follow/prompt 槽。
- 前端校验分块身份/偏移/大小/格式后组装 Blob URL；离开组件取消并释放；图片不进入会话快照。

### 导航白名单修正
- 与 0.1.6 条目同一根因；0.1.6 包含该修复。

## 0.1.4（2026-10-13）

### npm 包与发布流程
- 主包为未 scoped 的 `dsh-quorfloat`（名称在 0.1.3 已恢复）；平台包为
  `@azetus/dsh-quorfloat-darwin` 与 `@azetus/dsh-quorfloat-win32-x64`。
- 主包 `cordis.patch.yml` 的 `quorfloatPath` 为空，宿主通过 Node 模块解析平台包 `bin`；
  `optionalDependencies` 固定两平台同版本；不复制二进制、不使用 postinstall。
- 新增 GitHub Actions `publish-npm.yml`：手动运行构建并保存产物，不发布；正式 Release 触发 npm 发布。
  使用 npm OIDC；两平台构建与测试都成功后，先平台包、后主包。
- 发布脚本按名称/版本已存在就跳过；已有版本保留原内容，不替换。版本查询 404 时读 dist-tags，
  避免扫描期间重复上传。

### Windows 启动与监督修复
- 侧车 Reader 在 WebView 初始化前启动；初始化阶段的 shutdown/EOF 先冲刷应答再以 0 退出。
- Tauri 的 setup 在 `app.run` 内执行，`shell_ready` 必须在 `RunEvent::Ready` 才置位；
  过早置位会让“冷启动直接退出”分支覆盖真实 setup。
- 宿主心跳改为每 channel 单次在途；忽略停止/旧代结果；重复 stop 合并；手动 stop 使等待中的自动重启失效。
- `transport closed` 不再谎报进程已退出；仍存活的 child 会被实际清理。

### 测试与历史
- 8 项真实侧车测试移入 `tests/desktop/rust-peer.test.mjs`；云端 `ci:test` 只跑协议/上游/健壮性测试，
  不启动桌面侧车。`test:unit` / `test:desktop` / `test` 由明确文件路径选择，空套件报错。
- 本地对 git 历史做过压缩（130 → 71 提交），保留版本标签与最终 tree；未 push。旧历史的细节不再可查，
  因此本文只保留能解释当前行为的根因。

## 0.1.3（2026-10-12）

- 新增 `npm run build:npm`：生成主包与当前平台依赖目录/tgz，共用 release 的编译、lipo、哈希与 npm pack 逻辑。
- 主包保留 Harness 元数据、README/LICENSE；平台包只带二进制、元数据、说明与许可证，无 `dsh.bundle/client`。
- npm 12 的 `npm pack --json` 从数组改为包名映射对象；脚本兼容两种形状，继续要求唯一文件名与合法 basename。
- 补齐 repository/homepage/bugs 元数据；README 资源随离线包与 npm 包分发，npm 主包中的链接改为 GitHub 绝对地址。
- 版本 0.1.3 作为过渡版本；主包名在此期间恢复为未 scoped 的 `dsh-quorfloat`，平台包保持
  `@azetus/...` 前缀。

## 0.1.2（2026-10-12）

### 未分组浏览
- 工作区选择器在真实工作区后追加“未分组”；无 pin 按钮，不能作为新建目标；该范围隐藏“新会话”。
- 归属只按工作区注册表 `sessionIds` ∩ 可见会话列表；不比较 cwd。
- 选择真实工作区、点击具体会话、跟随会话变化或再次呼出时退出显式浏览。
- 这是对 Harness 选择器的刻意扩展；Harness 的未分组树虽显示新建图标，但其 `onCreate` 在没有
  `workspaceId` 时不会执行。

### 标题与列表
- 会话标题改读 `sessions/list` 的 `projections.values.title`；行本身没有顶层 `title`。
- 只有投影缺失/null 且 `updatedAt` 移动时才用 `inspect` 折叠 `session/title`，每次列表最多读 1 条。

### 分页与字体
- 初始快照与后续页统一按整轮对齐：`maxMessages:200`、`turnWindow:{minMessages:50,minTurns:2}`；
  大页改用 `session/history/begin|part|end` 分片，收齐整页后前插。
- 折叠键从“问题 + 位置”改为“问题 + 从最新端数的序号”，旧页前插不再让全线程折叠记忆失效。
- 移除随包字体，改用系统字体栈；WebView 负责字符回退，不再下载/分发字体资产。
- bundle 过滤 `.DS_Store`、`._*`、`__MACOSX`；manifest `files` 改为精确文件清单；构建后自动生成 tgz。

## 0.1.1（2026-10-12）

### 长会话空白
- 整个会话只走一帧 `session/snapshot`；两侧帧上限 1 MiB，超限帧被静默丢弃，快照不执行，
  之后每个 `session/event` 都被判 stale，面板永久空白且不重试。
- 超限帧带字节数并计数；新增 `session/snapshot/begin|part|end`（256 KiB 预算）与
  `session/snapshot/request`；侧车按 `sessionId+generation` 装配，收齐才折叠；缺片有界重发；
  旧的单帧 `session/snapshot` 仍作为“一片”接受。
- 分片与事件共用通道时必须逐帧串行发送，否则事件会越过分片并被判过期。

### 加载更早
- 初始快照此前固定约 50 条；新增 `session/history {sessionId,beforeSeq}` → `{records,hasMore,cursor}`。
- 页预算 512 KiB，单条记录硬夹取 32 KiB；超过 512 KiB 拆成约 256 KiB 分片。
- 滚动补偿取**行锚点**的真实起点差，不用总高度差；用户主动滚动或点“回到底部”时释放锚点。
- “加载更早”只在 `hasMore` 且滚动到顶（`scrollTop ≤ 1px`）时显示。

### 其他
- “加载更早”药丸在零高度 flex 槽里被 `align-items: stretch` 压成 10px 边框盒；改为
  `align-items:flex-start` + 固定按钮盒内布局。
- 版本从临时 0.1.2/0.1.3 收回 0.1.1；旧同名 tgz 早于修复，不能复用。

## 0.1.0（2026-10-12）

- 版本真值收敛到根 `package.json` 与 `quorfloat/Cargo.toml`；`scripts/version.mjs` 在发布构建第一句校验，
  不一致直接拒绝构建。
- `quorfloat/tauri.conf.json` 不写 `version`，Tauri 回退到 crate 版本；若将来改用 `tauri build` 打包，
  需补 `bundle.macOS.bundleVersion` 或版本字段。
- `PROTOCOL_VERSION` 保持 `quorfloat/1`，与产品版本独立。
- 工作区推送 `workspaces/changed`：baseline/upsert/remove/order；`archived` 与 `pinned` 转发但不应用。
- bundle 组装与 tgz 打包自动化；排除 macOS 元数据，失败清理临时目录，旧包不得冒充新产物。

## 预发布阶段（2026-10-07 – 2026-10-11）

### 宿主、侧车与协议
- 建立 TypeScript 宿主插件 + Rust 侧车的 stdio NDJSON JSON-RPC 2.0；握手、心跳、优雅退出、
  重启预算、stdout 纯净性、1 MiB 帧上限与 marker 观测面。
- 宿主通过 Cordis `inject` 等待 `sessionController`；直接探测会误报服务缺失。
- `user/message` 的 content 不套 `message`，其他记录类型套；`tool-call.arguments` 是 JSON 字符串；
  用户消息按 `source.kind` 判别。这些形状来自真实抓帧，不靠猜。
- 幂等键问题：`session/prompt` 的 `requestId` 被 Harness 拿去历史里比对 `source.rpcId`，命中就
  accepted 而不受理；按会话内计数会在重启后复用旧键，造成“输入框清空但消息没发出去”。改为
  `{sessionId}:prompt-{pid}-{毫秒}-{全局序号}`，宿主再拒绝本会话已花掉的键。

### 会话、审批与追问
- 会话策略从“跟随最近更新”改为“唯一 pin 或新会话态”；pin 是下次呼出打开哪个，工作区是它的投影。
- 新会话工作区按 P0–P4 阶梯；首次提交才创建。
- 审批归属按 `visible && hasFocus` 判定；面板只认领当前 attach 的会话；领取用 `prepend` 排他。
- 追问补上单选/多选/自由文本控件，`CAPABILITIES` 加入 `question`；答案形状与边界见根目录 `ARCHITECTURE.md`。
- `interaction/hint` 只表示“没有交给面板”；`panel/lifecycle` 让托盘能重启/停止自己。

### Tauri 与 React 迁移
- 视图层从 egui 迁到 Tauri 2.12 + WKWebView；宿主侧 spawn/env/协议不变。
- 迁移后先落地无框架 TS 前端，随后改为 React + 纯逻辑 + hooks；线程改为窗口化。
- 迁移中确认：握手必须先于窗口创建；Tauri 的 `setup` 在 `app.run` 内执行；`.app` 形态不是必须，
  裸可执行文件 + 平台包即可。
- 托盘、窗口拖动与位置记忆、`reduceMotion`、焦点归还、全屏 Space 主路径、多显示器位置记忆
  在 macOS 上落地。
- 旧抓图/帧缓冲工具在迁移后失效，已全部退役；不要按旧方法重建。

### 更早的界面与交付
- 设置页替换对话；主题/失焦/快捷键/语言；快捷键点框即录，裸按键拒绝，Esc/Enter 在录制期间让位。
- 底部三组：权限、模型与档位、统计；统计规则与缓存命中率“不四舍五入成 100%”的规则在此期间定稿。
- 折叠规则从“对齐上游 turn-process”改为“一轮一层折叠，其余 LLM 产出全部进折叠”；后台事件不画行。
- Markdown 表格限宽、长 ASCII 串软换行、滚动条沟槽、回到底部按钮、呼出动画与高度过渡在此期间完成。
- 发布从“手写 bundle”演进到 `release.mjs` + `release-npm.mjs` + CI；平台包与版本一致守卫逐步补齐。
