# Session pickers: mode, model, effort, permission

Each chat tab has an agent mode (General, Research, Plan, Implement, Validate), picked from the coloured tile at the composer's left edge. The mode is fixed once the session starts. The focused chat's model, reasoning effort and permission mode sit in the status bar at the bottom of the window, not in the composer. The model menu groups models by harness (Claude Code, Codex, OpenCode, plus Ollama when configured), so picking a model also picks the harness. After a session starts, models from other providers are disabled. Changing the model or permission mode on a live session relaunches the harness, and asks first if a turn is running. Claude switches permission mode in place.

## Sub-features

- `pick-mode` changes the tab's agent mode before the first send. It's a read-only tile afterwards.
- `pick-model` switches model and harness on an unstarted tab.
- `pick-model-live` switches a running Claude session's model, which relaunches it with `--resume`.
- `pick-model-lock` disables other providers' models once the session has started.
- `pick-effort` sets the reasoning effort. Claude models with effort tiers and Codex only.
- `pick-permission` sets the permission mode. Claude and Codex only.
- `pick-harnesses` opens Settings › Harnesses from the model menu.

## How to get to it (user POV)

- Click the coloured tile left of the composer (`Agent mode: <Mode>`). <!-- src: src/features/session/AgentModePicker.tsx:32 -->
- With a chat tab focused, use the status bar's permission pill, `Model: <name>` and `Effort: <level>`. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:290 --> <!-- src: src/features/statusbar/SessionControlsCluster.tsx:352 --> <!-- src: src/features/statusbar/SessionControlsCluster.tsx:409 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance with the `Verify map seed` workstream ([workstream-create.md](./workstream-create.md)) and the free-path seed from [session-send.md](./session-send.md). Leave Claude installed too, so its group has models.
- **Cost:** every pick on an unstarted tab only patches the tab, so it's free. `pick-model-lock` costs $0 on a free OpenCode session. `pick-model-live` needs a running Claude session: one short Haiku turn, the relaunch, and one turn on the new model to read it back. That's a few cents on a fresh session.

- **Mode.** Click `button[aria-label="Agent mode: General"]`. `[role="menu"][aria-label="Agent mode"]` lists the five modes as `button[role="menuitemradio"]`. Click `Implement` (`strategy: "text"`). The tile's label becomes `Agent mode: Implement`. Read back the tab's `mode` through the live `workstreamTabs` module (SKILL.md, Drive step 5): `useWorkstreamTabsStore.getState().byWs[<wsId>].shells`. Set it back to `General` before any send, because other stages run stage prompts. <!-- src: src/features/session/AgentModePicker.tsx:51 --> <!-- src: src/features/session/AgentModePicker.tsx:65 --> <!-- src: src/components/OptionMenuList.tsx:106 -->
- **Model.** Click `button[aria-label^="Model: "]`. `[role="menu"][aria-label="Model"]` shows the group headings `Claude Code`, `Codex` and `OpenCode`. Pick a free OpenCode row in JS. Its `textContent` ends with the provider label (`Beam FreeOpenCode`), so match the start: `[...document.querySelectorAll('[aria-label="Model"] button[role="menuitemradio"]')].find(b => b.textContent.trim().startsWith('<name>')).click()`. The trigger relabels, and the tab's `harness`/`opencodeModel` update in the tab store. Then pick `Haiku 4.5` (fallback catalog label; the live claude probe may name it differently): the tab's `harness` reads `claude` and `model` holds the full id (`claude-haiku-4-5-20251001`). <!-- src: src/features/statusbar/SessionControlsCluster.tsx:367 --> <!-- src: src/features/session/SessionChat.tsx:719 --> <!-- src: src/features/session/SessionChat.tsx:740 --> <!-- src: src/lib/domain/models.ts:21 -->
- **More models.** The Claude group's `More models` row expands the unlisted models. <!-- src: src/features/session/SessionChat.tsx:723 -->
- **Effort.** Haiku 4.5 has no effort tiers, so no button renders. Pick `Sonnet 5`, whose trigger starts at `Effort: High`. Click `button[aria-label^="Effort: "]` and pick `Low` in `[aria-label="Effort"]`. The trigger reads `Effort: Low`, and the tab's `effort` is `low`. On OpenCode, no effort button renders. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:435 --> <!-- src: src/features/session/SessionChat.tsx:1495 --> <!-- src: src/lib/domain/models.ts:219 -->
- **Permission.** On a Claude tab, click `button[aria-label^="Permission mode: "]` and pick `Ask each time` by exact text in JS, as for the model. The pill reads `Permission mode: Ask each time`, and the tab's `permissionMode` is `default`. On OpenCode, no pill renders. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:315 --> <!-- src: src/lib/domain/permissions.ts:14 --> <!-- src: src/features/session/SessionChat.tsx:1500 -->
- **Lock.** Start a free OpenCode session (session-send.md), then open the model menu. Claude and Codex rows are `disabled` with a `title`, and other OpenCode providers' rows are disabled too. The mode tile is now `span[role="img"]`, not a button. <!-- src: src/features/session/SessionChat.tsx:703 --> <!-- src: src/components/OptionMenuList.tsx:119 --> <!-- src: src/features/session/AgentModePicker.tsx:37 -->
- **Live model switch.** On a running Claude Haiku session, pick `Sonnet 5` while idle (no confirm). The log gains `relaunched`, but the session row's `model` comes from the next turn's `init`, so `list_active_sessions` still reports Haiku until you send one short turn. Do it on a fresh, short session: on a 67K-token resumed context, that one Sonnet turn cost $0.27. <!-- src: crates/hyprlayer-core/src/runtime/sessions/forwarder.rs:1303 --> Mid-turn, the `Switch model?` dialog opens first: `Cancel` keeps the old model. <!-- src: src/features/session/SessionChat.tsx:806 --> <!-- src: src/features/session/sessionStore.ts:1089 -->
- **Harnesses.** In the model menu, click `Harnesses` (`strategy: "text"`). The route becomes `/settings` with `section=harnesses`. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:386 --> <!-- src: src/routes/ProjectPage.tsx:962 -->
- **Proof.** Screenshot each open menu and the relabelled triggers. Save the tab-store `shells` JSON before and after, and the `list_active_sessions` JSON for the live switch.

## Gotchas

- The status bar shows the controls of the **focused pane's** active tab. Bridge clicks send no pointer events, so clicking into another pane doesn't move focus. Keep one pane open. <!-- src: src/features/session/sessionControls.ts:108 -->
- Menu rows are `button[role="menuitemradio"]` with `aria-checked`. Text strategy can land on the wrong row ("Auto" and "Ask each time" both match short text), so click by exact `textContent` in JS. <!-- src: src/components/OptionMenuList.tsx:106 -->
- On an ended session, the triggers are `disabled`: `interactive` is false, so no menu opens. <!-- src: src/features/session/SessionChat.tsx:1484 -->
- While Claude plans, the pill is pinned to `Plan` and can't be clicked. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:54 -->
- `Ultracode` is disabled unless the model supports xhigh. <!-- src: src/features/session/SessionChat.tsx:789 -->
- Model labels come from the live `claude` probe, so `Haiku 4.5` and `Sonnet 5` are fallback labels. Read the menu before matching.
- Don't trust the agent's own answer about which model it is. Read `list_active_sessions`.
