# Run and Setup scripts

A project's Setup, Run and Archive scripts live in `<project>/.hyprlayer/settings.toml` and are edited in Project settings. Setup runs once when a workstream is created, and the dock's Setup tab shows whether it passed. Run starts on ▶ in a PTY in the dock, with `HL_PORT` set. A `localhost:<port>` line in its output adds an `Open :<port>` button, and pressing ▶ again stops it.

## Sub-features

- `scripts-edit` edits a script in Project settings, autosaving to `.hyprlayer/settings.toml`.
- `scripts-run` starts Run in the dock and shows its output live.
- `scripts-port` detects the port from Run output and shows `Open :<port>`.
- `scripts-stop` stops Run, then keeps its captured output.
- `scripts-clear` clears the shown output.
- `scripts-setup-on-create` runs Setup when a workstream is created and shows `Passed`/`Failed` on the Setup tab.
- `scripts-empty` shows `No Run script configured for this project.` with `Edit scripts`.

## How to get to it (user POV)

- Edit: open the project's context menu and choose `Project settings…`. Or in the dock, click `Script options` then `Configure`, or `Edit scripts`.
- Run: open the dock, then press `Run` / `Start Run`, or ⌘R (`scripts.run`).
- Setup: create a workstream after a Setup script is saved, then open the dock's `Setup` tab.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance seeded with `verify-demo` (non-git) and one folder workstream, opened. Folder mode runs Setup on create; in-place and attached modes don't. <!-- src: crates/hyprlayer-server/src/dispatch.rs:953 -->
- `document.hidden === false`.
- Cost: none. Scripts are plain shell.

- **Empty state.** Open the dock and click the `Run` dock tab (`strategy: "text"`, see Gotchas). It shows `No Run script configured for this project.` <!-- src: src/features/session/ScriptDock.tsx:463 -->
- **Edit.** Click `Edit scripts`. The `Project settings: verify-demo` dialog opens. `webview_keyboard type` into `textarea[aria-label="Run script"]`: <!-- src: src/features/session/ScriptDock.tsx:469 --> <!-- src: src/features/projects/ProjectSettingsDialog.tsx:162 --> <!-- src: src/features/projects/ScriptsCard.tsx:146 -->
  `echo "port=$HL_PORT" > hl-run-proof.txt; echo "listening on http://localhost:$HL_PORT"; sleep 300`
  Wait for `Saved` (500ms debounce). <!-- src: src/features/projects/ScriptsCard.tsx:105 -->
- **Read back the edit.** `cat <SEED>/.hyprlayer/settings.toml` shows a `[scripts]` table with that `run`. `invoke('get_scripts', { projectPath })` returns the same value. <!-- src: crates/hyprlayer-core/src/repo_settings.rs:120 --> Close the dialog.
- **Run.** Click `Start Run`. Its text includes the `⌘R` hint, so match it with `//button[starts-with(normalize-space(),"Start Run")]`. <!-- src: src/features/session/ScriptDock.tsx:480 --> Expected:
  - The output shows `listening on http://localhost:<N>`.
  - `button` text `Open :<N>` appears, and the split button reads `Stop`. <!-- src: src/features/session/ScriptDock.tsx:242 --> <!-- src: src/features/session/ScriptDock.tsx:404 -->
  - The status bar dock button gets its dot.
- **Read back the run.**
  - `invoke('get_script_runtime', { workstreamId })` has `runTerminalId` set, `port` N, and `statuses[{kind:'run', phase:'running'}]`.
  - `invoke('list_running_scripts')` contains the workstream id.
  - `cat hl-run-proof.txt` reads `port=N`.
  - The attach replay from [terminal-shell-tab.md](./terminal-shell-tab.md) on `runTerminalId` shows the output. <!-- src: src/lib/bindings.ts:253 --> <!-- src: crates/hyprlayer-core/src/runtime/scripts.rs:421 -->
- **Stop.** Click the split button (now `Stop ⌘R`). `[process exited]` is written, `list_running_scripts` drops the id, and `get_script_runtime` reports `runTerminalId: null` and run `failed` (killed is a non-zero exit). <!-- src: src/features/session/ScriptRunView.tsx:84 --> <!-- src: crates/hyprlayer-core/src/runtime/scripts.rs:438 -->
- **Output kept.** `invoke('get_script_output', { workstreamId, kind: 'run' })` returns the captured text. <!-- src: crates/hyprlayer-core/src/runtime/scripts.rs:446 -->
- **Clear.** Click `button[aria-label="Clear output"]`. The pane empties, and `get_script_output` returns `null`. <!-- src: src/features/session/ScriptDock.tsx:252 -->
- **Setup on create.** In Project settings, type `echo setup-ran > hl-setup-proof.txt` into `textarea[aria-label="Setup script"]` and wait for `Saved`. Create a second folder workstream. Open its dock and click the `Setup` tab. <!-- src: src/features/session/ScriptDock.tsx:43 -->
  - The header reads `Passed · ran <time>`, with a check on the tab. <!-- src: src/features/session/ScriptDock.tsx:508 -->
  - `cat hl-setup-proof.txt` reads `setup-ran`.
  - `get_script_runtime` for the new id has `{kind:'setup', phase:'passed', exitCode:0}`.
- **Proof.**
  - Screenshots of the empty state, the editor showing `Saved`, Run with `Open :N`, the stopped state, and Setup `Passed`.
  - Save `settings.toml`, the runtime JSON, and both proof files.

## Gotchas

- Text `Run` matches the dock tab, the split button and `Start Run`. Use `Start Run`, or the button just before `[aria-label="Script options"]`. In the right-hand dock the split button is icon-only, with `aria-label` `Run`/`Stop`. <!-- src: src/features/session/ScriptDock.tsx:393 -->
- Setup on create runs to completion, not in a PTY (`run_lifecycle`), so the tab shows captured output, not a live terminal. `Run setup` in the dock re-runs it in a PTY. <!-- src: crates/hyprlayer-core/src/runtime/scripts.rs:263 -->
- Setup and Run auto-run arbitrary shell from the project folder. Keep scripts inside the seed dir.
- Saving writes `.hyprlayer/` into `verify-demo`, so the workstream-create recipe's `ls -A` shows more than `README.md` afterwards.
- Port detection reads only `localhost:`/`127.0.0.1:`/`0.0.0.0:`/`[::1]:`/`[::]:` URLs; `listening on http://[::]:<port>` was detected. A bare `port 5173` doesn't count. <!-- src: crates/hyprlayer-core/src/scripts.rs:25 -->
- `Open :<port>` calls `openUrl`, which opens the system browser. Don't click it in a run.
- A Run started before a reload reattaches through `runTerminalId`. If that terminal is gone, the view resets instead of replaying. <!-- src: src/features/session/ScriptRunView.tsx:102 -->
- Occluded window: `ScriptRunView` waits two frames before `script_run`, so nothing starts while `document.hidden`. <!-- src: src/features/session/ScriptRunView.tsx:108 -->
