# Workshop: agents and skills

The Workshop (`/workshop`) is where the user manages Claude agents and skills. A left nav switches `Your agents`, `System agents`, `Skills` and `Prompts`, and a scope picker chooses `Global · all projects` or one project. The user can create an agent in the editor, save it as a `.claude/agents/<name>.md` file in that scope, reopen it, and delete it. Skills list every `SKILL.md` the harnesses can see, tagged by source (`Project`, `Personal`, `Hyprlayer`, plugin), with filters and a detail pane.

## Sub-features

- `wk-open` opens the Workshop and switches its sections.
- `wk-scope` switches scope between Global and a project.
- `wk-agent-create` creates a project-scoped agent and saves it to disk.
- `wk-agent-delete` deletes that agent from its context menu.
- `wk-skill-list` lists a project skill with its `Project` badge and filters.
- `wk-skill-detail` opens a skill's detail pane.

## How to get to it (user POV)

- Header gear `Settings`, then `Workshop`. <!-- src: src/features/settings/SettingsPopover.tsx:180 -->
- `/workshop?section=skills|system|prompts&project=<path>` deep links; the prompt palette's prompt rows open `section=prompts`. <!-- src: src/features/workshop/workshopSearch.ts:3 --> <!-- src: src/features/prompts/PromptPalette.tsx:126 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance seeded with `verify-demo`. No model turns.
- **Scope to `verify-demo` before any write.** A global-scope agent is written to the real `~/.claude/agents`, shared with the user's Claude Code. <!-- src: crates/hyprlayer-core/src/agents.rs:166 --> A project agent goes to `<project>/.claude/agents/`. <!-- src: crates/hyprlayer-core/src/agents.rs:163 -->
- For skills, seed a project skill on disk: `mkdir -p <SEED>/.claude/skills/verify-skill`, then write `SKILL.md` there with frontmatter `name: verify-skill`, `description: Verify map seed skill`. Project skills are read from `<worktree>/.claude/skills`. <!-- src: crates/hyprlayer-core/src/skills.rs:201 -->
- Agent overlays (status, folder, default agent) are saved to the profile's app settings. <!-- src: src/features/agents/AgentLibrary.tsx:230 -->

- **Open (`wk-open`).** Click `[aria-label="Settings"]`, then `Workshop` (`strategy: "text"`). The URL is `/workshop`. `nav[aria-label="Workshop"]` lists `Your agents`, `System agents`, `Skills` and `Prompts`, each with a count, and the active one has `aria-current="true"`. <!-- src: src/components/workshop.tsx:106 --> <!-- src: src/features/workshop/useWorkshopChrome.ts:42 --> <!-- src: src/components/workshop.tsx:145 --> With no agent selected, the detail reads `Select an agent`. <!-- src: src/features/agents/AgentLibrary.tsx:516 -->
- **Scope (`wk-scope`).** Click `[aria-label="Workshop scope"]`, which reads `Global · all projects`. It lists `[role="menuitem"]` rows; click `verify-demo`. <!-- src: src/components/workshop.tsx:237 --> <!-- src: src/components/workshop.tsx:246 --> <!-- src: src/components/workshop.tsx:284 --> The trigger now reads `verify-demo`, and the URL gains `project=<path>`. <!-- src: src/routes/WorkshopPage.tsx:50 --> `Your agents` now counts project agents only. <!-- src: src/features/agents/agents.ts:355 -->
- **Create (`wk-agent-create`).** Click `New agent`. <!-- src: src/features/agents/AgentLibrary.tsx:426 --> The editor `h1` reads `Untitled agent`, `Save` is disabled, and `[aria-label="Available in all projects"]` is off (a project draft). <!-- src: src/features/agents/AgentEditor.tsx:125 --> <!-- src: src/features/agents/AgentEditor.tsx:166 --> <!-- src: src/features/agents/AgentEditor.tsx:370 --> <!-- src: src/features/agents/AgentLibrary.tsx:368 --> Type `Verify Agent` into `input[placeholder="e.g. Plan Reviewer"]` and a description into `textarea[placeholder="When should this agent be used?"]`. <!-- src: src/features/agents/AgentEditor.tsx:277 --> <!-- src: src/features/agents/AgentEditor.tsx:285 --> The subtitle shows `· Unsaved`. <!-- src: src/features/agents/AgentEditor.tsx:141 --> Click `[aria-label="Save"]`. A toast reads `"Verify Agent" saved` (curly quotes), and the editor closes back to the list. <!-- src: src/features/agents/AgentLibrary.tsx:267 --> <!-- src: src/features/agents/AgentLibrary.tsx:268 -->
- **Read back.** Run `ls <SEED>/.claude/agents/` and expect `verify-agent.md`, with `name: Verify Agent` in its frontmatter. <!-- src: crates/hyprlayer-core/src/runtime/agents.rs:135 --> <!-- src: crates/hyprlayer-core/src/agents.rs:548 --> `invoke('list_agent_files', { projectPath })` <!-- src: src/lib/bindings.ts:270 --> has an entry with `source` `project` and that `path`. `~/.claude/agents/verify-agent.md` must not exist.
- **Reopen.** The list shows a row `button[data-list-row]` reading `Verify Agent`; it has no `title`, so select it by text: `//button[@data-list-row][contains(.,"Verify Agent")]`. <!-- src: src/features/agents/AgentList.tsx:159 --> <!-- src: src/components/workshop.tsx:318 --> Click it to load the editor with the saved name.
- **Delete (`wk-agent-delete`).** Right-click that row and choose `Delete agent`. <!-- src: src/features/agents/AgentList.tsx:178 --> <!-- src: src/features/agents/AgentLibrary.tsx:577 --> `[role="dialog"][aria-label="Delete agent"]` shows the file path. Click `Delete`. <!-- src: src/features/agents/AgentLibrary.tsx:673 --> <!-- src: src/features/agents/AgentLibrary.tsx:674 --> A toast reads `Deleted 1 agent`, the file is gone from disk, and `list_agent_files` no longer has it. <!-- src: src/features/agents/AgentLibrary.tsx:287 -->
- **Skills (`wk-skill-list`).** Click `Skills` in the nav. The list header reads `<filter> · N skills`. <!-- src: src/features/workshop/SkillLibrary.tsx:299 --> With scope `verify-demo`, a row titled `verify-skill` carries the `Project` badge and the seeded description. <!-- src: src/features/workshop/skills.ts:39 --> <!-- src: src/features/workshop/SkillLibrary.tsx:319 --> The `Source` filters are `All`, `Yours`, `Installed`, `Hyprlayer bundle` and `Plugins`, each with a count. <!-- src: src/features/workshop/skills.ts:8 --> <!-- src: src/features/workshop/SkillLibrary.tsx:229 --> Type `verify` into `input[aria-label="Filter skills"]` and only matching rows remain. <!-- src: src/features/workshop/SkillLibrary.tsx:306 --> Read back with `invoke('list_skill_files', { projectPath })` <!-- src: src/lib/bindings.ts:277 -->, which includes `source: "project"` and `name: "verify-skill"`.
- **Detail (`wk-skill-detail`).** Click the `verify-skill` row. The empty `Select a skill` is replaced by the detail, which has rail sections `Source`, `Loaded by` (`No agents preload it`), `Invocation`, `Access` and `Size`. <!-- src: src/features/workshop/SkillLibrary.tsx:353 --> <!-- src: src/features/workshop/SkillDetail.tsx:351 --> <!-- src: src/features/workshop/SkillDetail.tsx:390 --> <!-- src: src/features/workshop/SkillDetail.tsx:430 --> The URL gains `skill=<path>`. <!-- src: src/routes/WorkshopPage.tsx:77 -->
- **Cleanup.** `rm -rf <SEED>/.claude`, so later recipes that `ls` the folder (for example `workstream-create.md`) still see only `README.md`.
- **Proof.** Save screenshots of the nav, the scope switch, the editor before and after save, the delete confirm and the skill detail, plus `list_agent_files`/`list_skill_files` JSON and `ls` of `.claude/agents` before and after delete.

## Gotchas

- In Global scope, `New agent` drafts a user agent (`~/.claude/agents`), and `Duplicate`, `Set as default` and `Move to folder…` act on the user's real agents. Don't drive those in Global. <!-- src: src/features/agents/AgentLibrary.tsx:368 -->
- Turning `Available in all projects` on moves the file to `~/.claude/agents` on save, then deletes the project copy. <!-- src: src/features/agents/AgentEditor.tsx:369 --> <!-- src: src/features/agents/AgentLibrary.tsx:253 -->
- Read-only (managed) agents have no `Save` and no `More actions`. A modified system agent's menu says `Restore shipped version` rather than `Delete agent`. <!-- src: src/features/agents/AgentEditor.tsx:146 --> <!-- src: src/features/agents/AgentLibrary.tsx:577 -->
- `Add skill`, `skills.sh` browse, `Check for updates`, `Update all` and `Restore` hit the network and install into app-owned stores linked into harness dirs (ADR-0010). Don't drive them. <!-- src: src/features/workshop/SkillLibrary.tsx:223 --> <!-- src: src/features/workshop/SkillLibrary.tsx:245 -->
- The footer's `Update agents to <v>` rewrites the shared agent bundle. Don't click it. <!-- src: src/features/workshop/BundleFooter.tsx:33 -->
- The list refetches on save, but a file written by hand (like the seeded skill) needs a reload or a scope switch to appear, because react-query caches `list_skill_files` per scope. <!-- src: src/lib/queries.ts:1371 -->
- Prompts are global: the scope picker hides on `Prompts`. <!-- src: src/features/workshop/useWorkshopChrome.ts:55 -->
