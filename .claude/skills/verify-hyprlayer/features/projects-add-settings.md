# Add, configure and remove a project

A user adds a project from the sidebar or Home, typing a path or browsing for one. A non-git folder is added as a plain folder project: no git, no thoughts. Project settings shows the storage backend, the scripts card and a backend health check, plus worktree-file and branch-type cards for git projects. Removing a project drops it from the sidebar. It can also delete the directory, and it refuses while the project still has active workstreams.

## Sub-features

- `proj-add-folder` adds a non-git folder by typed path (`Add folder`).
- `proj-add-cancel` closes the Add project dialog without adding anything.
- `proj-add-bare` blocks a bare repository with an inline error.
- `proj-settings-open` opens Project settings from the project's context menu.
- `proj-health` runs Backend health: `pass` for a present folder, `fail` with a remedy for a missing one.
- `proj-missing-dot` shows the sidebar's missing-on-disk dot.
- `proj-remove-blocked` refuses removal while active workstreams exist.
- `proj-remove` removes the entry, optionally deleting the directory.

## How to get to it (user POV)

- Sidebar `+` next to "Projects" (`aria-label` `Add project`); it opens in clone mode. <!-- src: src/routes/Root.tsx:1177 -->
- Home: `Add repo`. With zero projects, the first-run blank's `Open a project` opens in disk mode. <!-- src: src/routes/Dashboard.tsx:292 --> <!-- src: src/routes/Dashboard.tsx:194 -->
- Right-click a project row: `Project settings…` or `Remove project…`. <!-- src: src/routes/Root.tsx:1700 --> <!-- src: src/routes/Root.tsx:1709 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance, doctor passed. No model turns. `proj-*` writes only the profile SQLite, the run dir and the deleted folder. Never point this recipe at a git folder: a git import runs `thoughts init` and writes the shared CLI config (see Gotchas).
- Before driving, create an unimported folder in Bash: `mkdir -p <run-dir>/seed/verify-add && echo hi > <run-dir>/seed/verify-add/README.md`.
- `verify-demo` is seeded (SKILL.md, Launch), for the settings and remove steps.

- **Open (`proj-add-cancel`, `proj-add-folder`).** Click `[aria-label="Add project"]`. <!-- src: src/routes/Root.tsx:1180 --> A `[role="dialog"]` opens; its `h2` reads `Add project` and `Step 1 of 2 · Repository`. <!-- src: src/features/projects/AddProjectDialog.tsx:405 --> <!-- src: src/components/ui.tsx:673 -->
- **Switch to disk.** Click the `Open from disk` tab (`strategy: "text"`). <!-- src: src/features/projects/RepoSourcePicker.tsx:34 --> The field's placeholder is `/path/to/repo-or-folder`. <!-- src: src/features/projects/AddProjectDialog.tsx:449 -->
- **Type the path.** `webview_keyboard type` into `input[placeholder="/path/to/repo-or-folder"]` with `<run-dir>/seed/verify-add`. After the ~300ms debounce and `inspect_repo_path`, the footer reads "Added as a plain folder. No git, no thoughts.", the subtitle becomes `Local folder`, and a `Set up a git repository` switch appears, off. <!-- src: src/features/projects/AddProjectDialog.tsx:135 --> <!-- src: src/features/projects/AddProjectDialog.tsx:454 --> <!-- src: src/features/projects/AddProjectDialog.tsx:469 --> Leave the switch off; turning it on gives a git init and `thoughts init`.
- **Cancel.** Click `Cancel` (`strategy: "text"`). No dialog remains, and `invoke('get_bootstrap')` shows no project with `path` `<run-dir>/seed/verify-add`. <!-- src: src/features/projects/AddProjectDialog.tsx:773 -->
- **Add.** Reopen, switch to disk, type the path again, then click the submit button reading `Add folder` (`form button[type="submit"]`). <!-- src: src/features/projects/AddProjectDialog.tsx:283 --> <!-- src: src/features/projects/AddProjectDialog.tsx:776 --> The dialog closes and the app navigates to the project: the sidebar lists `verify-add`, and the board shows the folder blank `New session`. <!-- src: src/routes/Root.tsx:1718 --> <!-- src: src/features/board/NoWorktreeBlank.tsx:53 -->
- **Read back.** `invoke('get_bootstrap')`: `projects` has a `path` ending `/seed/verify-add` (canonical, see Gotchas), `mappedName` `verify-add`, `existsOnDisk: true`. <!-- src: src/lib/bindings.ts:8 --> `ls -A` on the folder still shows only `README.md`. Save both.
- **Settings (`proj-settings-open`).** Right-click the project row (selector `[data-project-path="<projectPath>"] > div:first-child`; each workstream row under it is now a `div` too), then click `Project settings…` (`strategy: "text"`). <!-- src: src/routes/Root.tsx:1307 --> <!-- src: src/routes/Root.tsx:1316 --> <!-- src: src/routes/Root.tsx:1700 --> The dialog `h2` reads `Project settings: verify-demo` with the path below it. Storage shows the badge `local folder` and "Not a git repository, so no branches, worktrees or thoughts." There are no `Files copied into new worktrees` or `Branch types` sections. <!-- src: src/features/projects/ProjectSettingsDialog.tsx:162 --> <!-- src: src/features/projects/ProjectSettingsDialog.tsx:194 --> <!-- src: src/features/projects/ProjectSettingsDialog.tsx:216 -->
- **Health (`proj-health`).** Under `Backend health`, the row has a `pass` badge, the label `Project folder` and the path. <!-- src: src/features/projects/ProjectSettingsDialog.tsx:224 --> <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:1373 --> Read it back with `invoke('run_backend_health', { path })` <!-- src: src/lib/bindings.ts:23 -->, which returns `checks[0]` `{ id: "folder.exists", status: "pass" }`.
- **Missing folder (`proj-health`, `proj-missing-dot`).** Open Project settings for `verify-add` first (a missing folder may fail `get_project` and show a page error instead). Then, in Bash, `mv <run-dir>/seed/verify-add <run-dir>/seed/verify-add.gone` and click `Run checks`. <!-- src: src/features/projects/ProjectSettingsDialog.tsx:186 --> The row turns `fail`, and the remedy reads "The folder was moved or deleted. Remove the project and re-add it." <!-- src: src/features/projects/ProjectSettingsDialog.tsx:225 --> <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:1377 --> After a reload (bootstrap is cached), the sidebar row shows the dot `span[title="Repository folder is missing on disk"]`. <!-- src: src/routes/Root.tsx:1391 --> Move the folder back.
- **Remove blocked (`proj-remove-blocked`).** With an active workstream in `verify-add` (seed one with `invoke('create_workstream', { projectPath, title: 'Blocker', base: null, mode: 'folder', branch: null, branchPrefix: null })`, then reload) <!-- src: src/lib/bindings.ts:46 -->, right-click the row, choose `Remove project…`, then click `Remove`. <!-- src: src/routes/Root.tsx:1709 --> <!-- src: src/features/projects/RemoveProjectDialog.tsx:51 --> An error note shows the remedy "Archive its workstreams first, then remove the project." and the project stays. Archive `Blocker` (`invoke('archive_workstream', { id, skipScripts: false })`) before the next step. <!-- src: src/lib/bindings.ts:64 --> <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:1189 --> <!-- src: crates/hyprlayer-core/src/error.rs:702 -->
- **Remove (`proj-remove`).** On `verify-add` (no active workstreams), open `Remove project…`. A `[role="dialog"][aria-label="Remove project?"]` reads "verify-add will be removed from the sidebar." <!-- src: src/features/projects/RemoveProjectDialog.tsx:50 --> <!-- src: src/components/ui.tsx:744 --> Optionally tick `Also delete the directory from disk`; the button changes to `Remove & delete`. <!-- src: src/features/projects/RemoveProjectDialog.tsx:73 --> Confirm. The row is gone. `get_bootstrap` lacks the path, and with delete ticked `ls` on the folder fails. The archived `Blocker` from the previous step is gone too: `list_archived_workstreams` no longer lists it.
- **Bare (`proj-add-bare`).** Run `git init --bare <run-dir>/seed/bare.git` and type that path. The footer reads "This is a bare repository. Choose a normal working copy instead.", and `Next` is disabled. <!-- src: src/features/projects/AddProjectDialog.tsx:459 --> <!-- src: src/features/projects/AddProjectDialog.tsx:273 --> Cancel; nothing is written.
- **Proof.** Save screenshots of the dialog in each state, the settings dialog with pass and fail, the remove confirm, plus `get_bootstrap` and `run_backend_health` JSON and the `ls` output.

## Gotchas

- `Browse` opens the native folder picker, which the bridge can't drive. Type the path instead. <!-- src: src/components/PathField.tsx:41 -->
- The sidebar `+` opens in clone mode, so click `Open from disk` first. `project.open` (⌘O) opens in disk mode. <!-- src: src/routes/Root.tsx:766 -->
- The submit button stays disabled until `inspect_repo_path` answers, about 300ms after the last keystroke. <!-- src: src/features/projects/AddProjectDialog.tsx:270 -->
- Turning on `Set up a git repository`, any git or clone path, and `Create new` all run `thoughts init` or create profiles. <!-- src: src/features/projects/AddProjectDialog.tsx:342 --> <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:595 --> These write `~/Library/Application Support/hyprlayer/config.json`, which every profile shares. Don't drive them in an isolated run.
- Dragging to reorder projects uses `onPointerDown`, which bridge clicks don't fire. <!-- src: src/routes/Root.tsx:1325 -->
- `remove_project` counts only active workstreams, so archived ones don't block it. Removal purges the project's archives too. <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:1224 --> <!-- src: crates/hyprlayer-core/src/runtime/workstreams.rs:2336 -->
- A project row's name also appears in the header breadcrumb once it's selected. Target the row through `[data-project-path=…]`, not a text match. <!-- src: src/routes/Root.tsx:1307 -->
- Project paths are canonicalised (`/var/folders/…` becomes `/private/var/folders/…`), so take `projectPath` from `get_bootstrap().projects[].path`, not the `$TMPDIR` spelling. This applies to `[data-project-path=…]` selectors and every IPC arg. <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:745 --> <!-- src: src/routes/Root.tsx:1307 -->
