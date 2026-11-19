# Project pipeline board

Opening a project with no workstream selected shows its pipeline. With no workstreams, it is a blank state with one call to action. Otherwise a `Pipeline view` switch picks one of three views. `Board` has five columns: Planning, Implementing, Waiting, Review, Merged. `Triage` groups workstreams by live activity, and `Table` is one row each. A workstream's column follows live activity first, then its PR, then its stage. Clicking a card opens the workstream. The header also has a project picker and `New worktree` (or `New session` in a folder project).

## Sub-features

- `board-blank` shows the empty project's blank state (`first session.` / `first branch.`) and its `New session`/`New worktree` call to action.
- `board-columns` places each workstream in a column by stage, and the column header shows its count.
- `board-views` switches between `Triage`, `Board` and `Table`, and the choice persists.
- `board-open` opens a workstream by clicking its card.
- `board-project-picker` switches projects from the header picker.
- `board-new` opens New workstream from the header button.

## How to get to it (user POV)

- Click a project row in the sidebar (no workstream selected), or the project name in a workstream's header crumb. <!-- src: src/routes/Root.tsx:1339 --> <!-- src: src/routes/Root.tsx:910 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance seeded with `verify-demo`. For the picker, seed a second non-git folder by IPC (the native picker can't be driven): `mkdir -p <run-dir>/seed/verify-two`, then `invoke('import_local_folder', { path: '<run-dir>/seed/verify-two', directoryName: null })`. No model turns. <!-- src: src/lib/bindings.ts:12 -->
- For `board-columns`, seed three folder workstreams (`create_workstream` with `mode: 'folder'`, see `workstreams-lifecycle.md`): `Plan card`, `Impl card`, `Review card`. Move two of them by IPC: `invoke('set_workstream_stage', { id: <Impl id>, stage: 'implement' })` <!-- src: src/lib/bindings.ts:58 --> and `{ id: <Review id>, stage: 'validate' }`. Then `location.reload()`. This is seed, not the action under test.

- **Blank (`board-blank`).** Before seeding workstreams, click the `verify-demo` row link (`[data-project-path="<projectPath>"] a[href]`, with `<projectPath>` from `get_bootstrap`). The blank title ends `first session.` (a `<br>` splits it from `Start the`) above "Every session is its own conversation in this folder.", with a `New session` button. <!-- src: src/features/board/NoWorktreeBlank.tsx:32 --> <!-- src: src/features/board/NoWorktreeBlank.tsx:46 --> <!-- src: src/features/board/NoWorktreeBlank.tsx:53 --> Clicking it opens `New workstream`. <!-- src: src/routes/ProjectPage.tsx:1293 --> A git project reads `first branch.` with `New worktree`.
- **Board (`board-columns`).** After seeding, click `Board` in `[aria-label="Pipeline view"]` (`strategy: "text"`). <!-- src: src/features/board/ProjectPipeline.tsx:88 --> <!-- src: src/features/board/pipeline.ts:22 --> Each column is a `section[aria-label="<label>"]` whose header shows the count. <!-- src: src/features/board/PipelineBoard.tsx:47 --> <!-- src: src/features/board/PipelineBoard.tsx:55 --> Expect the card `[role="button"][aria-label="Plan card"]` inside `section[aria-label="Planning"]` (with `Verify map seed` too, if workstream-create ran first), `Impl card` in `Implementing`, `Review card` in `Review`, and counts 0 for `Waiting` and `Merged`. <!-- src: src/features/board/PipelineCard.tsx:84 --> <!-- src: src/features/board/pipeline.ts:41 --> <!-- src: src/features/board/pipeline.ts:43 --> <!-- src: src/features/board/pipeline.ts:49 --> A folder project's header has `New session` and no `N branches active` footer. <!-- src: src/features/board/ProjectPipeline.tsx:96 --> <!-- src: src/features/board/ProjectPipeline.tsx:114 --> Read back with `list_workstreams` `{ projectPath, includeArchived: false }`: each `item.stage` matches (`general`, `implement`, `validate`).
- **Views (`board-views`).** Click `Table`. A table with headers `Workstream … Pipeline … Stage …` lists one `[role="button"]` row per workstream, and its Pipeline cell shows the column label. <!-- src: src/features/board/PipelineTable.tsx:36 --> <!-- src: src/features/board/PipelineTable.tsx:38 --> <!-- src: src/features/board/PipelineTable.tsx:68 --> Click `Triage`. With no live sessions, all three sit under `section[aria-label="Idle"]`. <!-- src: src/features/board/PipelineTriage.tsx:33 --> <!-- src: src/features/board/pipeline.ts:80 --> Reload: the last view is restored (localStorage key `hyprlayer:pipeline-view`, host-scoped). <!-- src: src/features/board/pipelineViewStore.ts:18 --> Set it back to `Board`.
- **Open (`board-open`).** Click `[role="button"][aria-label="Impl card"]`. The URL search gains `item=<id>`, and the workstream's Agent tab replaces the board. <!-- src: src/features/board/PipelineCard.tsx:37 --> <!-- src: src/routes/ProjectPage.tsx:393 --> Go back with the header crumb `header a[href^="/projects/"]` (`verify-demo`). `Back to <project>` exists only on the Library route. <!-- src: src/routes/Root.tsx:910 --> <!-- src: src/routes/Root.tsx:942 -->
- **Picker (`board-project-picker`).** Click the header button with `aria-haspopup="menu"` that shows `verify-demo`. A popover `[aria-label="Switch project"]` lists `[role="menuitemradio"]` rows, and the current one has `aria-checked="true"`. <!-- src: src/features/board/ProjectPipeline.tsx:136 --> <!-- src: src/features/board/ProjectPipeline.tsx:155 --> <!-- src: src/features/board/ProjectPipeline.tsx:162 --> Click the other project. The board shows that project and the URL path changes.
- **Proof.** Save screenshots of the blank, the board with three columns filled, Table and Triage, plus the `list_workstreams` JSON.

## Gotchas

- The view persists per Vite origin, so a reused port can open in Table or Triage. Click `Board` before asserting columns. <!-- src: src/features/board/pipelineViewStore.ts:18 -->
- Project paths are canonicalised (`/var/folders/…` becomes `/private/var/folders/…`), so take `projectPath` from `get_bootstrap().projects[].path`, not the `$TMPDIR` spelling. This applies to `[data-project-path=…]` selectors and every IPC arg. <!-- src: crates/hyprlayer-core/src/runtime/projects.rs:744 --> <!-- src: src/routes/Root.tsx:1287 -->
- `set_workstream_stage` only moves forward along research → … → revise. `general` has no rank, so setting it is a no-op, and any ranked stage beats it. <!-- src: crates/hyprlayer-core/src/pipeline.rs:19 --> <!-- src: crates/hyprlayer-core/src/store/workstreams.rs:248 -->
- A `general` workstream with uncommitted or ahead-of-base changes lands in Implementing, not Planning. Untracked files don't count. <!-- src: src/features/board/pipeline.ts:30 -->
- `Waiting` needs a live `needs-you` session, and `Merged` needs an observed merged PR or a same-day merged archive. Neither can be seeded without a real agent or PR. <!-- src: src/features/board/pipeline.ts:35 --> <!-- src: src/features/board/pipeline.ts:69 -->
- In a folder project, cards have no context menu, so `Attach PR…` and `Run until met…` are git-only. <!-- src: src/routes/ProjectPage.tsx:1295 -->
- A goal running on a workstream adds a `[data-testid="board-goal-badge"]` reading `goal N/M`. <!-- src: src/features/board/PipelineCard.tsx:99 -->
