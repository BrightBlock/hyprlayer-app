---
name: verify-hyprlayer
description: Launch, health-check and drive the Hyprlayer desktop app (Tauri webview over the MCP bridge) to prove a frontend or backend change at the real UI, with screenshots and side-effect evidence. Use whenever a change needs verifying in the running app, not just tests.
---

# Verify Hyprlayer in the running app

The surface is the Tauri desktop app's webview. Only a debug `tauri dev` build is drivable: `tauri-plugin-mcp-bridge` is compiled out of release builds. The server (`hyprlayer-server`) runs as a separate process the app spawns. Other surfaces (the `hyprlayer-server` CLI, remote hosts over ssh) are covered by `crates/hyprlayer-e2e`, not this skill.

Helper: `.claude/skills/verify-hyprlayer/verify.sh launch isolated|shared|attach <app-data-dir> | doctor <run-dir> | cleanup <run-dir>`. Run it from the repo root. It runs on macOS and Linux.

## Platform paths and keys

| | macOS | Linux |
|---|---|---|
| Profile config (`settings.json`, `hosts.json`, `account.json`) | `~/Library/Application Support/<ID>/` | `~/.config/<ID>/` |
| Profile data (`hyprlayer.db`) | `~/Library/Application Support/<ID>/` | `~/.local/share/<ID>/` |
| Server socket | `<data dir>/hyprlayer.sock` | `$XDG_RUNTIME_DIR/hyprlayer/<hash>.sock` <!-- src: crates/hyprlayer-server/src/socket.rs:11 --> |
| Shared CLI config | `~/Library/Application Support/hyprlayer/config.json` | `~/.config/hyprlayer/config.json` <!-- src: crates/hyprlayer-core/src/env.rs:118 --> |
| Run dirs | canonical `/private/var/…` | plain `/tmp/…` |
| `mod` in a chord | ⌘ (`Meta` in `webview_keyboard`, `metaKey` in a `KeyboardEvent`) | Ctrl (`Control`, `ctrlKey`) <!-- src: src/lib/keybindings/chord.ts:98 --> |
| File mode | `stat -f %Sp` | `stat -c %A` |

Recipes write ⌘ chords; read them as Ctrl on Linux. A few defaults differ by platform beyond that: `nav.dashboard` and the Library are `ctrl+1`/`ctrl+2` on macOS but `alt+1`/`alt+2` elsewhere. <!-- src: src/lib/keybindings/commands.ts:47 -->

## Launch

Pick the mode by what the feature needs:

- **isolated** (default): a throwaway Tauri identifier `ai.brightblock.hyprlayer.verify-<ts>`, its own empty SQLite, server and socket, and Vite on the first free port in 1431–1449. It runs beside the release app and anyone's `.dev` instance.
  ```bash
  .claude/skills/verify-hyprlayer/verify.sh launch isolated
  ```
  The script prints the run dir (`$TMPDIR/hl-verify/verify-<ts>`), then blocks until the log says `connected to this machine`, then runs doctor. It takes about 1–2 minutes warm and several minutes cold. Ready means doctor passed. Attach only after that: attaching while setup is still connecting deadlocks the app.
- **shared**: the `.dev` profile (`ai.brightblock.hyprlayer.dev`, Vite 1430), with the user's real projects, sessions and thoughts tree. Use it only for features that need real data, such as a plan under `thoughts/shared/plans`. If 1430 is already listening, the script says so and exits: attach to that instance, never start a competitor or kill it.
- **attach**: an isolated app that is a client of another profile's server, for a second desktop on one session (session-sync.md) or the `review_lifecycle` sandbox (review-workspace.md). It writes `hosts.json` with `localProfile` set to that server's app-data dir, waits for `connected to`, and never spawns a server of its own.
  ```bash
  .claude/skills/verify-hyprlayer/verify.sh launch attach <app-data-dir>
  ```
  The target server must already be serving. On Linux it must also share the desktop's `XDG_RUNTIME_DIR`, or the app looks for the socket in the wrong place.
  A free 1430 can still leave a `.dev` server running with live agents. The app attaches to it and replaces it only when it's below `MIN_SERVER_VERSION`, so check its version in the `.dev` log first. Cleanup leaves that server running. `Hyprlayer Dev.app` is the `.main` profile, not `.dev`. <!-- src: src-tauri/src/supervisor.rs:134 -->

`launch` creates a non-git seed folder and records it as `SEED` in `<run-dir>/run.env`. Run dirs are canonical (`pwd -P`) because the app stores resolved paths: on macOS a `/var/…` path matches nothing stored under `/private/var/…`. Once the page renders (Drive, step 1), import the seed from `webview_execute_js`:
```js
(async () => { await window.__TAURI__.core.invoke('import_local_folder', { path: '<SEED>', directoryName: null }); location.reload(); })()
```
Then click `a[aria-label^="Home"]`. Don't use `location.replace('/')`: on load the router replays `hyprlayer:last-route`, so `/` lands back on the last route. <!-- src: src/router.tsx:201 -->
Use a non-git folder. A git import runs `thoughts init`, which writes the shared CLI config (see the platform table) that every profile uses.

Teardown is `cleanup` (below).

## Doctor

```bash
.claude/skills/verify-hyprlayer/verify.sh doctor <run-dir>
```
It is read-only. It checks that `tauri dev` is alive, that the app binary is this repo's `target/debug/Hyprlayer`, that Vite owns its port, that the bridge port is owned by that app pid, that the app connected, and that a `target/debug/hyprlayer-server` serves this profile (by its socket on macOS, by the `HYPRLAYER_APP_DATA_DIR` it was given on Linux). An attach run checks the target's server instead, and that the app spawned none. It prints the git build and the bridge port. Run it before the first drive, after anything surprising, and after any failed drive. A non-zero exit means don't drive.

## Drive

1. `driver_session start` with `port` set to the bridge port doctor printed (9223 unless another debug app took it). When two apps share a bundle id, pass the numeric port as `appIdentifier`.
   - A session left from an earlier run answers `Already connected` while pointing at a dead identifier. Run `driver_session status` and check its `identifier` against `IDENTIFIER` in `run.env`; if they differ, `stop` and `start` again.
   - Then wait for the page to render: doctor passing doesn't mean Vite has served the page. A first load took about 2 minutes. Poll `webview_execute_js` with `document.readyState === 'complete' && document.getElementById('root').childElementCount > 0` every ~10s.
   - `webview_wait_for` gives up after about 5s whatever its `timeout`, so it's no good for this wait. `webview_execute_js` has the same ~5s cap, so poll with one call per check (an in-script wait of ≤4s), not a loop.
2. Resize to ≤1800 logical px wide before screenshotting (`manage_window resize`, e.g. 1400×860). Wider windows time out.
3. Prefer stable handles: `aria-label`s (`Settings`, `Add project`, `New workstream in <project>`, `Server: <name>`, `Open manual checks`), `data-testid`s (`plan-goal-action`, `phase-goal-action`, `criterion-outcome`), dialog titles and placeholders. `webview_interact` with `strategy: "text"` clicks by label, and `right-click` opens context menus.
   - Names repeat across the sidebar, the header breadcrumb, the Files pane and the dashboard (`Verify map seed`, `README.md`, `Agent` vs `Agent 2`), so a text selector often needs `nth`. An XPath scoped by role, such as `//*[@role="tab"][normalize-space()="Agent"]`, is steadier.
   - `strategy: "text"` resolves to the nearest interactive ancestor. Text inside a non-interactive `label` can resolve to a whole dialog and click its center; use an XPath like `//div[@role="dialog"]//label[span[normalize-space()="<text>"]]`.
   - The rail's Home is `a[aria-label^="Home"]`: its label becomes `Home (Multiplexer mode on)`. <!-- src: src/routes/Root.tsx:1081 -->
4. `webview_keyboard type` replaces a controlled input's value (it never appends), so retype to change a field. Enter doesn't submit a form under the bridge, so click the submit button.
   - `press` with `modifiers` reaches `window` keydown handlers: `Meta`+`k` on macOS (`Control`+`k` on Linux) and the keybinding recorder work. A synthetic `contextmenu` dispatched from JS doesn't open the sidebar's row menus; use `webview_interact` `right-click`.
   - It can't empty a field: `press Backspace` doesn't edit the value. Type the next value over it, or, as a seed, set it through the native `value` setter and dispatch `input`.
   - In xterm, `press Enter` sends nothing. End the `type` text with `\n` instead.
5. To read or set live app state, import the exact module URL the page loaded. After HMR it carries a `?t=` stamp, and a plain `import('/src/…')` returns a stale duplicate instance:
   ```js
   const u = performance.getEntriesByType('resource').map(e => e.name).filter(n => n.includes('planPreviewStore')).pop();
   const store = await import(u);
   ```
6. Raw IPC: `window.__TAURI__.core.invoke('<snake_case_command>', { camelCaseArgs })`. Every required key must be present (e.g. `list_workstreams` needs `includeArchived`). Raw IPC skips react-query invalidation, so the UI won't refresh from it. A bare top-level promise comes back as `{}`, so wrap the call in an async IIFE.

The feature recipes are in [features/README.md](features/README.md).

## Evidence

Write everything to the `EVIDENCE` dir from `<run-dir>/run.env`, which is `target/verify/<run-id>/` (gitignored). Use `webview_screenshot` with `format: "png"` and a `filePath` inside it, and number files in drive order (`00-launched.png`, `01-dialog.png`, …). Proof standards:

- Drive the user's path (clicks, menus, typed text). Use IPC and store imports only to seed or to read back, never as the action under test.
- Capture the action and the resulting state, not just the final screen.
- Read side effects back through a second view: a list RPC's JSON, the file on disk, `git` refs. Save them as `.json`/`.txt` beside the screenshots.
- A file you edit in the shared thoughts tree is committed and pushed by auto-sync within seconds. Copy it into the evidence dir first, revert it right after, and use the `thoughts/` symlink path, never `~/thoughts`. An app write can reset the file mode, and `cp` over it keeps the new mode, so check `git -C thoughts/shared diff --summary` after the revert.
  - An isolated profile with no mapped git project doesn't sync: a scratch doc it created stayed uncommitted for two minutes. <!-- src: src/features/library/useThoughtsSync.ts:65 -->
  - A Library save keeps the mode: a `755` doc stayed `755` (#403). A plan tick once reset one to `644` (plan-tab.md); on Linux a tick kept a `755` plan at `755`.
- `localStorage` in the shared profile holds the user's routes and scopes. Snapshot it into `sessionStorage` at attach (it survives a reload) and restore any changed keys before cleanup.

## Cleanup

```bash
.claude/skills/verify-hyprlayer/verify.sh cleanup <run-dir>
```
It kills only the process tree under the recorded pnpm pid. For isolated and attach runs it stops that profile's own server (servers outlive the app by design; an attach run has none) and removes the identifier's config, data and log dirs. It also deletes the run dir, including the seed. It copies `dev.log` into the evidence dir first, and evidence is never removed. Run it after every failed iteration too. Afterwards nothing should listen on the Vite port (`lsof -nP -iTCP:<port> -sTCP:LISTEN`, or `ss -ltn` on Linux). Never `pkill Hyprlayer` or free port 1430: that kills the user's app.

## Gotchas

- Never type `/model <x>` or press Enter in claude's `/model` picker inside a PTY tab. From claude 2.1.330 that Enter is "save": it writes the user's default model to `~/.claude/settings.json` and their claude account, which no isolated profile covers. Use the picker's `s` (this session only) or the status bar's model pill, or start the tab with `--model`.
- An isolated profile does not isolate OpenCode's own config. Setting OpenCode's default model in Settings writes the user's global `~/.config/opencode/opencode.json` (`model`), and a run of an older `opencode` binary with the real `HOME` re-pins `~/.config/opencode/package.json`. Copy both into the evidence dir before touching OpenCode settings and restore them before cleanup; pick the model per session in the status bar instead of changing the default where you can.
- Plain `tauri dev` uses the release identifier, so single-instance forwards to the user's app and the dev build exits 0 with an empty log. Always pass a config; the helper does.
- `tauri dev` doesn't rebuild the externalBin sidecar. After a server change, run `node scripts/build-server-sidecar.mjs --debug` first, or the stale sidecar overwrites `target/debug/hyprlayer-server` (symptom: `unsupported_command`).
- The shell exports the release profile's `HYPRLAYER_APP_*` dirs. A hand-run `hyprlayer-server stop` without overriding all three stops the release server. The helper's cleanup sets them.
- Webview storage persists per Vite origin, so an isolated run on a port an earlier run used opens on that run's last route ("No hyprlayer project is mapped at …"). Click the rail's Home; a reload replays the stale route.
- An occluded dev window stalls `requestAnimationFrame`: terminals never open and screenshots go stale. Check `document.hidden`.
- Bridge clicks send no pointer events, so `onPointerDown` handlers never fire. `:hover`/`:focus` styles don't apply while the window lacks OS focus.
- A git checkout switch (e.g. `gh pr merge --delete-branch`) makes Vite reload every module, and the dev app may exit. Run doctor again.
