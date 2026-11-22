# Settings: harness, keyboard and agent preferences

The full Settings page stores server-backed preferences in the profile's `settings.json`. They cover the default harness (General), confirm-before-quit (General), per-command keyboard shortcuts (Keyboard) and agent-control limits plus the Lead Engineer and its guardrails (Agent control). The Harnesses section holds a tab per harness. The Full Disk Access toast's dismissal is stored in the same file.

## Sub-features

- `pref-default-harness`: General's `Default harness` radiogroup (Claude Code, Codex, OpenCode, Ollama). It shares its value with the popover's dropdown.
- `pref-quit`: General's `Confirm before quitting` switch, stored as `quit.confirm`.
- `pref-key-unbind` / `pref-key-reset`: Keyboard's `Unbind` writes `keybindings.overrides[<id>] = null`. `Reset` deletes the key.
- `pref-key-record`: `Edit` records a new chord, and `Save` commits it.
- `pref-harness-tabs`: Harnesses has the Claude Code, Codex (`NEW`), OpenCode and Ollama tabs.
- `pref-agent-control`: the `Access` dropdown (Off, Read, Full) and its numeric limits, stored under `agentControl`.
- `pref-lead`: Agent control's `Lead Engineer` card: the `Start a Lead on every project` switch (on by default), `Lead model`, `Lead effort` and the `Lead max live children` stepper, stored under `agentControl.lead`. It renders only when the server returns a `lead` object. <!-- src: src/features/settings/AgentControlSettings.tsx:121 --> <!-- src: src/features/settings/settingsPatches.ts:63 -->
- `pref-guardrails`: the `Guardrails` card. Each rule has a switch named by its title and a `Remove <title>` button. `Add rule` opens `Rule title` / `Rule text` inputs whose `Add` stays disabled until both are filled. Each git project with a Lead adds a `<name> guardrails` scope dropdown. <!-- src: src/features/settings/AgentControlSettings.tsx:199 --> <!-- src: src/features/settings/AgentControlSettings.tsx:273 -->
- `pref-fda-notice`: a `Full Disk Access needed` toast when macOS denies it. Dismissing it sets `fullDiskAccessNoticeDismissed`.

## How to get to it (user POV)

- Settings popover, then `All settings`. General opens first.
- Pick `Keyboard`, `Harnesses` or `Agent control` in the left nav.
- The FDA toast appears on its own at launch.

## Driving it with the MCP bridge

Preconditions:

- An isolated instance seeded with `verify-demo`, connected (these sections need bootstrap). <!-- src: src/features/settings/SettingsView.tsx:61 -->
- In shared mode these writes hit the user's real `settings.json`. Copy the file to the evidence dir first and undo each change through the UI.
- `<settings.json>` is `~/Library/Application Support/<IDENTIFIER>/settings.json`. The file is camelCase JSON. <!-- src: crates/hyprlayer-core/src/settings.rs:633 --> <!-- src: crates/hyprlayer-core/src/settings.rs:66 -->

- **Default harness.** On General, click the `[role="radio"]` with text `OpenCode` inside `[role="radiogroup"][aria-label="Default harness"]`. It gets `aria-checked="true"`. Read back `invoke('get_app_settings')` `.providers.defaultHarness === "opencode"` and `jq .providers.defaultHarness <settings.json>`. Reopen the popover, whose dropdown now reads `OpenCode`. Restore `Claude Code`. <!-- src: src/features/settings/GeneralSettings.tsx:56 --> <!-- src: src/features/settings/HarnessPicker.tsx:23 --> <!-- src: src/lib/useRovingRadio.ts:45 -->
- **Quit confirm.** Click `input[role="switch"][aria-label="Confirm before quitting"]`. `.quit.confirm` flips in the RPC and the file. Click again to restore. <!-- src: src/features/settings/GeneralSettings.tsx:209 -->
- **Unbind a shortcut.** Open Keyboard. Commands are grouped by category, and `Open Settings` is under Navigation. The row's buttons are bare `Edit`/`Reset`/`Unbind`, so tag the right one first: `webview_execute_js` `[...document.querySelectorAll('p')].find(p => p.textContent === 'Open Settings').closest('div.flex.items-start').querySelectorAll('button').forEach(b => b.dataset.verify = b.textContent)`. Then click `button[data-verify="Unbind"]`. The row's chord pill reads `Unbound`, the popover header loses its `⌘,` hint, and `jq '.keybindings.overrides["nav.settings"]' <settings.json>` is `null`. <!-- src: src/features/settings/KeyboardSettings.tsx:94 --> <!-- src: src/features/settings/KeyboardSettings.tsx:141 --> <!-- src: src/features/settings/KeyboardSettings.tsx:174 --> <!-- src: src/lib/keybindings/commands.ts:58 --> <!-- src: src/features/settings/SettingsPopover.tsx:82 -->
- **Reset it.** Click `button[data-verify="Reset"]`; it is enabled only while an override exists. The pill shows `⌘,` again, and the key is gone from `overrides` (`jq 'has("nav.settings")'` on `.keybindings.overrides` is `false`). <!-- src: src/features/settings/KeyboardSettings.tsx:138 --> <!-- src: src/features/settings/KeyboardSettings.tsx:169 -->
- **Record.** Click `button[data-verify="Edit"]`. The row shows `Press a shortcut…` with `Save`/`Cancel`. Run `webview_keyboard` `press` `y` with `modifiers` `["Meta","Shift","Alt"]`; the pill reads `⌘⌥⇧Y`. A taken chord adds `Already used by <command>`. <!-- src: src/features/settings/KeyboardSettings.tsx:118 --> Re-tag the row's buttons (they re-render), click `button[data-verify="Save"]`, and `.keybindings.overrides["nav.settings"]` reads `mod+alt+shift+y`. Click `Reset` to restore. <!-- src: src/features/settings/KeyboardSettings.tsx:83 --> <!-- src: src/features/settings/KeyboardSettings.tsx:105 --> <!-- src: src/features/settings/KeyboardSettings.tsx:83 --> <!-- src: src/features/settings/KeyboardSettings.tsx:105 -->
- **Harness tabs.** Open Harnesses and click `Codex` (`strategy: "text"`). That tab button has `aria-current="true"`. Its content probes the codex binary, which is read-only. The `OpenCode` tab lists `Providers` and `Models`; its `OpenCode default model` dropdown renders only once a model is added under Models. Don't pick from it: it writes the user's global `~/.config/opencode/opencode.json` (SKILL.md Gotchas). <!-- src: src/features/settings/harnesses/OpenCodeHarnessTab.tsx:200 --> <!-- src: src/features/settings/harnesses/OpenCodeHarnessTab.tsx:234 --> <!-- src: crates/hyprlayer-core/src/env.rs:141 --> <!-- src: src/features/settings/harnesses/HarnessesSection.tsx:63 --> <!-- src: src/features/settings/harnesses/HarnessesSection.tsx:90 -->
- **Agent control.** Open Agent control, click `button[aria-haspopup="listbox"][aria-label="Access"]`, then the option `Off`. `.agentControl.level === "off"` in the RPC and the file. Restore `Read`, the default. <!-- src: src/features/settings/AgentControlSettings.tsx:62 --> <!-- src: src/features/settings/settingsPatches.ts:48 -->
- **Lead (`pref-lead`).** On Agent control, click the label wrapping `input[role="switch"][aria-label="Start a Lead on every project"]` (XPath `//label[.//input[@aria-label="Start a Lead on every project"]]`). `.agentControl.lead.enabled` flips in `get_app_settings`. Click again to restore. A folder project never gets a Lead, so this changes nothing visible in an isolated run seeded with `verify-demo`. <!-- src: crates/hyprlayer-core/src/runtime/agent_control/leads.rs:23 -->
- **Guardrails (`pref-guardrails`).** Click `Add rule` (`strategy: "text"`). `input[aria-label="Rule title"]` and `[aria-label="Rule text"]` (placeholder `What the Lead does, or never does`) appear, and `Add` is disabled. Click `Cancel`. The `<name> guardrails` rows need a git project, such as the `review_lifecycle` sandbox (review-workspace.md).
- **FDA notice (conditional).** Run `invoke('full_disk_access_status')`. Only on `"denied"`, with `fullDiskAccessNoticeDismissed` false, does a `[role="status"]` toast titled `Full Disk Access needed` show at launch. Click its `button[aria-label="Dismiss"]`. `.fullDiskAccessNoticeDismissed` becomes `true`. On `"granted"` the flag resets to `false` by itself, and no toast is drivable. Off macOS the status is always `unknown`, so the toast never shows. <!-- src: crates/hyprlayer-core/src/system.rs:31 --> <!-- src: src/features/system/FullDiskAccessNotice.tsx:29 --> <!-- src: src/features/system/FullDiskAccessNotice.tsx:51 --> <!-- src: src/features/system/FullDiskAccessNotice.tsx:22 --> <!-- src: src/components/Toast.tsx:99 -->
- **Proof.** Save screenshots of each changed control, plus the `get_app_settings` JSON and a `jq` of `<settings.json>` before and after.

## Gotchas

- In the keyboard section, `Edit`, `Reset` and `Unbind` repeat on every row. A text click hits the first row (Navigation's first command), not the one you mean.
- Agent control shows two `Max live children` steppers: the section's own and the Lead's (`Lead max live children`). Select by `aria-label`.
- `/settings` keeps a `?project=<path>` search param across section changes; the Guardrails card reads it to sort that project first. <!-- src: src/routes/Settings.tsx:15 -->
- Harness tabs use `aria-current="true"/"false"`, while the section nav uses `aria-current="page"`.
- The FDA toast's `Open Settings…` launches macOS System Settings. Don't click it in a verify run. The toast is sticky. A later toast replaces it without running its dismiss, so the flag stays unset and the toast comes back next launch. <!-- src: src/features/system/FullDiskAccessNotice.tsx:37 --> <!-- src: src/lib/toast.ts:39 -->
- Opening General runs `hyprlayer ai status --json` and `self-update --check` through the Hyprlayer card. CLI 1.6.4 records telemetry and rewrites `telemetry.lastFlush` in the shared `~/Library/Application Support/hyprlayer/config.json`. <!-- src: src/features/settings/HyprlayerCard.tsx:348 -->
- The Full Disk Access probe reads the real user TCC database, so the result is machine state, not profile state. <!-- src: crates/hyprlayer-core/src/system.rs:29 -->
- A `settings.json` that fails to parse is moved aside as `settings.json.unreadable-<ms>`, and the server starts from defaults. Never hand-edit the file while the server runs. <!-- src: crates/hyprlayer-core/src/settings.rs:688 -->
