# App actions in the default engine

With Multiplexer mode on and `App actions open in` set to `Default engine`, app actions open a new PTY tab of the default engine instead of a native chat. The server types the action's prompt into the CLI once its TUI is ready. A new workstream's first agent opens there too. General actions (custom prompts, Review) open in any engine. Stage actions (Commit, Ship, Implement, Validate, Revise) use the CLI only when the default engine is Claude Code.

## Sub-features

- `action-first-agent` opens a new workstream's first `Agent` tab in the default engine.
- `action-prompt` runs a custom prompt from the prompt library in a new engine tab titled after the prompt, and the server pastes its text.
- `action-focus-running` focuses an action's tab while it is still running, instead of launching a second (stage actions only).
- `action-stage-claude-only` sends stage actions to a native chat when the default engine isn't Claude Code.
- `action-launch-failed` removes the tab and shows a `<title> didn't start` toast if the engine fails to launch.

## How to get to it (user POV)

- In Settings > Advanced, set `Default engine` and set `App actions open in` to `Default engine`.
- Create a workstream; its first tab runs the default engine.
- Press ⌘K, pick a custom prompt, and a tab named after it opens running that engine.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance with Multiplexer mode on and the `Stub` engine added (do [terminal-engine-tab.md](./terminal-engine-tab.md) `engine-enable` and `engine-add-cli` first). `verify-demo` is seeded.
- A custom prompt, seeded once and followed by a reload (no event refreshes the bootstrap cache):
  `const s = await window.__TAURI__.core.invoke('get_app_settings'); await window.__TAURI__.core.invoke('set_app_settings', { settings: { ...s, prompts: { ...s.prompts, custom: [...(s.prompts?.custom ?? []), { id: 'verify-echo', name: 'Verify echo', body: 'hl-action-proof' }] } } }); location.reload();` <!-- src: src/features/prompts/prompts.ts:121 -->
- Cost: none with the stub. A stage action needs `claude-code` as default and spends a real model turn, because the server submits the prompt.

- **Pick the default.** In Settings > Advanced:
  - Click `button[aria-label="Default engine"]`, then the `[role="option"]` `Stub`. <!-- src: src/features/settings/AdvancedSettings.tsx:163 --> <!-- src: src/components/ui.tsx:422 -->
  - Confirm `button[aria-label="App actions open in"]` reads `Default engine`. <!-- src: src/features/settings/AdvancedSettings.tsx:181 --> <!-- src: src/features/settings/AdvancedSettings.tsx:186 -->
  - `get_app_settings` gives `multiplexer.defaultEngineId === '<stub id>'`, and `actionsInCli` is `true` or absent (the default is true). <!-- src: src/features/multiplexer/engines.ts:63 -->
- **First agent.** Create a folder workstream (see [workstream-create.md](./workstream-create.md)). Expected:
  - Its only tab, `Agent`, shows the `▸` glyph, not a chat composer.
  - The live `workstreamTabs` store gives `byWs[wsId].shells[0].ptyAgent.engineId === '<stub id>'`. <!-- src: src/features/session/workstreamTabs.ts:399 -->
  - The attach replay (see [terminal-shell-tab.md](./terminal-shell-tab.md)) on its `terminalId` shows `STUB-ENGINE`.
- **Run the prompt.** Press ⌘K (`webview_keyboard` key `k` with the Meta modifier). The `[aria-label="Prompt library"]` combobox opens. <!-- src: src/features/prompts/PromptPalette.tsx:167 --> <!-- src: src/lib/keybindings/commands.ts:36 -->
  Type `Verify echo` into `input[placeholder="Run a prompt…"]`, then click its `[role="option"]` (`strategy: "text"`, `Verify echo`). <!-- src: src/features/prompts/PromptPalette.tsx:187 --> <!-- src: src/features/prompts/PromptPalette.tsx:210 -->
- **Action tab.** A new active tab titled `Verify echo` (title locked) opens on the stub. The tab store row has `ptyAgent.started: false` and no `action` key, because general prompts aren't keyed. <!-- src: src/features/multiplexer/actionAgent.ts:61 --> <!-- src: src/features/multiplexer/actionAgent.ts:69 -->
- **Prompt delivered.** After ~3s of quiet, the server bracket-pastes the body and sends Enter. The replay on the new `terminalId` contains `hl-action-proof`, echoed by the tty and by `cat`. <!-- src: crates/hyprlayer-core/src/runtime/agent_control/spawn.rs:37 --> <!-- src: crates/hyprlayer-core/src/runtime/agent_control/spawn.rs:983 -->
- **Stage gate (read-only check).** With `Stub` as default, import the live `engines` module and run `actionEngine(resolveMultiplexer(settings), 'commit')`. It returns `undefined`, so Commit and Ship open a native chat. This reads the routing; it doesn't verify an entry point. <!-- src: src/features/multiplexer/engines.ts:93 -->
- **Proof.** Screenshots of the first-agent tab, the palette with the row, and the `Verify echo` tab. Save the settings JSON and both replays.

## Gotchas

- Default engine `Claude Code` plus any stage action submits a real prompt in the user's claude account. Never run Commit or Ship this way outside the live test repo (ADR-0012).
- Built-in Commit and Ship rows are hidden in folder workstreams, so only custom prompts show in `verify-demo`'s palette. <!-- src: src/features/prompts/PromptPalette.tsx:64 -->
- A prompt aimed at a tab with a live native session goes into that chat instead. Run it with a PTY or empty tab focused. <!-- src: src/features/session/sessionStore.ts:1800 -->
- The server waits up to 60s for the TUI. A stub that keeps printing never goes quiet, so the prompt is dropped and only logged (`its first prompt was dropped`). <!-- src: crates/hyprlayer-core/src/runtime/agent_control/spawn.rs:35 -->
- A general prompt goes in as typed. Non-claude engines get a leading `/skill` rewritten into prose for stage prompts only. <!-- src: crates/hyprlayer-core/src/pipeline.rs:456 -->
- A launch failure closes the tab and toasts `<title> didn't start`. A missing stub path shows as that toast, not as an exited tab. <!-- src: src/features/multiplexer/actionAgent.ts:103 -->
- The first-agent seed reads the bootstrap cache at seed time. Changing the default engine after a workstream's tabs exist doesn't convert its `Agent` tab. <!-- src: src/features/session/workstreamTabs.ts:420 -->
- Occluded window: the tab's launch waits on `requestAnimationFrame`; see [terminal-engine-tab.md](./terminal-engine-tab.md).
