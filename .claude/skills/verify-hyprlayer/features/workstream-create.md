# Create a workstream

A user adds a workstream to a project from the sidebar. In a folder project it runs in the project folder itself, with no branch or worktree. It then appears under the project and opens with an empty Agent tab and the folder's files in the Files pane.

## Sub-features

- `ws-open` opens the New workstream dialog from each sidebar entry point.
- `ws-create-folder` creates a folder-mode workstream with a title.
- `ws-cancel` closes the dialog without creating anything.

## How to get to it (user POV)

- Right-click the project row in the sidebar and choose `New workstream…`.
- Hover the project row and click its `+` (`aria-label` `New workstream in <project>`).

## Driving it with the MCP bridge

Preconditions:

- An isolated instance seeded with `verify-demo` (SKILL.md, Launch). Doctor passed.
- `verify-demo` has no workstreams yet.

- **Open from the context menu.** Right-click the project row. Run `webview_interact` `right-click` with selector `verify-demo` and `strategy: "text"` (add `nth: 1` once the project is selected, see Gotchas), then `click` with selector `New workstream…` and `strategy: "text"`. A dialog titled `New workstream` appears, reading "Runs in the project folder. No branch, no worktree."
- **Enter a title.** Run `webview_keyboard` `type` with selector `input[placeholder="What are you working on?"]` and text `Verify map seed`. The `Create` button enables.
- **Create.** Run `webview_interact` `click` on `form button[type="submit"]`, then `webview_wait_for` `text` `Verify map seed`. The workstream sits under `verify-demo` with the folder icon, the header reads `verify-demo > Verify map seed`, and Files lists `README.md`.
- **Read back.** Run `webview_execute_js` with `window.__TAURI__.core.invoke('list_workstreams', { projectPath: '<SEED>', includeArchived: false })`. Exactly one row, whose `item` has `title` `Verify map seed`, `worktreeMode` `folder`, and `worktreePath` equal to `projectPath`. `ls -A <SEED>` still shows only `README.md`.
- **Cancel.** Run this before Create, so the list is still empty: open the dialog, type `Discard me`, and click `Cancel` (`strategy: "text"`). No `[role="dialog"]` remains, and the list RPC returns zero rows.
- **Proof.** Save screenshots of the dialog and the created state, plus the RPC JSON and the folder listing.

## Gotchas

- The `+` link is `hidden` until the row is hovered. A selector click on it fails, so use the context menu or a coordinate hover first.
- Enter in the title field doesn't submit under the bridge. Click `Create`.
- Once a project is selected, its name also appears in the header breadcrumb. A text selector then matches twice: `nth: 0` is the breadcrumb, and right-clicking it opens no menu. Use `nth: 1` for the sidebar row.
- A git project shows Branch and Type of work fields and creates a worktree under the worktree root. That's a different recipe, and it writes the shared CLI config through `thoughts init`.
