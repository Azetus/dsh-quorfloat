# 构建与 npm 发布

工作流文件为 `workflows/publish-npm.yml`，显示名称为 **Build and publish npm packages**。

## 手动构建与验收

1. 将源码、workflow、`quorfloat/Cargo.lock`、前端 lockfile、README 与 assets 提交到默认分支。
2. 在 GitHub 仓库 Actions 中选择工作流，点击 Run workflow 并选择源码分支。
3. macOS 与 Windows 分别生成离线 tgz、平台 npm tgz；主包只保留 macOS 任务生成的一份。
4. 从运行页面下载 `offline-darwin-<版本>` 和 `offline-win32-<版本>` 附件，解开外层 ZIP 后安装其中的 tgz。
5. 安装后重启 Harness，由使用者实机验收。

手动运行只构建、运行已有单元测试并保存附件，不发布 npm。附件保留 14 天。
构建成功后，即使已有测试失败也保存附件供排查；工作流仍保持失败状态，正式发布任务不会执行。
已有窗口与快捷键测试需要 runner 的窗口服务器；环境导致的失败需依据日志处理，不自动忽略这些测试。
CI 的上游契约来自提供 `codec.create()` 的 `@deepseek-ai/dsh-api-session-controller@0.2.1-alpha.1`，升级支持的 Harness 版本时同步更新。
测试依赖仅安装 Cordis、YAML 解析器和生成的协议契约；使用 `--legacy-peer-deps` 避免自动安装契约包声明的整套 Harness 插件，不加载这些插件的运行时代码。

## 配置 npm 授权

准备首次正式发布时，在三个包的 Settings → Trusted Publisher 中选择 GitHub Actions：

- 用户或组织：`Azetus`
- 仓库：`dsh-quorfloat`
- Workflow filename：`publish-npm.yml`（只填文件名）
- Environment name：留空
- Allowed actions：允许直接 `npm publish`

三个包为 `dsh-quorfloat`、`@azetus/dsh-quorfloat-darwin`、`@azetus/dsh-quorfloat-win32-x64`，分别设置相同信任关系。
无需配置 NPM_TOKEN Secret。新建信任配置需在两天内完成首次成功发布，否则需要重新创建。

## 正式发布

1. 同步根 package.json 和 quorfloat/Cargo.toml 的版本，并通过构建更新 Cargo.lock；先完成云端构建与实机验收。
2. 在 GitHub 创建指向验收源码提交的标签 `v<版本>`，发布对应的正式 Release。
3. 工作流校验标签和源码版本，重新构建并运行已有测试，全部成功后发布两个平台包，再发布主包。
4. 确认三个 npm 包的新版本，并通过 npm 安装主包进行实机验收。

初版支持稳定的 `x.y.z` 版本；草稿不触发发布，预发布 Release 不执行 npm 发布。
普通代码提交不触发此工作流。工作流不会提交代码、修改版本或执行 npm 登录。
部分发布失败时使用 Re-run failed jobs，复用本次构建附件；已存在且完整性相同的包会跳过，内容不一致则停止。
过期附件或重新构建产生不同内容时不能强行覆盖 npm 已发布版本，需要按日志处理。
