# Changes tab

The right sidebar's lower panel shows a git workstream's working tree. The `changes` tab lists Unstaged and Staged files, as a list or a tree, with per-row and bulk stage/unstage. A row's context menu offers `Open diff` and `Discard changes…`. Clicking a row opens its diff as a viewer tab in the main pane, with a `working`/`staged` badge and a unified or side-by-side layout. The `commits` tab lists the commits the branch adds on top of its base; expanding one lists its files, and each file opens that commit's diff. The `files` tab browses the worktree. Above the tabs, the Git panel holds the branch → target row and the PR area (see [ship-pr.md](./ship-pr.md)).

## Sub-features

- `ch-sidebar` shows or hides the sidebar (`Show changes sidebar`/`Hide changes sidebar`). With a plan or goal, it picks `Changes` in the `Right panel` switch. <!-- src: src/routes/Root.tsx:1038 --> <!-- src: src/routes/ProjectPage.tsx:1764 -->
- `ch-list` covers the `Unstaged N` and `Staged N` sections, the `N` badge, `Nothing here.`, the `No changes yet` card and the list/tree toggle. <!-- src: src/features/workstreams/ChangesPanel.tsx:231 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:244 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:660 -->
- `ch-stage` covers the row `Stage`/`Unstage` buttons and `Stage all`/`Unstage all`. <!-- src: src/features/workstreams/ChangesPanel.tsx:86 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:688 -->
- `ch-menu` covers the row context menu: `Stage`/`Unstage`, `Open diff` and `Discard changes…`, which opens the confirm dialog. <!-- src: src/features/workstreams/ChangesPanel.tsx:721 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:737 -->
- `ch-diff` covers the diff viewer tab: the heading (`old → new` for renames), the badge (`working`, `staged` or a short sha) and `Unified`/`Side by side`. <!-- src: src/features/workstreams/DiffTab.tsx:26 --> <!-- src: src/features/workstreams/DiffTab.tsx:42 --> <!-- src: src/features/workstreams/DiffTab.tsx:55 -->
- `ch-commits` covers the commits tab: `No commits yet`, expanding a commit row, its files, `No files changed.` and the commit diff tab `<file> @ <sha7>`. <!-- src: src/features/workstreams/ChangesPanel.tsx:635 --> <!-- src: src/features/workstreams/CommitsPanel.tsx:53 --> <!-- src: src/routes/ProjectPage.tsx:517 -->
- `ch-files` covers the files tab: the lazy folder tree, where clicking a file opens a file viewer tab. <!-- src: src/features/workstreams/ChangesPanel.tsx:504 -->
- `ch-refresh` covers `Refresh`, which refetches the active tab. <!-- src: src/features/workstreams/ChangesPanel.tsx:594 -->

## How to get to it (user POV)

- Open a git workstream. The sidebar is on the right; the top-bar panels icon toggles it. <!-- src: src/routes/Root.tsx:1041 -->
- If the session has a plan or goal, choose `Changes` in the segmented control at the top of the sidebar. <!-- src: src/routes/ProjectPage.tsx:1756 -->
- Choose `changes`, `commits` or `files` in the panel's header. <!-- src: src/features/workstreams/ChangesPanel.tsx:531 -->

## Driving it with the MCP bridge

Preconditions:

- **Seed.** Use the `review_lifecycle` sandbox from [review-workspace.md](./review-workspace.md). It's isolated, with no model turns and no GitHub. The baseline `verify-demo` is a folder project: it shows only `files`, with no Git panel. <!-- src: src/features/workstreams/ChangesPanel.tsx:531 --> <!-- src: src/routes/ProjectPage.tsx:1786 -->
- **Read the state first.** `window.__TAURI__.core.invoke('worktree_git_status', { id })` gives `{ staged, unstaged }` and is the expected row list. <!-- src: src/lib/bindings.ts:71 --> Also save `git -C <worktreePath> status --porcelain` and `git -C <worktreePath> diff` to the evidence dir.
- **Branch commits.** The sandbox's branch carries `Edit` and `Rename` on top of `main`. <!-- src: crates/hyprlayer-e2e/tests/review_lifecycle.rs:171 -->

Steps:

- **Show the sidebar.** If `[aria-label="Show changes sidebar"]` exists, click it. <!-- src: src/routes/Root.tsx:1038 --> If `[aria-label="Right panel"]` exists, click its `Changes` radio. <!-- src: src/routes/ProjectPage.tsx:1759 --> <!-- src: src/routes/ProjectPage.tsx:1764 -->
- **List.** Section labels read `Unstaged N` and `Staged N` <!-- src: src/features/workstreams/ChangesPanel.tsx:231 -->, and the header badge equals their sum. <!-- src: src/features/workstreams/ChangesPanel.tsx:571 --> Each row is a `div[role="button"]` holding a status glyph and a `span[title=<path>]`. <!-- src: src/features/workstreams/ChangesPanel.tsx:68 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:79 --> The rows match `worktree_git_status` one for one.
- **Tree.** Click `tree` (text). Rows nest under folder buttons, and clicking a folder collapses it. Click `list` to go back. <!-- src: src/features/workstreams/ChangesPanel.tsx:678 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:169 -->
- **Stage one.** Find the target row by its `span[title]`, then click the `button[aria-label="Stage"]` inside it. <!-- src: src/features/workstreams/ChangesPanel.tsx:86 --> The row moves to Staged. Read back: the path is in `staged` and `git status --porcelain` shows index status. Restore it with that row's `Unstage`.
- **Bulk.** `Stage all` and `Unstage all` act on the whole section. <!-- src: src/features/workstreams/ChangesPanel.tsx:688 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:699 --> Afterwards, restore the index to the saved porcelain, using the UI or `git restore --staged` for paths that started unstaged.
- **Open a diff.** Click a modified row, such as `src/edit.rs`. A viewer tab titled `edit.rs` opens. <!-- src: src/routes/ProjectPage.tsx:516 --> Its header shows the path and a `working` badge, or `staged` from the Staged section. <!-- src: src/features/workstreams/DiffTab.tsx:42 --> Click `Side by side`, then `Unified`. Both are disabled for an added or deleted file. <!-- src: src/features/workstreams/DiffTab.tsx:49 --> <!-- src: src/features/workstreams/DiffTab.tsx:55 -->
- **Context menu.** Run `webview_interact` `right-click` on the row. <!-- src: src/features/workstreams/ChangesPanel.tsx:64 --> The `[role="menu"]` holds `Stage` (or `Unstage`), `Open diff`, and `Discard changes…` for unstaged rows only. <!-- src: src/features/workstreams/ChangesPanel.tsx:718 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:721 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:725 -->
- **Discard.** Only discard a change the run made:
  1. Append a line to `README.md` and click `Refresh`. <!-- src: src/features/workstreams/ChangesPanel.tsx:609 -->
  2. Right-click `README.md` → `Discard changes…`. A dialog titled `Discard changes?` names `README.md`. <!-- src: src/features/workstreams/ChangesPanel.tsx:737 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:749 -->
  3. Click `Discard`. <!-- src: src/features/workstreams/ChangesPanel.tsx:738 --> `git diff README.md` is now empty, and the row is gone.
- **Commits.** Click `commits` (text). Each row shows a short sha and the subject, titled with the subject. <!-- src: src/features/workstreams/CommitsPanel.tsx:33 --> <!-- src: src/features/workstreams/CommitsPanel.tsx:34 --> Click `Edit` to expand it: its files list with glyphs. <!-- src: src/features/workstreams/CommitsPanel.tsx:59 --> Click `src/edit.rs` and a tab `edit.rs @ <sha7>` opens, whose badge is the short sha in mono. <!-- src: src/routes/ProjectPage.tsx:516 --> <!-- src: src/features/workstreams/DiffTab.tsx:42 --> Read back: `window.__TAURI__.core.invoke('list_workstream_commits', { id })` matches the rows, and `worktree_commit_files` with `{ id, sha }` matches the expanded files. <!-- src: src/lib/bindings.ts:84 -->
- **Files.** Click `files` (text). Folders expand on click, and clicking a file opens a viewer tab named after it. <!-- src: src/features/workstreams/ChangesPanel.tsx:492 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:464 -->
- **Proof.** Save screenshots of list, tree, both diff layouts, the context menu, the discard dialog, an expanded commit and its diff tab, plus the porcelain and RPC JSON before and after each mutation.

## Gotchas

- After `Chat` closes a review, the right aside comes back on `Plan`, which hides the Git panel and the changes list. Click `Changes` in the aside first.
- Tab and toggle buttons render lowercase text (`changes`, `commits`, `files`, `list`, `tree`) styled `capitalize`. A text selector must use the lowercase string. <!-- src: src/features/workstreams/ChangesPanel.tsx:562 --> <!-- src: src/features/workstreams/ChangesPanel.tsx:566 -->
- Every row has the same `Stage`/`Unstage` label, so scope the selector to the row. The button is `opacity-0` until hover, but it's in the DOM and clickable. <!-- src: src/features/workstreams/ChangesPanel.tsx:91 -->
- `Discard changes…` is irreversible, and the dialog says so. Never use it on a seed file or a real project's change. <!-- src: src/features/workstreams/ChangesPanel.tsx:753 -->
- The `[aria-label="Right panel"]` switch only exists while a plan or goal is available. Otherwise the panel shows directly. <!-- src: src/routes/ProjectPage.tsx:1756 -->
- The commits tab fetches only while it's selected. If a file you wrote by hand doesn't show, click `Refresh` before judging. <!-- src: src/features/workstreams/ChangesPanel.tsx:542 -->
- `Review changes` in this header opens the review workspace ([review-workspace.md](./review-workspace.md)); it isn't a diff of the Changes list. <!-- src: src/features/workstreams/ChangesPanel.tsx:580 -->
- The sandbox has no `origin`. `Fetch remote` and `Refresh branches` in the Git panel fail or list `No remote branches`. That's the fixture, not a bug. <!-- src: src/features/workstreams/GitPanel.tsx:97 -->
