# quorfloat

[简体中文](README.md) | English

The native floating window for dsh-quorfloat, built with **Rust + Tauri 2**. It uses the system WebView to load the React interface in `frontend/`.

The host plugin inside Harness starts this sidecar, which communicates through NDJSON JSON-RPC over stdin/stdout. It manages the floating window, global shortcuts, menu bar or system tray, saved window position and preferences, and supplies session state to the frontend while forwarding user actions. Harness manages session execution and history; the sidecar does not run an agent independently.

Release builds embed frontend assets in the binary, so no separate frontend server is needed. The interface uses system fonts and does not bundle font files. Users can install the plugin by following the [project README](../README_EN.md).

## Rust Dependencies

| Dependency | Purpose |
|---|---|
| `tauri` 2 | Native windows, WebView, frontend commands and events, menu bar or system tray |
| `tauri-plugin-opener` | Open validated web links in the system default browser |
| `global-hotkey` | Register and listen for system-wide keyboard shortcuts |
| `serde`, `serde_json` | JSON serialization and parsing for the protocol, configuration, and state |
| `tauri-build` | Process Tauri configuration and resources at build time |

See [Cargo.toml](Cargo.toml) for exact versions and feature flags.

## Frontend Dependencies

| Dependency | Purpose |
|---|---|
| `react`, `react-dom` | UI components and rendering |
| `@tauri-apps/api` | Command calls and event communication between the frontend and the Rust shell |
| `@tanstack/react-virtual` | Virtualized lists for long conversations |
| `markdown-it`, `dompurify` | Markdown rendering and sanitization of the generated HTML |

Development tools include TypeScript, Vite, and Vitest. See [frontend/package.json](frontend/package.json) for the full dependency list.
