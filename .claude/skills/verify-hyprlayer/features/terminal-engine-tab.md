# Engine tab (PTY agent)

With Multiplexer mode on, the agent strip's `+` opens a menu of CLI engines (Claude Code, Codex, Gemini, Antigravity, and any added CLI). Picking one opens a tab that runs that CLI in a persistent PTY in the workstream folder. The tab shows the engine glyph and a state dot. The status bar shows that agent's controls: always Restart, plus permission, model, effort and context for a hooked claude or codex.

## Sub-features

- `engine-enable` turns on Multiplexer mode in Settings > Advanced.
- `engine-add-cli` adds a custom CLI engine (name, command, arguments).
- `engine-open` opens an engine tab from the `New agent` menu.
- `engine-state-dot` sets the tab dot from output: working, blocked (a `y/n` prompt), done, idle.
- `engine-restart` relaunches the PTY from the status bar.
- `engine-close` archives the tab and kills its PTY.
- `engine-hooked-controls` adds permission, model, effort and ctx pills for a hooked claude-code or codex agent.

## How to get to it (user POV)

- Open `Settings`, then `All settings`, then `Advanced`, and switch on `Multiplexer mode`. `Add CLI…` adds an engine.
- In a workstream, click the strip's `+` (`New agent`) and pick an engine. ⌘T opens the default engine.
- The `New agent` menu's `Configure engines…` goes back to Settings > Advanced.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance seeded with `verify-demo` and one folder workstream, opened. Engines are app settings in that profile's own `settings.json`, not the shared CLI config. <!-- src: crates/hyprlayer-core/src/settings.rs:548 -->
- A stub engine script in the run dir: `<run-dir>/seed/stub-engine.sh`, containing `#!/bin/sh` + `echo "STUB-ENGINE cwd=$(pwd)"` + `exec cat`, then `chmod +x`.
- Cost: the stub spends nothing. For `engine-hooked-controls` you need `claude` installed and signed in. An idle claude TUI spends no turn until you send a prompt, but a model change from the bar drives its `/model` picker.

- **Enable.** Click `button[aria-label="Settings"]`, then `All settings`, then `Advanced` in `nav[aria-label="Settings sections"]` (the popover has no Advanced link), then `input[aria-label="Multiplexer mode"]` (a `role="switch"` checkbox). The engines row, `Default engine` and `App actions open in` appear. <!-- src: src/features/settings/SettingsButton.tsx:41 --> <!-- src: src/features/settings/SettingsNav.tsx:23 --> <!-- src: src/features/settings/AdvancedSettings.tsx:127 -->
- **Add the stub.**
  - Click `Add CLI…`. A `Add a CLI engine` dialog opens. <!-- src: src/features/settings/AdvancedSettings.tsx:154 --> <!-- src: src/features/settings/AdvancedSettings.tsx:247 -->
  - Type `Stub` into `input[placeholder="My CLI"]` and the script's absolute path into `input[placeholder="my-cli"]`. <!-- src: src/features/settings/AdvancedSettings.tsx:259 --> <!-- src: src/features/settings/AdvancedSettings.tsx:267 -->
  - Click `form button[type="submit"]` (`Add`). A `Stub` chip appears.
- **Read back the settings.** `invoke('get_app_settings')` has `.multiplexer.enabled === true`, and `.multiplexer.engines` holds `{label:'Stub', command:'<path>', builtin:false}`. Record its `id`, a random uuid. <!-- src: src/features/settings/AdvancedSettings.tsx:86 --> `defaultEngineId` is still `claude-code`, so any workstream created now opens claude in its first tab. Set `Default engine` to `Stub` next (terminal-action-engine.md) unless you mean to launch claude. <!-- src: src/features/multiplexer/engines.ts:28 -->
- **Open.** Go back to the workstream. Click `[aria-label="New agent"]`, then `Stub` (`strategy: "text"`). <!-- src: src/routes/ProjectPage.tsx:1166 --> <!-- src: src/routes/ProjectPage.tsx:1570 --> Expected:
  - A `role="tab"` pill `Agent N` with the `▸` glyph becomes active. <!-- src: src/routes/ProjectPage.tsx:641 -->
  - Within ~3s, the replay shows `STUB-ENGINE cwd=<SEED>`.
- **Bound terminal.** Find the id with `const u = performance.getEntriesByType('resource').map(e => e.name).filter(n => n.includes('/features/session/workstreamTabs')).pop(); (await import(u)).useWorkstreamTabsStore.getState().byWs['<wsId>'].shells.find(s => s.ptyAgent)?.ptyAgent`. It gives `{ engineId: '<stub id>', terminalId }`. Run the attach replay from [terminal-shell-tab.md](./terminal-shell-tab.md) on `terminalId`: `alive: true`, with the `STUB-ENGINE` line. <!-- src: src/features/session/workstreamTabs.ts:63 -->
- **State dot.** Click the terminal and `webview_keyboard type` `Proceed? (y/n)\n`. `cat` echoes it, and the engine's state becomes `blocked`. Read it from the live `multiplexerStore`: `useMultiplexerStore.getState().agents['<wsId>:<shellId>'].state`. With no more output it stays `blocked`. <!-- src: src/features/multiplexer/engineState.ts:44 --> <!-- src: src/features/multiplexer/multiplexerStore.ts:56 -->
- **Restart.** With the stub tab focused, the status bar shows only `button[aria-label="Restart"]`; no harness means no model or permission pill. Click it. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:264 --> <!-- src: src/features/multiplexer/PtySessionControls.tsx:30 --> Expected:
  - The tab's `ptyAgent.terminalId` changes.
  - The replay on the old id returns `alive: false`.
  - The new id shows a fresh `STUB-ENGINE` line.
- **Close.** Click the active tab's `[aria-label="Archive Agent N"]`. The pill goes, its `terminalId` attach returns `false`, and the shell moves to `byWs[wsId].archived`. <!-- src: src/routes/ProjectPage.tsx:230 -->
- **Hooked controls (optional, needs claude).** Open `Claude Code` from `New agent`, and wait for the TUI to paint. Expected:
  - `invoke('list_pty_agents', { workstreamId })` returns its row with `hooked: true` and `status.model` set.
  - The status bar shows `[aria-label^="Permission mode: "]`, `[aria-label^="Model: "]` and, once reported, `[aria-label^="Effort: "]`. <!-- src: src/lib/bindings.ts:227 --> <!-- src: src/features/statusbar/SessionControlsCluster.tsx:300 --> <!-- src: src/features/statusbar/SessionControlsCluster.tsx:352 --> <!-- src: src/features/statusbar/SessionControlsCluster.tsx:419 -->
  - Picking a model calls `pty_agent_set_model`. Verify the change with `list_pty_agents` `status.model`, not the agent's own words.
  - Close the tab when done so claude exits.
- **Proof.**
  - Screenshots of Settings with the stub chip, the `New agent` menu, the stub tab, and the bar's `Restart`.
  - Save the `get_app_settings` JSON, plus the replay before and after Restart.

## Gotchas

- `list_pty_agents` lists only hooked agents (claude-code, codex) or agent-spawned ones, so a stub or gemini tab never appears there. Prove those through the tab store's `terminalId` and `terminal_attach`. <!-- src: crates/hyprlayer-core/src/runtime/agent_control/pty_agents.rs:255 --> <!-- src: crates/hyprlayer-core/src/runtime/terminals.rs:550 -->
- Hooks also need Agent control above `off` (the default is `read`). With it off, claude launches unhooked and the bar shows only Restart. <!-- src: crates/hyprlayer-core/src/runtime/agent_control/mod.rs:94 -->
- `Arguments` split on whitespace with no quoting, so pass a script path as the command rather than a quoted one-liner. <!-- src: src/features/settings/AdvancedSettings.tsx:93 -->
- The server launches `exec <command> <args>` under `$SHELL -lc` in the workstream folder. A missing command shows as an immediate `[agent exited]`. <!-- src: crates/hyprlayer-core/src/runtime/terminals.rs:869 --> <!-- src: src/features/multiplexer/AgentPtyTab.tsx:121 -->
- With Multiplexer mode off, the menu never opens; `+` adds a native chat. <!-- src: src/routes/ProjectPage.tsx:680 -->
- The strip's close button is `invisible` on inactive tabs and missing on the last tab. Select the tab first, and keep a second tab open. <!-- src: src/routes/ProjectPage.tsx:240 -->
- An unhooked tab's state is a regex over the last ~4000 chars. Any output marks it `working`, then `done` after 700ms quiet. <!-- src: src/features/multiplexer/agentRuntime.ts:25 -->
- No handle exposes the xterm buffer. Read text through the attach replay.
- Occluded window: `AgentPtyTab` waits two frames before `pty_launch`, so nothing launches while `document.hidden`. <!-- src: src/features/multiplexer/AgentPtyTab.tsx:150 -->
- A codex tab with no session first mints one through `codex exec` and shows `Creating session…` for up to 90s. <!-- src: src/features/multiplexer/AgentPtyTab.tsx:375 -->
- Engines persist in the profile's settings. Isolated cleanup removes them; on shared, remove the stub (`[aria-label="Remove Stub"]`) and switch Multiplexer back. <!-- src: src/features/settings/AdvancedSettings.tsx:53 -->
