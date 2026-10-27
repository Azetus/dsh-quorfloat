# CHANGELOG — Releases and engineering record

> This file records each release's user-visible changes and protocol/engineering root causes. Current behavior is
> specified in `ARCHITECTURE.md`; read it when investigating why something is the way it is today. The Chinese
> version is authoritative; keep both in sync in the same commit.
>
> New entries go at the top; keep each entry within about 8 lines. Record only changes to behavior, protocol, or
> releases; fold routine fixes into the adjacent entry. Keep test counts only when they explain a release state,
> and mark them as "at the time". Do not record verification results.
> Third-level headings group topics; the body states the change directly and mentions a reason briefly.

## Version index

| Version | Date | Theme |
|---|---|---|
| 0.1.10 | <date> | Event-driven session list, archived-conversation alignment, and 30-second reconcile |
| 0.1.9 | 2026-10-22 | Diagnostics readability, tray window-position reset |
| 0.1.8 | 2026-10-19 | Host version range, window drag-area guard, development script cleanup |
| 0.1.7 | 2026-10-18 | Diagnostics log export, atomic state writes, Windows build/CI fixes |
| 0.1.6 | 2026-10-14 | Automatic image display, built-in home-page navigation fix, README version placeholders |
| 0.1.5 | 2026-10-13 | Three phases of Markdown external content (links, remote images, local images) |
| 0.1.4 | 2026-10-13 | npm platform packages and GitHub Actions, Windows startup/heartbeat fixes, test grouping |
| 0.1.3 | 2026-10-12 | npm package build entry, npm 12 compatibility, release metadata |
| 0.1.2 | 2026-10-12 | Ungrouped browsing, i18n, system fonts, title projection, pagination alignment |
| 0.1.1 | 2026-10-12 | Chunked snapshots for long sessions, load earlier, button height, bundle filtering |
| 0.1.0 | 2026-10-12 | Version consistency guard, workspace push, automatic bundle/tgz assembly |
| pre-0.1.0 | 2026-10-07 – 10-11 | Host/sidecar/protocol, approvals and questions, Tauri + React migration, tray, pin |

> Versions before 0.1.3 were not published to npm and had no GitHub Release; they are local/offline build records.
> 0.1.10 is not released yet; `<date>` is a placeholder to replace with the actual release date.

## 0.1.10 (<date>)

### Event-driven session list
- The conversation list is now maintained by host pushes: the host subscribes to the Harness's
  `api-session/added|removed|status|activity` and to `sessionProjections`'s `title`, and forwards them
  as `sessions/changed`; the picker and relative times no longer wait for a 3-second poll.
- The sidecar merges pushes into the same list: a name appears but never disappears, `cwd` fills in
  but is not replaced, `updatedAt` only moves forward, and `blank` only lowers. Pushes that land while
  a list answer is in flight are applied first and replayed over that answer.
- `removed` means "left the live registry", not "gone from the persisted list"; it never deletes a row
  and asks for the authoritative list instead. `sessions/list` becomes a 30-second reconcile, with one
  refresh at handshake and on summon, and a 3-second retry for an unanswered request. A restarted
  process still starts from a full list.
- Live titles depend on `sessionProjections`; without it titles follow the reconcile (at most 30s late).

### Archived conversations aligned
- The panel hides conversations the Harness has archived, using the complete archive set the
  workspace stream already carries; the picker, workspace menus, and "ungrouped" all follow it,
  and unarchiving restores the row on the next frame without a list request.
- Archiving the followed conversation leaves it and returns to the new-conversation state
  (host `session/detach`, transcript and in-flight snapshots/pages dropped); archiving a pinned
  conversation clears the pin silently (marker only, no `pinNotice`).
- Late snapshots for an archived conversation are dropped; the panel offers no "show archived"
  view — archiving and restoring belong to the Harness window.

### Comments and docs
- Corrected two stale claims in `follow.rs` ("no session-appeared notification", "follow the newest conversation").
- `ARCHITECTURE.md` documents `sessions/changed` and the list-refresh rules; `AGENT.md` lists the new markers.

## 0.1.9 (2026-10-22)

### Diagnostics readability
- Exported snapshot sections changed from JSON fragments to flat text.
- Data sources, redaction, size limits, and file writing are unchanged; a JSON export is not provided.

### Tray window-position reset
- Added "Reset Window Position" to the menu bar/notification-area menu; it moves the panel to the current
  `window.anchor` on the primary display's work area.
- While hidden it only moves the window, without showing it or taking focus; a missing display or failed
  move writes a marker and leaves visibility unchanged.

## 0.1.8 (2026-10-19)

### Host range and DSH 0.2.1-rc.2
- `package.json` now declares `engines.dsh: ">=0.2.0-rc.2"` with a lower bound only, matching
  the most common community form. Since DSH 0.2.1-alpha.3 this range takes precedence over peerDependencies.
- Diagnostics `dshVersion` now reads the running
  `@deepseek-ai/dsh-app-boot/package.json`, the same source as DSH's `getDshRuntimeVersion()`.
  `DSH_VERSION` is only a fallback; if neither resolves, the value is `unknown`.
- The session-controller contract passes 11/11 for 0.2.0-rc.1 through 0.2.1-rc.2; the full
  unit suite passes 95/95 on rc.2 with Cordis 4.0.5. CI pins contract 0.2.1-rc.2 and Cordis 4.0.5.
- A lower-only range leaves future DSH versions installable but unverified; the range must be
  updated after adaptation.

### Window drag-area guard
- The old rule treated any positive overlap as visible. A window could be dragged to a sliver at
  the edge or behind the Dock, menu bar, or taskbar, leaving its top drag strip almost unclickable.
- Before startup, showing, or a hotkey/tray action, the top drag strip must overlap a display's
  **work area** by at least 64x24 logical px; otherwise the panel returns to the primary display's anchor.
  A partially visible window whose strip is still usable stays where it is.
- A visible window whose strip is unusable is rescued rather than hidden by the hotkey/tray.
  There is no monitor polling and no tray reset item; if a visible window is unplugged and the platform does not
  move it, a later summon is required.

### Development script cleanup
- Removed the dead `--perf` and `DSH_QUORFLOAT_PERF`; the variable came from the egui era and has no consumer in
  the Tauri/React sidecar.
- Replaced hardcoded product and DSH versions in test data with neutral values so they cannot go stale.

## 0.1.7 (2026-10-18)

### Atomic state writes
- `preferences.json`, `pinned.json`, and `window.json` were rewritten in place; an interruption,
  disk error, or Windows sharing conflict could leave a half-written file and lose settings, pins, or the window
  position on the next start.
- Saving now uses a temporary file in the same directory, `sync_all`, then rename; Unix preserves the symlink target and
  permissions, while Windows uses `MoveFileExW` with replacement. On failure the old file is kept and the
  `preferences save failed`, `pins save failed`, or `window position save failed` marker is written.
- Tests keep only two invariants: a normal replacement, and a forced rename failure that keeps the old file.

### Diagnostics log export
- "Export diagnostics log" in the settings page writes the current state, recent host supervisor
  events, and sidecar stderr as a `.log` under `~/.dsh-quorfloat/logs/` and shows the absolute path.
  `DSH_QUORFLOAT_LOG_DIR` overrides the directory; dev runs are isolated under `.dev-home/logs/`.
- Log limits are 100 host events/64 KiB, 200 stderr lines/32 KiB, 2 KiB per line, and a 256 KiB export; the oldest
  entry is dropped first. Snapshot IDs are shortened and home paths become `~`; conversation text and
  credentials are excluded.
- DSH's own logs and the dev/CI-only marker are not read; there is no clipboard or browser
  download; each manual export leaves one file, and files are not cleaned up automatically.

### Windows build and CI tooling
- The frontend build points `TEMP` inside the workspace, avoiding dsh/esbuild conflicts with Windows directory
  integrity constraints.
- Added `npm run ci:install` to install the upstream contract dependencies under `.tooling/ci`; the workflow calls
  it, and `ci:test` now gives an actionable error when those dependencies are missing.
- `contract:extract` walks path segments when looking up the asar entry, fixing Windows backslash paths being
  treated as one filename.

## 0.1.6 (2026-10-14)

### Automatic image display
- Local and remote Markdown images no longer require the "Load image" confirmation; the component
  enters loading on mount and can retry after failure.
- Remote images use `lazy`/`async`/`no-referrer`; local images resolve against the session cwd
  and absolute paths may lie outside the cwd; `file://`, UNC, and `~` are rejected; formats are limited to
  PNG/JPEG/WebP/GIF; the limit is `min(Harness maxImageBytes, 8 MiB)`.
- At most two active reads globally in the frontend; a cancelled waiter is not read; leaving the
  component releases the Blob; `session/image` returns 256 KiB of raw bytes per request, expires after 30 seconds,
  and the sidecar allows at most four in-flight requests.

### Built-in home-page navigation fix
- The host reported that it had started, but the panel was invisible; the marker showed
  `webview navigation blocked` immediately after the window/tray were created.
- Tauri reduces `index.html` to `tauri://localhost`, whose path is the empty string; the allowlist
  only accepted `/` and `/index.html`, so it blocked the startup page itself.
- The fix accepts the empty path under the built-in scheme/host restriction; external navigation and scheme
  restrictions are unchanged.

### README version placeholders
- The Chinese and English root READMEs now use `<version>` in tgz filenames, npm version examples, and build
  directory examples, with a note to replace it with the actual version.

## 0.1.5 (2026-10-13)

### Markdown external content: links
- Web links accept only complete HTTP(S) URLs; the frontend intercepts clicks and middle-clicks, and Rust hands
  the URL to the system browser through the Tauri Opener helper; in-page anchors are preserved; other navigation
  and new-window requests are rejected.
- Raw HTML continues to be shown as text; DOMPurify continues to sanitize Markdown output.
- CSP is enabled in production and development: scripts, fonts, objects, frames, forms, and base are restricted;
  `img-src` allows self/http/https/blob; Tauri IPC, inline layout styles, and the dev Vite WebSocket are preserved.
- Added `tauri-plugin-opener` (Rust helper); no file-open command is registered and no frontend npm dependency is
  added.

### Markdown external content: remote images
- Markdown emits only inert `data-image-src`/`data-image-alt` placeholders; DOMPurify still forbids `img`; the
  `AnswerBody` component mounts the controlled `MarkdownImage` component over the placeholder.
- The first render does not request the image; clicking creates the `img`, which decodes asynchronously, sends no
  Referer, and can retry after failure; the image width does not exceed the answer area, there is no zoom overlay,
  and clicking the image or button does not activate the wrapping link.
- All new copy comes from the Chinese/English dictionaries.

### Markdown external content: local images
- The host reads through Harness `fs.resolve/readBytes` in `src/harness/images.ts`; the session cwd comes from
  `sessionController.list`; a missing cwd blocks only relative paths, not absolute ones.
- `session/image` generates a random token on the first read and then reads chunks by offset; completion,
  cancellation, and failure release the snapshot actively; Rust keeps an independent in-flight table with a
  12-second timeout and does not consume the follow/prompt slot.
- The frontend validates chunk identity/offset/size/format before assembling a Blob URL; leaving the component
  cancels and releases it; image data never enters the session snapshot.

### Navigation allowlist fix
- Same root cause as the 0.1.6 entry; 0.1.6 contains the fix.

## 0.1.4 (2026-10-13)

### npm packages and release flow
- The main package is the unscoped `dsh-quorfloat` (the name was restored in 0.1.3); the platform packages are
  `@azetus/dsh-quorfloat-darwin` and `@azetus/dsh-quorfloat-win32-x64`.
- The main package's `cordis.patch.yml` has an empty `quorfloatPath`; the host resolves the platform package's
  `bin` through Node module resolution; `optionalDependencies` pin both platforms to the same version; the binary
  is not copied and no postinstall is used.
- Added the GitHub Actions workflow `publish-npm.yml`: manual runs build and keep artifacts without publishing;
  a published Release triggers npm publishing. It uses npm OIDC and publishes the platform packages before the
  main package after both platform builds and tests pass.
- The publish script skips a package when its name/version already exists; existing versions keep their original
  contents and are not replaced. When the version query returns 404 it reads dist-tags, avoiding duplicate
  uploads while the registry is still scanning.

### Windows startup and supervision fixes
- The sidecar Reader starts before WebView initialization; during initialization, shutdown/EOF flushes the
  response before exiting with status 0.
- Tauri's `setup` runs inside `app.run`, so `shell_ready` must be set at `RunEvent::Ready`; setting it earlier lets
  the "exit immediately on cold start" branch cover the real setup.
- Host heartbeats now allow one in-flight request per channel; stopped/stale-generation results are ignored;
  duplicate stops coalesce; a manual stop invalidates a pending automatic restart.
- `transport closed` no longer falsely reports that the process exited; a still-live child is actually cleaned up.

### Tests and history
- Eight real-sidecar tests moved to `tests/desktop/rust-peer.test.mjs`; cloud `ci:test` runs only
  protocol/upstream/robustness tests and does not start the desktop sidecar. `test:unit` / `test:desktop` / `test`
  select files explicitly and fail on an empty suite.
- The local Git history was compressed (130 → 71 commits), preserving version tags and the final tree; it was not
  pushed. Details of the old history are no longer recoverable, so this file keeps only the root causes that
  explain current behavior.

## 0.1.3 (2026-10-12)

- Added `npm run build:npm`: builds the main package and the current platform dependency directory/tgz, sharing
  the release compilation, lipo, hashing, and npm pack logic.
- The main package keeps Harness metadata, README, and LICENSE; platform packages contain only the binary,
  metadata, readme, and license, with no `dsh.bundle/client`.
- npm 12 changed `npm pack --json` from an array to a package-name-keyed object; the script accepts both shapes
  and still requires a unique filename and a valid basename.
- Added repository/homepage/bugs metadata; README resources ship with the offline and npm packages, and links in
  the npm main package point to absolute GitHub URLs.
- Version 0.1.3 is a transition version; during this period the main package name was restored to the unscoped
  `dsh-quorfloat`, while platform packages kept the `@azetus/...` prefix.

## 0.1.2 (2026-10-12)

### Ungrouped browsing
- The workspace picker appends "Ungrouped" after the real workspaces; it has no pin button and cannot be a
  creation target; that scope hides "New conversation".
- Membership is derived only from the workspace registry's `sessionIds` ∩ the visible session list; cwd is not
  compared.
- Choosing a real workspace, clicking a specific conversation, a change of followed conversation, or summoning
  again exits explicit browsing.
- This is a deliberate extension of the Harness picker; although Harness's ungrouped tree shows a create icon,
  its `onCreate` does nothing without a `workspaceId`.

### Titles and lists
- Session titles now read `sessions/list`'s `projections.values.title`; the row itself has no top-level `title`.
- Only when the projection is missing/null and `updatedAt` has moved does the host use `inspect` to fold
  `session/title`, at most once per list.

### Pagination and fonts
- The initial snapshot and later pages are aligned to whole turns: `maxMessages:200`,
  `turnWindow:{minMessages:50,minTurns:2}`; large pages use `session/history/begin|part|end`, and the whole page
  is prepended only after it is complete.
- The fold key changed from "question + position" to "question + ordinal counted from the newest end", so
  prepending an older page no longer invalidates fold memory across the thread.
- Bundled fonts were removed in favor of the system font stack; WebView performs character fallback and no font
  assets are downloaded or distributed.
- The bundle filters `.DS_Store`, `._*`, and `__MACOSX`; the manifest `files` is an exact file list; a tgz is
  generated automatically after the build.

## 0.1.1 (2026-10-12)

### Blank long conversations
- The whole conversation arrived in one `session/snapshot` frame; both sides cap frames at 1 MiB,
  oversized frames were silently dropped, the snapshot never applied, every later `session/event` was treated as
  stale, and the panel stayed permanently blank without retrying.
- Oversized frames now carry byte counts and are counted; added
  `session/snapshot/begin|part|end` (256 KiB budget) and `session/snapshot/request`; the sidecar assembles by
  `sessionId+generation`, folds only when complete, and resends missing parts within a bound; the old
  single-frame `session/snapshot` is still accepted as a one-part case.
- When parts and events share a channel, forwarding must be serialized frame by frame; otherwise
  events overtake their preceding parts and are judged stale.

### Load earlier
- The initial snapshot was previously fixed at about 50 messages; added
  `session/history {sessionId,beforeSeq}` → `{records,hasMore,cursor}`.
- The page budget is 512 KiB, each record is hard-capped at 32 KiB, and pages over 512 KiB are split into roughly
  256 KiB parts.
- Scroll compensation uses the **row anchor**'s real start delta, not the total height delta; the anchor is
  released when the user scrolls or clicks "Back to bottom".
- "Load earlier" appears only when `hasMore` is true and the scroll position is at the top (`scrollTop ≤ 1px`).

### Other
- The "Load earlier" pill was crushed to a 10px border box by `align-items: stretch` in a zero-height flex slot;
  it now uses `align-items:flex-start` plus a fixed in-button layout.
- The version was rolled back from temporary 0.1.2/0.1.3 to 0.1.1; an older same-name tgz predates the fix and
  must not be reused.

## 0.1.0 (2026-10-12)

- The version source of truth was reduced to the root `package.json` and `quorfloat/Cargo.toml`;
  `scripts/version.mjs` checks them as the first step of a release build and refuses to continue on a mismatch.
- `quorfloat/tauri.conf.json` does not declare `version`; Tauri falls back to the crate version. If packaging
  later switches to `tauri build`, add `bundle.macOS.bundleVersion` or the version field.
- `PROTOCOL_VERSION` remains `quorfloat/1`, independent of the product version.
- Workspace push `workspaces/changed`: baseline/upsert/remove/order; `archived` and `pinned` are forwarded but not
  applied.
- Bundle assembly and tgz packaging are automated; macOS metadata is excluded, temporary directories are cleaned
  on failure, and an old package must not masquerade as a new one.

## Pre-release phase (2026-10-07 – 2026-10-11)

### Host, sidecar, and protocol
- Built the TypeScript host plugin plus the Rust sidecar over stdio NDJSON JSON-RPC 2.0: handshake, heartbeat,
  graceful shutdown, restart budget, stdout purity, the 1 MiB frame limit, and the marker observation surface.
- The host waits for `sessionController` through Cordis `inject`; probing once reports a false missing-service
  result.
- `user/message` content is not wrapped in `message`, while other record types are; `tool-call.arguments` is a
  JSON string; user messages are identified by `source.kind`. These shapes came from real captured frames, not
  guesses.
- Idempotency-key issue: Harness compares `session/prompt`'s `requestId` against `source.rpcId` in history; a hit
  returns accepted without accepting the prompt. A per-session counter reused old keys after a restart, producing
  "the input box was cleared but the message was not sent". The key is now
  `{sessionId}:prompt-{pid}-{milliseconds}-{global sequence}`, and the host also rejects keys already spent in
  that session.

### Sessions, approvals, and questions
- The session policy changed from "follow the most recently updated" to "a single pin or the new-session state";
  the pin says which conversation the next summon opens, and the workspace is its projection.
- The new-session workspace follows the P0–P4 ladder; the conversation is created only on the first submit.
- Approval authority is decided by `visible && hasFocus`; the panel claims only the currently attached
  conversation; claiming uses `prepend` for exclusivity.
- Questions gained single-choice, multi-choice, and free-text controls, and `CAPABILITIES` added `question`; the
  answer shape and limits are in the root `ARCHITECTURE.md`.
- `interaction/hint` only means "this request was not handed to the panel"; `panel/lifecycle` lets the tray
  restart or stop itself.

### Tauri and React migration
- The view layer moved from egui to Tauri 2.12 + WKWebView; the host-side spawn/env/protocol stayed unchanged.
- After the migration, a framework-free TS frontend landed first, then became React + pure logic + hooks; the
  thread became windowed.
- The migration confirmed that the handshake must precede window creation; Tauri's `setup` runs inside
  `app.run`; the `.app` form is not required, and a bare executable plus a platform package is enough.
- Tray, window dragging and position memory, `reduceMotion`, focus return, the main full-screen Space path, and
  multi-display position memory landed on macOS.
- The old screenshot/frame-buffer tools stopped working after the migration and were retired; do not rebuild them
  the old way.

### Earlier UI and delivery
- The settings page replaces the conversation; theme/blur/hotkey/language; the hotkey field records on click,
  bare keys are rejected, and Esc/Enter yield while recording.
- The three footer groups are permissions, model/effort, and statistics; the statistics rules and the
  "cache-hit rate must not round up to 100%" rule were finalized during this period.
- The fold rule changed from aligning with upstream `turn-process` to "one fold per turn, all other LLM output
  goes into it"; background events draw no rows.
- Markdown table width limiting, soft wrapping for long ASCII strings, the scrollbar gutter, the back-to-bottom
  button, the summon animation, and the height transition were completed during this period.
- Release engineering evolved from hand-written bundles to `release.mjs` + `release-npm.mjs` + CI; platform
  packages and the version consistency guard were added step by step.
