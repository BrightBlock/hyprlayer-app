# Review workspace

`Review changes` replaces the chat with a full-width review of the workstream branch against its base. The header shows the review status, `N / M viewed`, a progress bar and `branch → base` with `+adds −dels`. A file tree on the left groups files by folder, or by plan task when the implement session tracks one. The diff pane shows one file at a time, with prev/next, a `Viewed` checkbox, a layout toggle, and empty states for binary and oversized files. The agent's notes and the reviewer's threads render inline in the diff. Notes, threads and the Submit popover are in [review-threads.md](./review-threads.md).

## Sub-features

- `rv-open` opens the workspace from the Changes header, the `⌘⇧R` chord or the chat's `Ready for review` card. <!-- src: src/features/workstreams/ChangesPanel.tsx:582 --> <!-- src: src/lib/keybindings/commands.ts:262 --> <!-- src: src/features/review/ReadyForReviewCard.tsx:229 -->
- `rv-close` goes back to the chat with `‹ Chat`; the selected file survives the round trip. <!-- src: src/features/review/ReviewHeader.tsx:124 --> <!-- src: src/features/review/reviewStore.ts:9 -->
- `rv-header` shows the status chip, the viewed count and progress bar, `Mark all viewed`, the base line and `base not found · diffing against HEAD`. <!-- src: src/features/review/ReviewHeader.tsx:141 --> <!-- src: src/features/review/ReviewHeader.tsx:213 -->
- `rv-tree` covers the file tree: the filter, `Group by`, the viewed ring per row, the open-items badge and the `Changed since your review` group. <!-- src: src/features/review/FileTree.tsx:253 --> <!-- src: src/features/review/FileTree.tsx:259 --> <!-- src: src/features/review/FileTree.tsx:238 -->
- `rv-diff` covers the diff pane: `i of N`, Previous/Next, `Viewed`, `Diff layout`, `Binary file`, `File too large to diff` and `Mark viewed, next: <file>`. <!-- src: src/features/review/DiffPane.tsx:288 --> <!-- src: src/features/review/DiffPane.tsx:320 --> <!-- src: src/features/review/DiffPane.tsx:374 -->
- `rv-viewed` marks a file viewed or not. The viewed state is stored per content oid, so a changed file drops back to unviewed. <!-- src: src/features/review/ReviewWorkspace.tsx:113 -->
- `rv-plan-tab` shows the `Plan d/t` tab, only when the implement session has a plan. <!-- src: src/features/review/ReviewWorkspace.tsx:157 --> <!-- src: src/features/review/ReviewWorkspace.tsx:163 -->

## How to get to it (user POV)

- In the right sidebar's Changes header, click the review icon (`aria-label` `Review changes`). It's hidden for folder projects. <!-- src: src/features/workstreams/ChangesPanel.tsx:581 -->
- Press `⌘⇧R` (`ws.review`) while a git workstream is open. <!-- src: src/routes/ProjectPage.tsx:1431 -->
- In an implement chat, click `Review changes` on the `Ready for review` card. <!-- src: src/features/review/ReadyForReviewCard.tsx:198 -->
- In the Plan tab's Review step, click `Review changes` or `Review N updated files` ([review-lifecycle.md](./review-lifecycle.md)). <!-- src: src/features/review/ReviewChecklist.tsx:137 -->

## Driving it with the MCP bridge

Preconditions:

- **The baseline `verify-demo` seed can't reach this.** A folder workstream isn't reviewable, and the review entry and Git panel are hidden. <!-- src: src/routes/ProjectPage.tsx:901 -->
- **Seed with the kept `review_lifecycle` E2E sandbox.** It's a git project `demo` with workstream `Review work`. Its review holds 7 files: a modify, a rename, a deletion, an unstaged `mv`, an untracked file, a binary blob and a 3 MiB file. It also has agent notes (one `outside_plan_scope`), sent threads on both sides, three rounds, and status `approved`. <!-- src: crates/hyprlayer-e2e/tests/review_lifecycle.rs:168 -->
- **It's isolated and costs nothing to run.** The harness is a stub `claude` pinned in the sandbox's own settings, and `gh` is pinned absent, so there are no model turns and no GitHub calls. <!-- src: crates/hyprlayer-e2e/src/sandbox.rs:12 -->
- **Launch it with `verify.sh launch attach`.** The app attaches to the sandbox's server through `hosts.json` `localProfile`, which is read once at startup. <!-- src: src-tauri/src/hosts.rs:135 -->
  1. From the repo root, run `cargo build -p hyprlayer-server`. Then run `HYPRLAYER_E2E_KEEP=1 HYPRLAYER_E2E_SERVER_BIN=$PWD/target/debug/hyprlayer-server cargo test -p hyprlayer-e2e --test review_lifecycle -- --nocapture 2>&1 | grep 'keeping the sandbox at'`. Without `--nocapture`, a passing test swallows that line. <!-- src: crates/hyprlayer-e2e/src/sandbox.rs:392 -->
  2. Serve the sandbox with `SB=<path>`, `REPO=$PWD`, and `(cd "$SB/daemon-cwd" && exec env -i HOME="$SB/home" USER=e2e LOGNAME=e2e PATH="$SB/bin:/usr/bin:/bin:/usr/sbin:/sbin" SHELL=/bin/sh TMPDIR="$SB/tmp" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" GIT_CONFIG_NOSYSTEM=1 HYPRLAYER_APP_CONFIG_DIR="$SB/app-config" HYPRLAYER_APP_DATA_DIR="$SB/app-data" HYPRLAYER_APP_LOG_DIR="$SB/logs" "$REPO/target/debug/hyprlayer-server" serve) > "$SB/serve.log" 2>&1 < /dev/null &`. Pass the desktop's own `XDG_RUNTIME_DIR`: on Linux the socket lives under it, and a sandbox-local one leaves the app looking in the wrong place. <!-- src: crates/hyprlayer-e2e/src/sandbox.rs:178 --> <!-- src: crates/hyprlayer-server/src/socket.rs:36 -->
  3. Run `verify.sh launch attach "$SB/app-data"`. Doctor passes when this app connects to the sandbox server and spawns none of its own.
  4. Teardown: run `verify.sh cleanup <run-dir>`. Then stop the sandbox server with the step 2 env and `hyprlayer-server stop`, and `rm -rf "$SB"`.
- **Read the state before driving.** Get the workstream id and `worktreePath` from `window.__TAURI__.core.invoke('list_all_workstreams')`, then save `window.__TAURI__.core.invoke('get_review', { workstreamId })` as `00-review.json`. <!-- src: src/lib/bindings.ts:113 --> Expect `status` `approved` and `viewed` `["src/new.rs"]` (1 of 7).

Steps:

- **Open.** Select `Review work` under `demo`, then click `[aria-label="Review changes"]`. <!-- src: src/features/workstreams/ChangesPanel.tsx:582 --> The URL search gets `review=files`. <!-- src: src/routes/ProjectPage.tsx:905 --> The header reads `Review changes` with an `Approved` chip and `1 / 7 viewed`. <!-- src: src/features/review/ReviewHeader.tsx:58 --> <!-- src: src/features/review/ReviewHeader.tsx:145 -->
- **Header.** `span[role="progressbar"][aria-label="Files viewed"]` has `aria-valuenow` = viewed and `aria-valuemax` = total. <!-- src: src/features/review/ReviewHeader.tsx:149 --> The second line reads `<branch> → main · 7 files +A −D`. <!-- src: src/features/review/ReviewHeader.tsx:208 -->
- **Tree.** Rows are `div[role="button"]` whose `title` is the path (`old → new` for a rename). The selected row has `aria-current="true"`. <!-- src: src/features/review/FileTree.tsx:166 --> <!-- src: src/features/review/FileTree.tsx:175 --> Type `edit` into `input[aria-label="Filter files"]` and only matching rows remain. Type `zzz` and the tree reads `No matching files`. <!-- src: src/features/review/FileTree.tsx:253 --> <!-- src: src/features/review/FileTree.tsx:265 --> Clear the filter afterwards. A row with open notes or threads shows a badge titled `N open item(s)`. In the kept sandbox the tree also opens with a `Changed since your review` group, holding `src/edit.rs`, which the fixture rewrote after an earlier round. <!-- src: crates/hyprlayer-e2e/tests/review_lifecycle.rs:1310 --> <!-- src: crates/hyprlayer-e2e/tests/review_lifecycle.rs:1319 --> <!-- src: src/features/review/FileTree.tsx:197 -->
- **Diff pane.** Click the `src/edit.rs` row. The pane header shows `i of 7`. <!-- src: src/features/review/DiffPane.tsx:288 --> `[aria-label="Next file"]` and `[aria-label="Previous file"]` move the selection and `aria-current`. <!-- src: src/features/review/DiffPane.tsx:294 --> <!-- src: src/features/review/DiffPane.tsx:291 --> In `[aria-label="Diff layout"]`, click the `Side by side` radio; it's disabled for added and deleted files. <!-- src: src/features/review/DiffPane.tsx:320 --> <!-- src: src/features/review/DiffPane.tsx:25 -->
- **Empty states.** Select `assets/blob.bin` and the pane reads `Binary file`. <!-- src: src/features/review/DiffPane.tsx:336 --> Select `big.txt` and it reads `File too large to diff`. <!-- src: src/features/review/DiffPane.tsx:348 -->
- **Viewed.** On an unviewed file, click the row's `button[aria-label="Mark viewed"]`. It flips to `Mark not viewed` with `aria-pressed="true"`, and the header count rises by one. <!-- src: src/features/review/FileTree.tsx:105 --> The pane's `Viewed` checkbox does the same. <!-- src: src/features/review/DiffPane.tsx:315 --> Read back: the path is in `get_review`'s `viewed`. Untick it to restore `1 / 7`.
- **Mark viewed, next.** The footer button reads `Mark viewed, next: <basename>`. It marks the current file and selects the next unviewed one. <!-- src: src/features/review/DiffPane.tsx:374 --> Once all are viewed, the footer reads `All 7 files viewed.` <!-- src: src/features/review/DiffPane.tsx:370 -->
- **Mark all viewed.** The ghost button in the header marks every unviewed file, one `set_review_file_viewed` each. <!-- src: src/features/review/ReviewHeader.tsx:141 --> <!-- src: src/lib/queries.ts:1202 --> To restore, untick the files that weren't in `00-review.json`'s `viewed`.
- **Close.** Click `Chat` (text). <!-- src: src/features/review/ReviewHeader.tsx:124 --> The chat and sidebar return, `review` leaves the URL, and reopening lands on the same file. <!-- src: src/features/review/reviewStore.ts:9 -->
- **Plan tab.** There's no recipe on the sandbox: its Plan panel names `thoughts/shared/plans/e2e.md`, which isn't in the thoughts index. On a shared workstream whose implement session has one, the `Plan d/t` tab lists the plan's coverage, and clicking a file there jumps back to `Files` with it selected. <!-- src: src/features/review/ReviewWorkspace.tsx:194 -->
- **Proof.** Save screenshots of the header, a text diff, both empty states and the viewed toggle, plus the `get_review` JSON before and after.

## Gotchas

- `Review changes` is also the `h1` text and the Ready card's button text, so select the header icon by `aria-label`, not by text. <!-- src: src/features/review/ReviewHeader.tsx:126 -->
- `Group by` (`Plan task` / `Folder`) only renders when the implement session's todos are tracked. The sandbox has no plan, so it shows folder groups only. <!-- src: src/features/review/FileTree.tsx:259 --> <!-- src: src/features/review/ReviewWorkspace.tsx:220 -->
- `Ship` appears in the header whenever there's no live PR. Clicking it opens the Ship check, not a push: see [ship-pr.md](./ship-pr.md). <!-- src: src/features/review/ReviewHeader.tsx:185 -->
- Changing a file in the worktree drops it out of `viewed`, because the stored oid no longer matches. A stray edit during the run moves the count. <!-- src: crates/hyprlayer-core/src/runtime/reviews.rs:464 -->
- Raw `set_review_file_viewed` skips react-query invalidation. Toggle through the UI, or reload before judging. <!-- src: .claude/skills/verify-hyprlayer/SKILL.md:79 -->
- `.hyprlayer/review/` and `.hyprlayer/settings.toml` never appear as review files: they're excluded. <!-- src: crates/hyprlayer-core/src/review/diff.rs:130 -->
