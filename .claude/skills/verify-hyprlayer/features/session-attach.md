# Attach files and docs to a message

A user attaches context to the next message. Typing `@` in the composer opens a `Files` list from the workstream's folder, and picking one adds a file chip above the textarea. The `+` (`Attach context`) menu offers `Add files…` and `Add screenshots…`, plus `Attach from library` in git projects. Dropping files on the chat or pasting an image also attaches. Chips can be removed with their `×`. On send, the chips move onto the user row in the transcript, and the paths go to the agent under a `Relevant documents:` list.

## Sub-features

- `attach-at-file` picks a workstream file through `@` autocomplete.
- `attach-remove` removes a chip before sending.
- `attach-send` sends the attachments with the message, shown as chips on the user row.
- `attach-menu` opens the `Attach context` menu.
- `attach-native` covers `Add files…`, `Add screenshots…`, drag-and-drop and clipboard image paste.
- `attach-library` attaches thoughts docs from the library (git projects only).

## How to get to it (user POV)

- Type `@` then part of a file name in the composer, and click a row under `Files`. <!-- src: src/features/session/ComposerAutocomplete.tsx:46 -->
- Click `+` (`Attach context`) right of the composer. <!-- src: src/features/session/SessionChat.tsx:243 -->
- Drag files onto the chat ("Drop to attach"), or paste an image. <!-- src: src/features/session/SessionChat.tsx:1387 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance with the `Verify map seed` folder workstream ([workstream-create.md](./workstream-create.md)). Its folder holds `README.md`.
- Picking and removing chips is **free**. `attach-send` spends one turn: $0 on the free OpenCode path from [session-send.md](./session-send.md).

- **Trigger.** Run `webview_keyboard` `type` on `[data-pane-content] textarea` with `Summarize @READ`. Within ~100ms (search debounce), a dropdown headed `Files` lists `README.md`. <!-- src: src/features/session/ComposerAutocomplete.tsx:36 --> <!-- src: src/features/session/ComposerAutocomplete.tsx:356 -->
- **Pick.** Click the row with `README.md` (`strategy: "text"`, `nth: 0`; `nth: 1` is the Files pane). The `@READ` token leaves the draft, and a chip `README.md` with `button[aria-label="Remove README.md"]` sits above the textarea. The tab store's shell has `attachments: [{ kind: 'file', name: 'README.md', path: '<SEED>/README.md' }]`. <!-- src: src/features/session/ComposerAutocomplete.tsx:246 --> <!-- src: src/features/session/SessionChat.tsx:1025 --> <!-- src: src/features/session/AttachmentCard.tsx:79 -->
- **Remove.** Click `button[aria-label="Remove README.md"]`. The chip goes away, and `attachments` is `[]`. Pick it again for the send step.
- **Send.** Click `button[aria-label="Start"]`. The user row shows `Summarize` with a `README.md` chip. `get_session_log` (`{ sessionId }`) should have a first `userMessage` whose text ends with `Relevant documents:` and `- <SEED>/README.md`, and the composer has no chips left. <!-- src: crates/hyprlayer-core/src/pipeline.rs:566 --> <!-- src: crates/hyprlayer-core/src/runtime/sessions/service.rs:1289 --> <!-- src: src/features/session/SessionChat.tsx:1125 -->
- **Menu.** Click `button[aria-label="Attach context"]`. The popover lists `Add files…` and `Add screenshots…`. In a folder project like `verify-demo`, it has no `Attach from library`. <!-- src: src/features/session/SessionChat.tsx:233 --> <!-- src: src/features/session/SessionChat.tsx:234 --> <!-- src: src/routes/ProjectPage.tsx:1005 -->
- **Native paths: report as unreachable.** `Add files…`/`Add screenshots…` open the OS file dialog, and drop arrives as a native Tauri drag-drop event. The bridge can drive neither, so record the step and the unmet precondition. <!-- src: src/routes/ProjectPage.tsx:938 --> <!-- src: src/features/session/SessionChat.tsx:711 -->
- **Proof.** Screenshot the open `Files` dropdown, the chip, and the sent user row. Save the tab-store `attachments` before and after, and the log's first `userMessage`.

## Gotchas

- Suggestion rows act on `mousedown`, not `click`. A bridge selector click sends `mousedown`, so it works, but a handler-only `el.click()` from JS does nothing. <!-- src: src/features/session/ComposerAutocomplete.tsx:372 -->
- The trigger reads the caret position. If typing doesn't move it (no dropdown appears), click into the textarea and type the `@` token again.
- While the dropdown is open, Enter and Tab pick the active row instead of sending. <!-- src: src/features/session/ComposerAutocomplete.tsx:296 -->
- `@` paths are relative to the workstream's `worktreePath`. The chip holds the absolute path, and a picked file is not copied. Dropped or pasted images are copied into the workstream via `import_attachment`/`save_attachment`. <!-- src: src/features/session/attachments.ts:153 -->
- Unstarted tabs also show suggested doc chips (`WorkstreamDocChips`), which are sent as attachments too. Don't count them as your pick. <!-- src: src/features/session/SessionChat.tsx:1540 --> <!-- src: src/features/session/SessionChat.tsx:1105 -->
- A file chip's name button is disabled when the server is remote, since the path can't be opened locally. <!-- src: src/features/session/AttachmentCard.tsx:69 -->
