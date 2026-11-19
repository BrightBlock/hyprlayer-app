# Hyprlayer verification map

The maintained source for verifying Hyprlayer's user-facing behavior in the desktop app. Read this index, then use the matching feature file as the recipe. Launch, doctor, evidence and cleanup live in [../SKILL.md](../SKILL.md).

## Baseline preconditions

- An instance launched by `verify.sh launch isolated` (or `shared`, when the feature says so) whose `verify.sh doctor <run-dir>` passed.
- `driver_session start` on the bridge port doctor printed.
- The window is resized to ≤1800 logical px wide.
- Isolated runs are seeded with the non-git folder project `verify-demo` from SKILL.md's Launch section.
- Never drive an instance this run didn't launch or doctor, except the user's own `.dev` instance when a feature needs shared data and the user is fine with it.

## Driving conventions

- Prefer `aria-label`s, `data-testid`s, dialog titles and placeholders over coordinates.
- Use IPC and store imports to seed or read back, never as the action under test.
- Restore anything you change in shared data (thoughts files, settings) before the run ends.

## Proof and skip reporting

- UI proof is a screenshot of the action and of the resulting state, saved in the run's evidence dir.
- Mutation proof adds a read-back through a second view (a list RPC's JSON, the file on disk).
- Record the feature ID and the entry point you used. Don't report one entry point as verified through another.
- Report an unreachable entry point with the attempted step and the unmet precondition.

## Feature entry contract

Each file has an H1, one paragraph of user-visible behavior, then exactly these H2s in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with the MCP bridge`, `Gotchas`.

## Features

Mode is the cheapest one each feature works in. **Cost** marks anything that spends model turns, touches GitHub, or writes data you share.

**App shell and settings**
- [Server status](./server-status.md) covers the status bar's connection label and the Servers popover. Isolated, free.
- [Settings popover and page](./settings-popover.md) covers the header gear popover's quick settings, its links and the Settings section nav. Isolated, free.
- [Appearance](./settings-appearance.md) covers themes, the popover's Light/Dark/Auto, zoom, text size and fonts, all stored in localStorage. Isolated, free; restore them afterwards.
- [Preferences](./settings-preferences.md) covers the default harness, quit confirm, keybinding unbind/reset, harness tabs, agent control and the Full Disk Access notice. Isolated, free.
- [Servers page](./servers-page.md) covers adding, validating, testing, editing and removing SSH servers, plus Set default and Install. Isolated, free; Install needs a reachable host.
- [Cloud account](./account-sign-in.md) covers the account button and starting and cancelling GitHub sign-in. Isolated; it opens your browser.
- [Home dashboard and setup wizard](./dashboard-home.md) covers the first-run blank, the dashboard panels, Discover and the onboarding wizard. Isolated, free.
- [Search everything](./search-palette.md) covers the spotlight palette's groups, doc search and navigation. Isolated, free.

**Projects and workstreams**
- [Add, configure and remove a project](./projects-add-settings.md) covers Add project by typed path, Project settings and health, and Remove project. Isolated, free.
- [Create a workstream](./workstream-create.md) covers the folder-mode New workstream dialog and its stored row. Isolated, free.
- [Workstream lifecycle](./workstreams-lifecycle.md) covers rename, pin, archive, the Archived worktrees popover and git-mode creation. Isolated, free; git mode writes the shared CLI config.
- [Project pipeline board](./board-pipeline.md) covers the blank state, board columns, the Triage/Table views and the project picker. Isolated, free.
- [Workshop](./workshop-agents-skills.md) covers sections, scope, creating and deleting a project agent, and the skills list and detail. Isolated, free.
- [Goal sheet](./goals-sheet.md) covers opening and validating a goal and its checks. Free up to submit; one run to met costs about $0.01 on Haiku.

**Agent sessions**
- [Send and run a turn](./session-send.md) covers starting from the Agent tab, follow-ups, queueing, Stop, a session ending and Clear context. Costs model turns (free on an OpenCode `-free` model once titles and recaps are off).
- [Session pickers](./session-pickers.md) covers the agent-mode tile and the status bar's model, effort and permission menus. Costs model turns for the live switch.
- [Permissions and questions](./session-permissions.md) covers the permission card and the question card. Costs Claude Haiku turns.
- [Session tabs](./session-tabs.md) covers New agent, per-tab drafts, rename, archive and the Stop agent confirm. Costs model turns once a tab starts.
- [Attach context](./session-attach.md) covers `@` file autocomplete, chips and sending with attachments. Costs model turns.

**Terminals and PTY agents**
- [Dock terminals](./terminal-shell-tab.md) covers the dock's login shells: open, close, reattach with replay, and dock position. Isolated, free.
- [Setup and Run scripts](./terminal-scripts.md) covers editing scripts, Start Run with `$HL_PORT`, Stop, Clear and Setup on create. Isolated, free.
- [Engine tabs](./terminal-engine-tab.md) covers Multiplexer mode and engine tabs with a stub engine, plus the optional Claude Code pills. Isolated, free; the Claude step costs.
- [App actions in the default engine](./terminal-action-engine.md) covers the first Agent tab and ⌘K prompts opening in the default engine. Isolated, free with the stub.

**Library and prompts**
- [Library: browse](./library-browse.md) covers the nav filters, project scope, list search/filter/sort, read state and the thoughts sync button. Isolated for reads; sync needs shared.
- [Library: doc](./library-doc.md) covers opening a doc, the Contents/Properties rail, editing, saving, the unsaved-changes guard and conflict reload. It writes your real `~/thoughts`, so get the user's OK and use a scratch doc.
- [Library: create](./library-create.md) covers the New thought dialog and the file it writes. It writes your real `~/thoughts`, so get the user's OK.
- [Prompt library](./prompts-library.md) covers Workshop → Prompts and the run palette. Isolated, free; running a prompt costs on Claude, $0 on a free OpenCode model.

**Plan, review and ship**
- [Plan tab](./plan-tab.md) covers the right sidebar plan preview, its counts, the open manual checks roll-up, ticking a check and the Ship warning. Shared; edits real plans, so copy and revert.
- [Changes tab](./review-changes-tab.md) covers the changes, commits and files tabs, stage/unstage, discard and the diff viewer tab. Uses the `review_lifecycle` sandbox, free.
- [Review workspace](./review-workspace.md) covers opening the review, its file tree, diff pane and viewed state, and how to seed the `review_lifecycle` sandbox. Sandbox, free.
- [Review notes, threads and verdicts](./review-threads.md) covers notes, comments, replies, resolve, Submit review and delivery to the agent. Sandbox, free.
- [Review lifecycle steps](./review-lifecycle.md) covers the Plan tab's Implement/Validate/Review/Ship stages and the chat's validate and Ready for review cards. Shared; anything that starts an agent runs only in `jtsiros/hyprlayer-live-tests`.
- [Ship and pull requests](./ship-pr.md) covers the Ship check, PR bar, merge, archive, attach PR, CI checks, target branch and update from base. Sandbox for the offline tier; the live tier is gated by ADR-0012.
