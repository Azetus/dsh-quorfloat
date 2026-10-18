# AGENT.md — 开发守则与操作手册

> 本文只包含每次都要遵守的规则，包括工作纪律、怎么验证、怎么观测、代码守则。
> 产品与架构见 `ARCHITECTURE.md`；版本与工程记录见 `CHANGELOG.md`；安装与使用见 `README.md`。
> 改协议/会话/审批/UI 前，先读 `ARCHITECTURE.md` 的 §4、§5、§6、§8 与 §11。
>
> 本文只写**现在仍然成立**的操作规则，不写开发历程、不写版本故事、不引用未追踪的本地笔记。
> 需要知道某个行为为什么变成现在这样，读 `CHANGELOG.md`；不要从发布记录反推规范。
> 本地未追踪笔记（如 `TODO.md`）不随仓库分发，也不作为规范；新代码与注释不要依赖它。

## 1. 项目定位

`dsh-quorfloat` 是 DeepSeek Harness 的系统级悬浮会话窗口：TypeScript 宿主插件（`src/`，编译到
`lib/`）＋ Tauri 侧车（`quorfloat/`，Rust）＋ React 面板（`quorfloat/frontend/`），宿主与侧车用
stdio 上的 NDJSON JSON-RPC 通信。

## 2. 项目边界

- **不修改 Harness 本体**。`src/harness/` 是唯一允许接触 Harness 服务细节的模块。
- 侧车不持有 API Key，不执行工具，不直接读写 Harness 会话存储。
- 前端不改快照；所有写入经 `quorfloat/frontend/src/api.ts`。
- 用户真实状态（`~/.dsh-quorfloat/`、`~/.dsh/profiles/`）只读；测试必须重定向到临时目录。

## 3. 协作与工作纪律

### 3.1 先讨论，再计划，后执行

- 写代码前先讨论方案；只有用户明确表示开始执行后才修改代码。
- 已获授权的计划可以连续完成，不需要每个文件重复确认；方案变化要重新说明。
- 禁止 `git commit`、`git push`、rebase、reset、切分支、创建/删除标签、fetch 等修改 Git 状态或引用的
  操作，除非用户明确授权。只读 Git（`status`、`log`、`diff`、`show`、`branch -l`、`tag -l`、
  `ls-remote`）允许。
- “讨论/确认方案”不等于授权执行历史改写。

### 3.2 先量，再改

- 怀疑某处之前，先加一行可观测记录或写一个最小探针；不要凭感觉改。
- 能断言的规则就断言；不能断言的（平台行为、视觉）交给真实运行 + 用户验收。
- 探针用完必须删除并说明；不要把临时脚本留在仓库里。

### 3.3 改动必须有测试，且先证明测试能失败

- 新增/修改行为时，先让测试在旧实现上变红，再让它变绿；不能变红的测试没有在守任何东西。
- 测试只守协议/接口、上游绊线、少量不变量；产品决定与 UI 外观不写常驻测试。
- 测试不得依赖开发者真实状态、真实 home 路径或本机特定文件；路径必须重定向到临时目录，并在结束后还原。
- `#[test]` 丢失不会报错，只会静默不跑；测试数量变化时必须查为什么。平台分支不许静默 `skip`。

### 3.4 写完立刻验证

- 写完文件后断言内容确实变了（尤其是多锚点补丁）；锚点写错可能整段静默不写入。
- 每改一处就编译一次；不要带着坏树继续改。
- 报告“修好了”之前，跑完 §4 的相关命令；说“编译通过”与“用户已实机确认”必须分开。
- 视觉改动要看真实运行的面板；不要只看代码自证，也不要用“测试全绿”代替 UI 验收。

### 3.5 不留死代码，不重复事实

- 删除方法时连同它的测试、导出、字典项、文档引用一起删；未使用的再导出会触发告警，告警即失败。
- 同一张表、同一组常量、同一个事实只写一处。协议方法表只在 `ARCHITECTURE.md` §4 与代码常量中维护；
  不要在 router 的 `switch` 和另一个列表里各写一遍。
- fixture 与解析器不能出自同一个假设；优先把真实载荷粘进测试，并写明来源。
- 注释只写功能与实现，不写日期、“用户报告/要求/决定”、“第一轮/曾回退”、“实测/已验证”这类叙述；
  保留现在时与真实的“为什么”（平台怪癖、协议规则、顺序要求、上限）。
- 注释里不得引用未追踪的本地笔记；需要的事实内联一句，或改指向 tracked 文档。

## 4. 验证命令

在仓库根目录执行。测试数量不写死在这里：以命令输出为准。

```sh
# Rust：单测、构建、文档
cd quorfloat && cargo test
cargo build && cargo doc --no-deps      # 两者都必须 0 告警
cd ..

# JS：完整套件（协议/上游/健壮性 + 桌面集成）
npm run test:only                       # 使用已编译产物；不重新构建
npm run test:unit                       # 先编译宿主，跑协议/上游/健壮性，不启动真实侧车
npm run test:desktop                    # 完整 debug 构建后，跑真实侧车桌面集成
npm run test                            # 完整 debug 构建后跑以上两组

# 云端门槛（不启动真实桌面侧车）
npm run ci:test

# 前端：单测与类型检查
cd quorfloat/frontend && npm test
npx tsc --noEmit
cd ..

# 本地开发（隔离环境，状态写入 .dev-home/）
npm run dev
npm run dev -- --release                # 用 release 侧车做体感验证
npm run dev -- --empty-workspaces       # 零工作区首启；默认工作区建到 .dev-home/
```

构建入口：

| 命令 | 产物 |
|---|---|
| `npm run build` | 可发布的离线 bundle：release 编译 + macOS universal `lipo` + 组装到 `bundles/` + 自动 `npm pack` |
| `npm run build:npm` | npm 主包 + 当前平台依赖包及各自 tgz，输出到 `bundles/npm/<版本>/` |
| `npm run build:local` | 本地 debug 全量：`build:plugin` + `build:sidecar` |
| `npm run build:plugin` | 宿主 TS → `lib/` |
| `npm run build:frontend` | Vite 前端 → `quorfloat/frontend/dist/` |
| `npm run build:sidecar` | 先构建前端，再编译侧车 |

`scripts/cargo.mjs` 是本地与发布共用的 cargo 构建层；新脚本必须复用它，不要各写一遍。
**改了前端就必须重建侧车**：前端资源在编译期嵌入侧车二进制，只跑 `build:frontend` 不会更新二进制。

`npm run dev` 只在启动时把侧车复制到 `.dev-home/sidecar/`；改了 Rust 后必须 `cargo build` 并**重启 dev**，
否则面板仍跑旧副本。判断副本是否更新，可以直接检查二进制里是否还有源码中已删除的字符串。

## 5. 观测面

### 5.1 marker 是唯一可靠的运行时记录

宿主会吞掉侧车的 stderr，所以事后可查的是 marker。marker 默认不写；在 DSH 的运行环境里设置一次：

```sh
DSH_QUORFLOAT_RUST_MARKER=<path>
```

侧车随后每次启动都往该文件追加。正常安装的用户不产生 marker 文件、零追加写入。

常用行：

```text
workspaces N                         工作区列表答复
sessions N named M                   会话列表答复 + 其中带标题的条数
follow <id> generation=N             附着成功
detached <id>                        离开会话（已通知宿主）
interaction <id> approval arrived    审批到达面板（追问同理：question arrived）
create-then-prompt N bytes           惰性新建会话后补发首条消息
unpin <id> (no longer listed)        固定项失效被清除
layout panel=… window=WxH          高度核算；面板被裁切时看这行
height capped at N: asked M, …       平台始终没给够高度：改用窗口现有高度并停止重绘
transcript skipped: N dropped        转录窗口按字节回收时丢弃的条数
snapshot parts: N (M bytes max)      分块快照装配完成
snapshot part missing; requesting resend  缺片，已请求重发
snapshot dropped: <reason>           分块快照被丢弃（超时/被新 begin 取代/打开过多）
frames decoded=D garbage=G oversized=X  退出时统计：解码/垃圾/超限帧
settings initial / preferences set / window config applied  设置与实际窗口配置的真值面包屑
preferences save failed: <path>: <error>  偏好写入失败（内存值仍生效）
pins save failed: <path>: <error>         固定项写入失败（内存值仍生效）
window position save failed: <path>: <error>  窗口位置写入失败（窗口仍可用）
workspaces changed: baseline <n>     工作区推送流基线
workspaces changed: upsert <id>      工作区新增/改名
workspaces changed: removed <id>     工作区删除
workspaces changed: order <n>        工作区顺序变化
panel languages reported: tag, tag   面板 WebView 原样上报的 navigator.languages
webview navigation blocked           主 WebView 拒绝离开允许的面板文档
external URL could not be opened     已校验的网页链接未能交给默认浏览器
language seeded: zh|en               首次启动采用种子语言
language changed: zh|en              运行时语言变更
window position remembered at X,Y    启动时读到的记忆位置
window restored at X,Y               记忆位置有效
window anchored at X,Y               没有记忆位置，按锚点摆放
window position rejected: …          记忆位置不在任何屏幕内 → 回退锚点
tray created                         菜单栏/通知区图标建好
tray menu: show|hide|restart|exit    用户从托盘菜单选了什么
window hide: yielding activation to the previous app  收起时把激活交还上一个应用
transcript skipped: … internal / N unrecognised (kind, …)  被藏起来的条数与未识别 kind
```

前端自己的 marker 走 `api.log`，都带 `frontend` 前缀：`frontend show`、`frontend blur: …`、
`frontend hide requested (generation N)`、`frontend hide finished`、
`frontend hide ignored: hiding=… visible=…`、`frontend hide ack timeout: resetting the hide state`、
`frontend hotkey: …`、`frontend switch aria-checked=… hideOnBlur=…`。加/改显隐路径时顺手补一行。

### 5.2 诊断入口

- 安装态：DSH 的插件日志（宿主半边）、supervisor 捕获并限流转发的侧车 stderr、上面的 marker。
- 宿主 `diag/snapshot`：配置、握手与监督状态；宿主侧协议入口，正常 UI 不调用。
- 用户导出：设置页“导出诊断”调用 `quorfloat/diagnostics`，宿主把当前状态快照写到
  `~/.dsh-quorfloat/logs/` 并返回绝对路径。
- 不要依赖旧的帧缓冲截图、图元 dump 或动画抓帧工具；这些方法已退役，当前栈不再产出那些开关。

常用环境变量：

| 变量 | 用途 |
|---|---|
| `DSH_QUORFLOAT_PATH` | 覆盖侧车可执行文件路径；开发与 CI 用 |
| `DSH_QUORFLOAT_PREFERENCES` / `_PINNED` / `_WINDOW_STATE` | 覆盖三个状态文件路径；测试必须指向临时目录 |
| `DSH_QUORFLOAT_LOG_DIR` | 覆盖诊断日志目录；默认随 preferences 的父目录，dev 下落在 `.dev-home/logs/` |
| `DSH_QUORFLOAT_RUST_MARKER` | 打开侧车 marker；唯一可靠的运行时记录 |
| `DSH_QUORFLOAT_DEV_URL` | 指向 Vite dev server；WebView 只允许同一文档 |
| `DSH_QUORFLOAT_WINDOW_*` / `_THEME` / `_HOTKEY` | 宿主 spawn 时下发的启动基线；`ready` 会再下发一次有效配置 |
| `DSH_QUORFLOAT_WINDOW_START_VISIBLE` | 启动即显示，供无人值守的调试运行 |
| `DSH_QUORFLOAT_PARENT_PID` | 宿主进程身份，供侧车做父进程存活检测 |
| `DSH_QUORFLOAT_CARGO_LOCKED` | CI 中强制 `cargo --locked` |
| `DSH_QUORFLOAT_PEER_*` | 测试/脚本化 peer 的行为开关，不用于正常安装 |

`DSH_QUORFLOAT_PERF` 仍由 `scripts/dev.mjs` 设置，但侧车已无人读取；清理前不要把它当作性能开关。

## 6. 代码约定

### 6.1 通用

- 公开项写 `///`，含 `@param` / `@returns`；解释为什么，而不是复述代码。
- 能收紧的可见性就收紧（`pub(crate)`、`#[must_use]`）；测试要用的东西用 `#[cfg(test)]` 门控。
- 可测的纯函数优先：把“决定”（取值、拆分、折叠、优先级）做成纯函数，绘制/传输只做转交。
- 协议形状以实测为准；认不出的形状宁可什么都不做，也不要猜一个错误的结果。
- 不写死 UI 数字：字号/间距/圆角/颜色一律来自 `quorfloat/frontend/src/styles/tokens.css`。
- 用户可见文案一律来自 `quorfloat/frontend/src/logic/i18n/` 的中英字典；字典之外不得出现 CJK 代码字面量；
  key 类型由字典推导，漏 key 是编译错误。
- 面向用户的文案一律说“悬浮窗”；`侧车`/sidecar 只留在代码、标识符、注释、locale 命名空间与日志里。
  面板与 Harness 客户端半是两套字典，但状态词必须同口径。

### 6.2 前端

- 一个文件一个组件；跨组件复用的小块也各自成文件，不合并成 `parts.tsx`。
- 纯逻辑放 `quorfloat/frontend/src/logic/`（无 DOM）；副作用放 `quorfloat/frontend/src/hooks/`；
  组件只做绘制与事件转发。
- 快照是权威，组件不改快照；所有改动经 `api.ts`。
- 线程只挂载视口内的行（`@tanstack/react-virtual`），行高实测；动 `#q-thread` 结构前先读 `Thread.tsx`
  的坐标空间注释。
- 呼出即复位：页面、打开的菜单、录制状态、本地错误消息在 `App.onPanelShown` 清零；线程阅读位置与
  折叠状态属于对话，不在复位范围。
- 父组件的 `max-height` 是声明式的；子组件 layout effect 先于父组件，测量要么恢复原状，要么别动它。
- 窗口化里“记住的高度”必须能过期：每帧重读已挂载行的高度；未挂载行变矮不会触发 ResizeObserver。
- 读者在看哪儿是一条承诺：输入框长高、正文变化时同步兑现跟随底部或保留滚动偏移。
- 图标直接放在设计放置它的容器里；多包一层 `<span>` 会让针对子元素的选择器失效。
- 设置页的行、开关、芯片复用同一套令牌与交互词汇；不要再造第二套按钮或颜色。

### 6.3 Rust / 侧车

- `quorfloat/src/app/` 是状态与逻辑；`ipc/` 只做协议；`runtime/` 只做热键、托盘、marker。
- 侧车不产出面向用户的句子；结构化原因交给前端字典渲染。
- 高度协调遵守“申请 → 等待有期限 → 兑现或放弃并写 marker”的状态机；不要逐帧 resize。
- 主 WebView 只允许内置页面及其页内锚点；开发 URL 只允许同一文档；其他导航与新窗口拒绝。
- CSP 只允许必要的脚本、样式、字体、图片与 IPC；不要为了省事放开 `script-src` 或 `object-src`。

### 6.4 协议与传输

- 协议表、限额与形状以 `ARCHITECTURE.md` §4 为准；改协议时两边镜像一起改，并更新方法表测试。
- 大负载一律分片，并按**字节**度量：历史页 512 KiB、分片约 256 KiB、单条记录 32 KiB；不要用字符数估算。
- 分片与事件共用通道时发送端逐帧串行化，否则事件会越过分片被判过期。
- 缺片有界重发；装配超时、被取代、打开过多都要写 marker，不能静默空白。
- 幂等键必须每次尝试唯一且跨进程唯一；宿主拒绝已花掉的键，侧车把原因显示给用户。
- 超时只说明“没等到答复”，不等于远端没执行；不可逆操作要按此设计。

### 6.5 平台与构建

- 从 `URL`/`import.meta.url` 造文件系统路径一律 `fileURLToPath`，禁止取 `.pathname`。
- npm 脚本不得出现前导 `./`、`.bin/` shim 或依赖 shell 解析的写法；工具用 `node <真实入口>`。
- `chmod`/`X_OK` 在 Windows 上不可靠，只能是非致命检查，不得成为构建门禁。
- Windows 构建期需要 `icons/icon.ico` 与 MSVC Build Tools + Windows SDK；新增平台先查清构建期资源。
- 源码目录不得与 `.gitignore` 的任意条目重名；新增目录后 `git check-ignore -v <path>` 确认没被吞掉。
- 忽略规则归属它管辖的那一层：根 `.gitignore` 只管根一级，子项目产物写在子项目自己的 `.gitignore`。
- 发布产物只能由 `scripts/release.mjs` / `scripts/release-npm.mjs` 产出；Windows 包在 Windows 上构建，
  macOS 默认 universal；不签名、不公证，由发布者在真机安装验证。
- 同名产物是陷阱：打包前升版本或先删旧档；不要复用同名旧 tgz 或旧 debug 二进制。

### 6.6 版本与文档

- 产品版本只在根 `package.json` 与 `quorfloat/Cargo.toml` 声明，两处必须一致；
  `scripts/version.mjs` 在发布构建第一句校验。测试从真值来源推导，不硬编码版本号。
- `PROTOCOL_VERSION` 是接口契约号，与产品版本独立，不随发版变动。
- 随代码提交的文档：`AGENT.md`、`ARCHITECTURE.md`、`CHANGELOG.md`、`CHANGELOG_EN.md`、
  `README.md`、`README_EN.md`。本地未追踪笔记（如 `TODO.md`）不作为规范；代码注释可以引用 tracked
  文档的稳定章节，不得引用本地笔记。

文档更新分工：

| 变化 | 更新 |
|---|---|
| 行为、协议、策略、UI 契约、分发 | `ARCHITECTURE.md` |
| 命令、纪律、观测面、编码约定 | 本文 |
| 发布、根因、工程结论 | `CHANGELOG.md` / `CHANGELOG_EN.md` |
| 安装、更新、基本使用 | `README.md` / `README_EN.md` |
| 本地待办、临时笔记 | 本地未追踪文件（如 `TODO.md`），不作为规范 |

## 7. 测试与验收策略

### 7.1 只留三类测试

1. **跨进程协议 / 接口**：帧/NDJSON、握手与能力表、方法表与 `switch` 的一致性、浏览器远程端点的
   名字/参数/信封/坏输入。
2. **上游绊线**：真实组装、Harness 契约抽取/conformance、patch 整体替换 config 的机制。红了意味着
   “上游变了，我们要适配”。
3. **不变量（尽量少）**：协议级健壮性（畸形帧、未知方法、超限输入、永不答复的 peer，都不得抛错或
   污染状态），以及极少量数据完整性不变量（状态文件原子替换失败不得破坏旧文件）。

每个存活的测试文件在文件头一行写明它属于哪一类。每个里程碑重新分诊：如果某个产品决定变了，
这条测试要不要改？要改的就退役。

### 7.2 执行环境分组

- `tests/*.test.mjs`：云端可执行的协议/接口、上游绊线与健壮性测试；允许 Node 测试 peer。
- `tests/desktop/rust-peer.test.mjs`：启动真实 Rust/Tauri 侧车的桌面集成，需要本地窗口服务与 WebView。
- `scripts/test.mjs unit|desktop|all` 用明确文件路径选择套件，不依赖 shell glob；空套件报错。
- `ci:test` 只跑 Rust/前端既有单测、前端 typecheck 与 JS 协议/上游/健壮性测试，不执行桌面集成。
  这是明确缩小云端范围，不把桌面测试计成通过或静默 skip；发布前的桌面集成与 UI 实机验收在本地完成。

### 7.3 UI 验收

- 外观、布局、观感以真实运行的面板与用户实机验收为准；不写 UI 外观单测。
- UI 的行为与契约（写入路径、错误路径、焦点/关闭、顺序规则）仍应测试；只要一个合理的实现错误能把它
  弄红，就值得留。
- 不用“测试全绿”代替“功能可用”；也不用“画出来了”代替“点得开/看得见”。

## 8. 文档维护

- **一事一处**：同一事实只在 `AGENT.md`、`ARCHITECTURE.md`、`CHANGELOG.md`、`CHANGELOG_EN.md`、
  `README.md` 或 `README_EN.md` 中写一次，其他文档引用它，不复制；本地未追踪笔记不是规范。
- **规范不写历史**：`AGENT.md` 与 `ARCHITECTURE.md` 使用现在时；版本与根因写
  `CHANGELOG.md` / `CHANGELOG_EN.md`。CHANGELOG 保持当前“版本 → 主题”的结构，不引入
  Added/Fixed/Changed 分类。
- **双语同步**：`CHANGELOG.md` 中文为准，`CHANGELOG_EN.md` 在同一次提交内更新，版本号、日期与段落
  顺序一一对应。
- **不引用本地笔记**：tracked 文档不引用未追踪的本地笔记，也不引用已退役的文件名；需要的事实直接
  内联，或指向 `ARCHITECTURE.md` 的对应章节。
- **数值不复制**：版本、测试数、界面令牌、依赖版本以代码/配置/命令输出为准；文档只写规则与指针。
- **新增事实先判断归属**：行为/协议写 `ARCHITECTURE.md`，操作/纪律写本文，安装使用写
  `README.md` / `README_EN.md`，发布/根因写 `CHANGELOG.md` / `CHANGELOG_EN.md`。
