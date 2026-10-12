# quorfloat

简体中文 | [English](README_EN.md)

dsh-quorfloat 的原生悬浮窗，由 **Rust + Tauri 2** 实现，通过系统 WebView 加载 `frontend/` 中的 React 界面。

悬浮窗由 Harness 内的宿主插件启动，通过 stdin/stdout 上的 NDJSON JSON-RPC 通信。它负责悬浮窗口、全局快捷键、菜单栏或系统托盘、窗口位置与偏好保存，以及向前端提供会话状态和转发用户操作。会话执行与历史记录由 Harness 管理，悬浮窗不独立运行 agent。

发布构建会将前端资源嵌入二进制，无需单独启动前端服务器。界面使用系统字体，不附带独立字体文件。普通使用者按[项目 README](../README.md)安装插件即可。

## Rust 依赖

| 依赖 | 用途 |
|---|---|
| `tauri` 2 | 原生窗口、WebView、前端命令与事件、菜单栏或系统托盘 |
| `global-hotkey` | 注册和监听系统全局快捷键 |
| `serde`、`serde_json` | 协议、配置与状态的 JSON 序列化和解析 |
| `tauri-build` | 构建时处理 Tauri 配置与资源 |

具体版本和功能开关见 [Cargo.toml](Cargo.toml)。

## 前端依赖

| 依赖 | 用途 |
|---|---|
| `react`、`react-dom` | 界面组件与渲染 |
| `@tauri-apps/api` | 前端与 Rust 壳之间的命令调用和事件通信 |
| `@tanstack/react-virtual` | 长会话的虚拟列表 |
| `markdown-it`、`dompurify` | Markdown 渲染及生成 HTML 的清理 |

开发工具使用 TypeScript、Vite 和 Vitest；完整依赖见 [frontend/package.json](frontend/package.json)。
