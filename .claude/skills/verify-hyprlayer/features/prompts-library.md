# Prompt library

Prompts are reusable agent instructions. Two are built in (Commit and Ship, whose text you can edit and reset), and the rest are custom. Each custom prompt has a name (which makes it a `/slug` composer command), a body with `{{input}}` and variable tokens, per-project overrides, an optional shortcut, and a Pin to composer toggle. They're managed in Workshop → Prompts and run from the `mod+k` palette in the focused workstream. A prompt with inputs asks for them first. Prompts live in the profile's own app settings.

## Sub-features

- `prompts-open` opens Workshop → Prompts from the Settings popover, the palette's `Manage prompts…`, or a legacy `/prompts` route.
- `prompts-filter` covers the nav filters (`All prompts`, `Pinned to composer`, `Project overrides`) and the `Filter…` text box.
- `prompts-create` adds an `Untitled prompt`, renames it (the `/slug` chip follows), and edits its body (auto-saved after 600ms or on blur).
- `prompts-pin` flips Pin to composer, which shows a `Pinned` badge.
- `prompts-builtin` edits a built-in body to get `Modified` and `Reset to default`.
- `prompts-delete` asks to confirm, then removes the prompt and selects Commit.
- `prompts-palette` lists built-ins, custom prompts and `Manage prompts…`, disabled with reasons when no workstream is focused.
- `prompts-run` runs a custom prompt in the focused workstream, through the `{{input}}` args dialog when it has inputs.
- `prompts-io` imports from and exports to `.hlprompts` files.

## How to get to it (user POV)

- Click `Settings` (window header gear), then `Workshop`, then the `Prompts` section.
- Use the `prompts.palette` chord (default `mod+k`), then choose `Manage prompts…`.
- Use the palette to run a prompt; the footer says where it runs.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** is enough. Settings are stored per identifier in `~/Library/Application Support/<identifier>/settings.json`, and cleanup deletes them. <!-- src: crates/hyprlayer-core/src/runtime/mod.rs:177 --> <!-- src: crates/hyprlayer-core/src/settings.rs:633 -->
- Running a prompt (`prompts-run`) starts a real agent turn: $0 on a free OpenCode tab, paid on Claude. It needs a workstream with an agent tab. Built-ins never show for a folder workstream like `verify-demo`'s. <!-- src: src/features/prompts/PromptPalette.tsx:64 --> <!-- src: src/features/prompts/runPrompt.ts:58 -->
- In shared mode, edits change the user's real prompts. Copy `get_app_settings().prompts` into the evidence dir first and restore it after.

Steps:

- **Open.** Click `button[aria-label="Settings"]`, then `Workshop` (`strategy: "text"`). Then, in `nav[aria-label="Workshop"]`, click `Prompts`. The list header reads `All prompts · 2 prompts`, with `Built-in` (Commit, Ship) and `Custom` sections reading `No custom prompts yet.`, and the URL has `section=prompts`. <!-- src: src/features/settings/SettingsButton.tsx:41 --> <!-- src: src/features/settings/SettingsPopover.tsx:182 --> <!-- src: src/components/workshop.tsx:106 --> <!-- src: src/features/workshop/useWorkshopChrome.ts:47 --> <!-- src: src/features/prompts/PromptLibrary.tsx:703 -->
- **Create.** Click `New prompt`. An `Untitled prompt` row is selected, the detail shows `input[aria-label="Prompt name"]`, and the chip reads `/untitled-prompt`. Read back with `invoke('get_app_settings')`: `.prompts.custom` holds one entry named `Untitled prompt`. <!-- src: src/features/prompts/PromptLibrary.tsx:576 --> <!-- src: src/features/prompts/PromptLibrary.tsx:308 --> <!-- src: src/features/prompts/PromptLibrary.tsx:733 --> <!-- src: src/features/prompts/PromptLibrary.tsx:742 -->
- **Rename and body.** Select-all in `input[aria-label="Prompt name"]` and type `Verify echo`; the chip reads `/verify-echo`. Type `Say {{input:word}} back.` into `textarea[placeholder="What should the agent do when this prompt runs?"]`. The list row's meta reads `/verify-echo · 1 input`. After ~600ms, `get_app_settings` returns that body, and `settings.json` contains `Verify echo`. <!-- src: src/features/prompts/PromptLibrary.tsx:900 --> <!-- src: src/features/prompts/PromptLibrary.tsx:75 --> <!-- src: src/features/prompts/PromptLibrary.tsx:693 -->
- **Pin.** Click `[role="switch"][aria-label="Pin to composer"]`. The text reads `Shown in the composer bar`, the row shows `Pinned`, and `Pinned to composer` counts 1. <!-- src: src/features/prompts/PromptLibrary.tsx:1016 --> <!-- src: src/features/prompts/PromptLibrary.tsx:1019 --> <!-- src: src/components/ui.tsx:514 -->
- **Filter.** Type `echo` into `input[aria-label="Filter prompts"]`. Only `Verify echo` remains, under Custom. <!-- src: src/features/prompts/PromptLibrary.tsx:633 -->
- **Built-in.** Empty the filter (the bridge can't; seed it with the native `value` setter plus an `input` event) and click `Commit`, then `type` into the textarea (it replaces the body, which is enough to modify it). A `Modified` badge and `Reset to default` appear. Click `Reset to default`, and `get_app_settings().prompts.builtinBodies.commit` is gone. <!-- src: src/features/prompts/PromptLibrary.tsx:753 -->
- **Palette (no workstream).** On `/`, run `webview_keyboard` `press` `k` with `modifiers` `["Meta"]`. `[role="combobox"][aria-label="Prompt library"]` lists Commit, Ship, Verify echo and `Manage prompts…`. Each run row has `aria-disabled="true"` with the title `Open a workstream to run prompts`. Click `Manage prompts…`, and the route becomes `/workshop?section=prompts`. <!-- src: src/routes/Root.tsx:796 --> <!-- src: src/features/prompts/PromptPalette.tsx:167 --> <!-- src: src/features/prompts/PromptPalette.tsx:61 --> <!-- src: src/features/prompts/PromptPalette.tsx:212 --> <!-- src: src/features/prompts/PromptPalette.tsx:126 -->
- **Run (costly, optional).** Open the `Verify map seed` workstream with an agent tab, open the palette and click `Verify echo`. A `[role="dialog"][aria-label="Prompt arguments"]` asks for `word` and has a `Run` button. Fill its `input` and click `Run`. The focused agent tab runs it, as a follow-up if its session is live. The user row is a collapsed card reading `Verify echo: <word>` · `1 line`, and the log's `userMessage` has `text: "Say <word> back."` and `promptName: "Verify echo: <word>"`. $0 on the free OpenCode path ([session-send.md](./session-send.md)). <!-- src: src/features/prompts/PromptArgsDialog.tsx:14 --> <!-- src: src/features/prompts/PromptArgsDialog.tsx:44 --> <!-- src: src/features/prompts/PromptArgsDialog.tsx:60 --> <!-- src: src/features/prompts/runPrompt.ts:19 -->
- **Delete.** Select `Verify echo` and click the detail's `Delete`. A dialog titled `Delete “Verify echo”?` opens; click its `Delete` (`[role="dialog"] button`, text `Delete`). The row is gone, Commit is selected, and `.prompts.custom` is empty. <!-- src: src/features/prompts/PromptLibrary.tsx:321 --> <!-- src: src/features/prompts/PromptLibrary.tsx:758 -->
- **Proof.** Save screenshots of each state, plus the `get_app_settings().prompts` JSON after create, pin and delete.

## Gotchas

- The palette's only entry is the `prompts.palette` chord; no button opens it. Dispatching a keydown reaches the real resolver (`event.key`, `metaKey` = `mod` on mac). `webview_keyboard` `press` with `Meta` does reach it. Calling `togglePromptPalette()` from the store doesn't count as verifying `prompts-open`. <!-- src: src/lib/keybindings/useKeybindings.ts:40 --> <!-- src: src/features/prompts/promptsStore.ts:11 -->
- Name and body edits are debounced (600ms), so read back after the wait or after a blur. Create, delete and pin save immediately. <!-- src: src/features/prompts/PromptLibrary.tsx:897 -->
- Two `Delete` buttons exist while the confirm is open. Scope the click to `[role="dialog"]`.
- `Export prompts` opens an in-app `Export prompts` dialog (prompt checkboxes, `Include keyboard shortcuts`, `Include project overrides`, a file name, `Export N prompt(s)`); only its final save is native. `Import prompts` goes straight to a native file dialog, so report it unreachable past the button. <!-- src: src/features/prompts/PromptLibrary.tsx:610 --> <!-- src: src/features/prompts/PromptLibrary.tsx:619 --> <!-- src: src/features/prompts/PromptLibrary.tsx:610 --> <!-- src: src/features/prompts/PromptLibrary.tsx:619 -->
- Palette Enter is a keydown handler. Click the `[role="option"]` row instead.
- A body that is only whitespace leaves the row disabled (`This prompt has no text yet`). <!-- src: src/features/prompts/PromptPalette.tsx:106 -->
