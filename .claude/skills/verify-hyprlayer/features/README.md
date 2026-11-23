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
- [Preferences](./settings-preferences.md) covers the default harness, quit confirm, keybinding unbind/reset, harness tabs, agent control, the Lead Engineer card (with its permission mode) and Guardrails, and the Full Disk Access notice. Isolated, free.
- [Servers page](./servers-page.md) covers adding, validating, testing, editing and removing SSH servers, plus Set default and Install. Isolated, free; Install needs a reachable host.
- [Cloud account](./account-sign-in.md) covers the account button and starting and cancelling GitHub sign-in. Isolated; it opens your browser.
- [Home dashboard and setup wizard](./dashboard-home.md) covers the first-run blank, the dashboard panels, Discover and the onboarding wizard. Isolated, free.
- [Search everything](./search-palette.md) covers the spotlight palette's groups, doc search and navigation. Isolated, free.

**Projects and workstreams**
- [Add, configure and remove a project](./projects-add-settings.md) covers Add project by typed path, Project settings and health, and Remove project. Isolated, free.
- [Create a workstream](./workstream-create.md) covers the folder-mode New workstream dialog and its stored row. Isolated, free.
- [Workstream lifecycle](./workstreams-lifecycle.md) covers rename, pin, archive, the Archived worktrees popover and git-mode creation. Isolated, free; git mode writes the shared CLI config.
- [Project pipeline board](./board-pipeline.md) covers the blank state, board columns, the Triage/Table views, the project picker, and the Lead chip and dock (λ mark, day summary, Remote Control button) on git projects. Isolated, free.
- [Workshop](./workshop-agents-skills.md) covers sections, scope, creating and deleting a project agent, and the skills list and detail. Isolated, free.
- [Goal sheet](./goals-sheet.md) covers opening and validating a goal and its checks. Free up to submit; one run to met costs about $0.01 on Haiku.

**Agent sessions**
- [Send and run a turn](./session-send.md) covers starting from the Agent tab, follow-ups, queueing, Stop, a session ending and Clear context. Costs model turns (free on an OpenCode `-free` model once titles and recaps are off).
- [Session pickers](./session-pickers.md) covers the agent-mode tile and the status bar's model, effort and permission menus (`Stage default` through `Bypass permissions`). Costs model turns for the live switch.
- [Permissions and questions](./session-permissions.md) covers the permission card and the question card. Costs Claude Haiku turns.
- [Session tabs](./session-tabs.md) covers New agent, per-tab drafts, rename, archive and the Stop agent confirm. Costs model turns once a tab starts.
- [Attach context](./session-attach.md) covers `@` file autocomplete, chips and sending with attachments. Costs model turns.
- [Failed start and Resume](./session-failed-start.md) covers the `Failed to start session` view, a refused Resume and a Resume that replays the first turn. Isolated; the retry costs one Haiku turn.
- [Sync notices across clients](./session-sync.md) covers the `N new turns` strip, `Load`, and the draft guard's `Pull N turns`. Needs a second debug app on the same server; costs Haiku turns.

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

## Not yet mapped

User-facing surfaces with no recipe yet. Each names its entry point in source.

- Terminal resume after a dropped connection (#410): `src/features/session/TerminalTab.tsx:112`. A local server kill only reaches the `[process exited]` branch, because the PTY dies with the server; resuming needs a relay drop with the server still up (a remote host, or the `crates/hyprlayer-e2e` container tier).
- Archive Keep, Purge merged and expiry: `src/features/archive/ArchivedWorktreesPopover.tsx:272`, `src/features/archive/archiveStatus.ts:32`
- Workstream docs panel: `src/features/workstreams/WorkstreamDocsPanel.tsx:23`
- File viewer tab and ⌘S save: `src/features/workstreams/FileViewerTab.tsx:29`, `src/lib/fileSave.ts:14`
- Sidebar collapse and project drag: `src/routes/Root.tsx:695`, `src/features/projects/useProjectDrag.ts:34`
- Add project `Create new` fields: `src/features/projects/CreateProjectFields.tsx:64`
- Agent bundle `Update agents to <v>`: `src/features/workshop/BundleFooter.tsx:32`
- Skill install (`Add skill`, skills.sh): `src/features/workshop/AddSkillDialog.tsx:19`
- Live goal chip controls (Pause, Stop, Edit): `src/features/statusbar/SessionControlsCluster.tsx:101`
- Agent-spawned PTY tabs: `src/features/multiplexer/ptyAgentBridge.ts:39`
- Onboarding setup terminal: `src/features/onboarding/SetupTerminal.tsx:26`
- Dock maximize: `src/features/session/scriptsStore.ts:102`
- Terminal font zoom keys: `src/lib/terminal/terminalFontSize.ts:18`
- Multiplexer persist and notify toggles: `src/features/settings/AdvancedSettings.tsx:194`
- PTY bypass-permissions relaunch: `src/features/multiplexer/PtySessionControls.tsx:50`
- Archive script run and `Skip script & archive`: `src/features/workstreams/ArchiveWorkstreamDialog.tsx:13`
- Harness sign-in card: `src/features/settings/harnesses/AuthCard.tsx:3`
- Handoff banner: `src/features/session/SessionChat.tsx:273`
- Rewind: `src/features/session/TranscriptView.tsx:123`
- Prompt history: `src/features/session/promptHistory.ts:20`
- Steering (⌘Enter mid-turn): `src/features/session/SessionChat.tsx:1334`
- Background tasks: `src/features/session/BackgroundTasksIndicator.tsx:8`
- Agent sessions popover and archived sessions: `src/features/session/AgentSessionsPopover.tsx:63`
- Doc preview tab and split panes: `src/features/session/DocPreviewTab.tsx:26`
- Plan approval and build until met: `src/features/plan/BuildUntilMet.tsx:110`
- Draft plan preview: `src/features/plan/PlanPreviewDraft.tsx:30`
- Plan aside header actions: `src/features/plan/PlanAside.tsx:163`
- Review plan coverage: `src/features/review/PlanCoverage.tsx:141`
- Goal handoff: `src/features/review/GoalHandoff.tsx:23`
- Commit flow (`Commit · N`): `src/features/workstreams/GitPanel.tsx:237`
- Settings sections Git, Notifications, Goals, Binaries: `src/features/settings/GitSettings.tsx:23`, `src/features/settings/NotificationsSettings.tsx:14`, `src/features/settings/GoalsSettings.tsx:13`, `src/features/settings/BinariesSettings.tsx:7`
- Server update flow: `src/features/hosts/ServerUpdateConfirm.tsx:12`
- Disconnected notice and reconnect: `src/features/system/ServerDisconnectedNotice.tsx:20`, `src/features/hosts/reconnect.ts:148`
- SSH prompt dialog: `src/features/hosts/SshPromptDialog.tsx:22`
- App updater: `src/features/settings/updates.tsx:14`
- Usage meter: `src/features/statusbar/UsageMeter.tsx:4`
- Status bar thoughts sync: `src/features/library/ThoughtsSyncButton.tsx:165`
- Pinned prompt chips and prompt hotkeys: `src/features/prompts/PromptChips.tsx:17`, `src/features/prompts/usePromptHotkeys.ts:15`
- Per-project prompt overrides: `src/features/prompts/PromptLibrary.tsx:277`
- Doc Properties related picker and raw YAML: `src/features/library/DocProperties.tsx:23`
- View on GitHub: `src/routes/Root.tsx:1002`
- Lead dock beyond its idle state (send, expand to the Lead's chat and minimize, its question, the note strip, the day summary, `Open in Claude` and the copy-link button), and the status bar's Lead model, effort and permission pills while it's expanded: `src/features/lead/LeadDock.tsx:76`. Each needs a Lead turn.
- Lead-driven board marks (`◎ Driven by the Lead`, a Lead question pointing at a workstream card, `Archive`/`Later`): `src/features/board/PipelineCard.tsx:115`, `src/features/lead/lead.ts:71`. Need a Lead that spawns or asks.
- Sidebar project header as the Lead (Lead mark, the needs-you warn glow on the repo avatar titled `Lead: 1 workstream needs you` or `Lead: N workstreams need you`, main hidden once the Lead holds a session): `src/features/lead/LeadProjectTitle.tsx:22`, `src/routes/Root.tsx:1292`. The mark alone renders free on the `review_lifecycle` sandbox.
- The Lead panel (Queue, Running, Stats), in the right aside of the Lead's own tab and beside the expanded dock's chat: `src/features/lead/LeadPanel.tsx:33`. Queue items need a Lead `queue_add`; Running needs Lead-spawned workstreams; Stats render free once the Lead holds a session.
- Sub-workstreams folding under their parent's board card (`N sub-workstreams`, `↳ <parent>`) and nesting under their parent in the sidebar (`Collapse <title>`/`Expand <title>`): `src/features/board/PipelineBoard.tsx:121`, `src/routes/Root.tsx:1417`. Need `parentWorkstreamId`, set only for agent-created workstreams.
- Server-side workstream list changes (an agent's create or archive) reaching the sidebar without refocus: `src/lib/events.ts:134`. Needs a create from outside the webview.
- Session wake after a server restart and `wake_when` turns: server-side; their `[from hyprlayer]` and `[from hyprlayer · wake <id>]` turns render as incoming-message cards. Needs a mid-turn session and a server restart.
- Incoming-message cards from other agents and hyprlayer (title, `Open agent`/`Open session`/`Open terminal`, a wake's `PR #N` link): `src/features/session/IncomingMessageCard.tsx:43`, `src/features/session/incomingMessage.ts:44`. A real one needs another agent's `message_agent` or a wake; session-send.md notes that typed text with the header renders the same card.
- Plan `[x]` write-back: `src/features/plan/PlanAside.tsx:241`
