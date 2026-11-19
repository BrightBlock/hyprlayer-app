# Terminal dock shell

A workstream's Terminal & scripts dock holds login-shell terminals that run in the workstream's folder. The dock opens on shell terminal 1. `+` adds more terminals, and `×` kills one. Closing the dock only unmounts its terminals: the server keeps each PTY, and reopening the dock reattaches and replays its output. Dock position (bottom or right sidebar) and height persist.

## Sub-features

- `term-dock-toggle` opens and closes the dock from the status bar or with ⌘\`.
- `term-shell-open` starts a login shell in the workstream folder and shows it in xterm.
- `term-shell-add` adds a terminal tab named after the shell.
- `term-shell-close` kills that tab's PTY.
- `term-reattach` reattaches to the same PTY after the dock is closed and reopened, with its output intact.
- `term-dock-position` moves the dock between the bottom and the right sidebar.

## How to get to it (user POV)

- Open a workstream, then click the status bar's console icon (`Terminal & scripts dock`).
- Press ⌘\` (`terminal.open`) inside a workstream.
- In the dock, click `+` for another terminal, or a tab's `×` to end one.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance seeded with `verify-demo` and one folder workstream (follow [workstream-create.md](./workstream-create.md), or seed with `create_workstream` `{ projectPath, title: 'Term verify', base: null, mode: 'folder', branch: null, branchPrefix: null }`). Open the workstream.
- `document.hidden === false`. Check it before every terminal step (see Gotchas).
- Cost: none. A plain shell spends no model turns.

Helpers (paste into `webview_execute_js`):

- Live dock store: `const u = performance.getEntriesByType('resource').map(e => e.name).filter(n => n.includes('/features/session/scriptsStore')).pop(); const { useScriptsStore } = await import(u); useScriptsStore.getState().dock['<wsId>']`. `termIds` maps tab number to terminal id. <!-- src: src/features/session/scriptsStore.ts:11 -->
- Output read-back. A second attach replays the server's ring from its start. Release the channel afterwards; the PTY keeps running. <!-- src: crates/hyprlayer-core/src/terminal/mod.rs:134 --> <!-- src: src/lib/bindings.ts:240 -->
  ```js
  const { Channel, invoke } = window.__TAURI__.core;
  const ch = new Channel(); let out = '';
  ch.onmessage = (e) => { if (e.kind === 'output') out += new TextDecoder().decode(Uint8Array.from(atob(e.data), c => c.charCodeAt(0))); };
  const alive = await invoke('terminal_attach', { terminalId: '<id>', onEvent: ch });
  await new Promise(r => setTimeout(r, 1500));
  ch.onmessage = () => {}; await invoke('terminal_release', { channelIds: [ch.id] });
  ({ alive, tail: out.slice(-1500) })
  ```

Steps:

- **Open dock.** Click `button[aria-label="Terminal and scripts dock"]`. `aria-pressed` becomes `true`, the dock shows a shell tab labelled with `default_shell_name` (e.g. `zsh`), and an xterm `.xterm-screen` mounts. <!-- src: src/features/statusbar/ScriptsDockButton.tsx:18 --> <!-- src: src/features/session/ScriptDock.tsx:217 -->
- **Shell started.** Within ~3s, the store's `dock[wsId].termIds[1]` holds a uuid. Run the read-back on it: `alive: true`, and the tail shows a shell prompt. <!-- src: src/features/session/ScriptDock.tsx:279 -->
- **Type.** Click `.xterm-screen`, then `webview_keyboard type` `pwd; echo HL_TERM_OK\n` into `textarea.xterm-helper-textarea`. End the text with `\n`: a separate `press Enter` never reaches the PTY. The read-back tail contains `<SEED>` and `HL_TERM_OK`. Folder mode's worktree path is the project folder. <!-- src: crates/hyprlayer-core/src/runtime/terminals.rs:206 --> <!-- src: node_modules/@xterm/xterm/css/xterm.css:61 -->
- **Side effect.** Type `touch hl-term-proof\n`. `ls -A <SEED>` shows `hl-term-proof`. Delete it afterwards.
- **Add.** Click `button[aria-label="New terminal"]`. A second shell tab is active, and `termIds[2]` appears with a different id. <!-- src: src/features/session/ScriptDock.tsx:192 -->
- **Close one.** Click the active tab's `[aria-label="Close terminal"]` (a `span[role=button]`). The tab goes, `termIds[2]` is deleted, and the read-back on the old id returns `alive: false`. <!-- src: src/features/session/ScriptDock.tsx:222 -->
- **Reattach.** Note `termIds[1]`, then click `button[aria-label="Minimize scripts dock"]` and reopen with the status bar button. Closing the active terminal left the dock on the `Run` tab, so click the shell tab (text `zsh`). <!-- src: src/features/session/scriptsStore.ts:162 --> `termIds[1]` is unchanged, and a screenshot shows the earlier `HL_TERM_OK` output replayed. <!-- src: src/features/session/ScriptDock.tsx:170 --> <!-- src: src/features/session/TerminalTab.tsx:77 -->
- **Position.** Click `button[aria-label="Toggle dock position"]`. The dock moves into the right sidebar, whose minimize button reads `Toggle scripts dock`. Click again to restore `bottom`. <!-- src: src/features/session/ScriptDock.tsx:179 -->
- **Proof.** Screenshots of the dock closed, open, with two tabs, and reattached. Save the read-back JSON before and after the close.

## Gotchas

- The main strip has no plain shell tab. `ShellTab.terminalView` renders a `TerminalTab`, but nothing in `src/` creates one, so shells live only in the dock. <!-- src: src/routes/ProjectPage.tsx:1244 -->
- No handle exposes the live xterm buffer. `Terminal` is private to `XtermEngine`, and the WebGL renderer draws to a canvas, so DOM text is empty. Read output through the attach replay above, and use screenshots only for layout. <!-- src: src/lib/terminal/terminalEngine.ts:88 -->
- An occluded window stalls `requestAnimationFrame`. `TerminalTab` waits two frames before `terminal_open`, so the pane stays blank and `termIds` stays empty. Bring the window forward and check `document.hidden`. <!-- src: src/features/session/TerminalTab.tsx:86 -->
- Bridge `type` reaches xterm, including a trailing `\n`; `press Enter` doesn't. `terminal_write` (`{ terminalId, data: base64 }`) only seeds input; it isn't the user path. <!-- src: src/lib/bindings.ts:236 -->
- The shell is your `$SHELL -l`, so profile output (such as a mise banner) lands in the replay. <!-- src: crates/hyprlayer-core/src/runtime/terminals.rs:1070 -->
- The replay is capped ring scrollback. Very long output loses its head.
- Dock position and height persist in webview storage for this Vite origin. Restore `bottom` before cleanup.
- The `Resize scripts dock` separator is `onPointerDown` only, so a bridge click can't drag it. <!-- src: src/features/session/ScriptDock.tsx:147 -->
