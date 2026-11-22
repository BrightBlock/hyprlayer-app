# Settings popover and page

The header's gear opens a Settings popover with quick rows: Light/Dark/Auto, the default harness, the notify and confirm-quit toggles, and "Update server when idle". Below them are links to Workshop, Servers, Archived worktrees and All settings. All settings opens the full page, which has a section nav on the left. Escape leaves the page.

## Sub-features

- `set-popover-open`: the gear (or `⌘,`) toggles the popover. It stays closed while the server is disconnected.
- `set-popover-harness`: the Default harness dropdown writes `providers.defaultHarness` to `settings.json`.
- `set-popover-toggles`: `Notify when a session needs me` writes `notifications.enabled`, and `Confirm before quitting` writes `quit.confirm`.
- `set-popover-autoupdate`: `Update server when idle` is desktop-local (localStorage), not `settings.json`.
- `set-popover-links`: Servers goes to `/settings?section=servers`, All settings to `/settings`, Workshop to `/workshop`, and Archived opens its own popover.
- `set-nav`: the page's section nav marks the selected section and shows the server count beside Servers.
- `set-escape`: Escape on `/settings` goes back.

## How to get to it (user POV)

- Click the gear in the window header (`Settings`).
- Press `⌘,` (the `nav.settings` default).
- In the popover, click `All settings`, then pick a section from the left nav.

## Driving it with the MCP bridge

Preconditions:

- An isolated instance seeded with `verify-demo` whose doctor passed. Onboarding swallows `⌘,`, so seed first.
- Shared mode works too, but every toggle below writes the user's real `settings.json`. Record the original values with `get_app_settings` first and click them back afterwards.
- `<settings.json>` means `~/Library/Application Support/<IDENTIFIER>/settings.json`, with `IDENTIFIER` from `<run-dir>/run.env`. A fresh isolated profile may have no file until the first save.

- **Open.** Run `webview_interact` `click` on `button[aria-label="Settings"]`. A `[role="dialog"]` appears, `aria-labelledby` its `Settings` heading (no `aria-label`, so select it by role), and the gear gets `aria-expanded="true"`. <!-- src: src/features/settings/SettingsButton.tsx:41 --> <!-- src: src/components/ui.tsx:1190 --> <!-- src: src/features/settings/SettingsPopover.tsx:79 -->
- **Default harness.** Click `button[aria-haspopup="listbox"][aria-label="Default harness"]`, then click the `[role="option"]` with text `Codex` (`strategy: "text"`). The trigger now reads `Codex`. Read back with `invoke('get_app_settings')`: `.providers.defaultHarness === "codex"`. Then `jq .providers.defaultHarness <settings.json>`. Restore `Claude Code` the same way. <!-- src: src/features/settings/SettingsPopover.tsx:131 --> <!-- src: src/features/settings/HarnessPicker.tsx:52 --> <!-- src: src/components/ui.tsx:347 --> <!-- src: src/components/ui.tsx:422 --> <!-- src: src/features/settings/settingsPatches.ts:19 -->
- **Toggles.** Click the label that wraps the switch: XPath `//div[@role="dialog"]//label[span[normalize-space()="Confirm before quitting"]]`. A `strategy: "text"` click resolves to the whole dialog and toggles nothing. The `input[role="switch"]` flips. The read-back is `.quit.confirm` in the RPC and the file. Click again to restore. Do the same for `Notify when a session needs me`, which maps to `.notifications.enabled`. <!-- src: src/features/settings/SettingsPopover.tsx:153 --> <!-- src: src/features/settings/SettingsPopover.tsx:146 --> <!-- src: src/components/ui.tsx:514 --> <!-- src: src/features/settings/settingsPatches.ts:23 --> <!-- src: src/features/settings/settingsPatches.ts:27 -->
- **Auto-update toggle.** Click the `Update server when idle` label the same way. The read-back is `JSON.parse(localStorage.getItem('hyprlayer:server-auto-update')).state.enabled`. Restore it. <!-- src: src/features/settings/SettingsPopover.tsx:168 --> <!-- src: src/features/hosts/serverAutoUpdate.ts:33 -->
- **All settings.** Click the text `All settings`. The popover closes, `location.pathname === "/settings"`, the header crumb reads `Settings`, and `nav[aria-label="Settings sections"] button[aria-current="page"]` is `General`. <!-- src: src/features/settings/SettingsPopover.tsx:101 --> <!-- src: src/features/settings/SettingsNav.tsx:43 --> <!-- src: src/features/settings/SettingsNav.tsx:65 --> <!-- src: src/features/settings/SettingsHeader.tsx:17 -->
- **Nav.** Click `Keyboard` in the nav. `location.search` is `?section=keyboard`, and that button carries `aria-current="page"`. The `Servers` entry shows `hosts.length + 1`. <!-- src: src/routes/Settings.tsx:15 --> <!-- src: src/features/settings/SettingsNav.tsx:71 --> <!-- src: src/features/settings/SettingsView.tsx:33 -->
- **Servers link.** Reopen the popover and click the text `Servers`. You land on `?section=servers`, and the crumb reads `Settings / Servers` with a back button. <!-- src: src/features/settings/SettingsPopover.tsx:89 --> <!-- src: src/features/settings/SettingsHeader.tsx:21 -->
- **Escape.** Run `webview_keyboard` `press` `Escape` on `/settings`. You return to the previous route, or `/`. After the popover's `Servers` link, which pushes a history entry, the first Escape goes back to the earlier section and a second leaves Settings. <!-- src: src/routes/Root.tsx:808 -->
- **Proof.** Save screenshots of the open popover and the page with the nav, plus the `get_app_settings` JSON and a `jq` of the file before and after each toggle.

## Gotchas

- `Default harness` labels three controls: the popover dropdown, its listbox, and General's radiogroup. Scope it with `[aria-haspopup="listbox"]` or `[role="radiogroup"]`. <!-- src: src/features/settings/HarnessPicker.tsx:23 -->
- The popover's switches have no `aria-label`, so their name is the label text. Target the `label` by XPath, as in Toggles.
- The harness, notify and quit rows render only after bootstrap loads settings, but the Appearance and auto-update rows always render. <!-- src: src/features/settings/SettingsPopover.tsx:85 -->
- Section navigation uses `replace`, so one Escape leaves Settings however many nav sections were visited. Popover links push. An unknown `section` falls back to General. <!-- src: src/routes/Settings.tsx:13 --> <!-- src: src/features/settings/sections.ts:17 -->
- The nav's `Workshop` is a link to `/workshop`, not a section.
- Raw `set_app_settings` takes the whole `AppSettings` object, not a patch. Reading back with `get_app_settings` is fine. Writing with the raw call skips the bootstrap cache.
