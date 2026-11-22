# Send a message in an agent session

A user types a prompt into an Agent tab's composer and clicks the send arrow. The app starts a session on the tab's harness and model. The transcript shows the prompt, the streamed reply and one row per tool call, and the activity line above the composer says what the agent is doing. Once the session is running, the same button sends follow-ups, or queues them while a turn is open. `Stop` interrupts the turn and kills the harness if it hasn't stopped within 3s. An ended session offers `Resume` and `New session`, and `Clear context` ends the session and empties the tab.

## Sub-features

- `send-start` starts a session from an unstarted tab.
- `send-followup` sends a second message to the running session.
- `send-queue` queues a message mid-turn as a dashed `Queued` row, which can be cancelled.
- `send-tool-rows` shows tool rows and folded tool groups, and expands them to show input and result.
- `send-stop` interrupts a running turn.
- `send-ended` shows the `Session ended.` bar with `Resume` and `New session`.
- `send-clear` clears context, ending the session and resetting the tab.

## How to get to it (user POV)

- Open a workstream. Its first tab is `Agent`. <!-- src: src/features/session/workstreamTabs.ts:101 -->
- Type in the composer at the bottom, then click the arrow button or press Enter (Shift+Enter adds a newline). <!-- src: src/features/session/SessionChat.tsx:1247 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance seeded with `verify-demo`, plus the `Verify map seed` folder workstream from [workstream-create.md](./workstream-create.md).
- The `opencode` binary is installed: `get_bootstrap` returns a `binaries` entry with `kind: "opencode"` and `state.state === "ok"`. <!-- src: src/lib/bindings.ts:618 -->
- **Cost: $0** on an OpenCode Zen `opencode/*-free` model, but only with tab titles and recaps turned off. Both default to on with `claude:haiku`, so each started session would otherwise make one paid Claude call for its title and one per recap. <!-- src: crates/hyprlayer-core/src/settings.rs:149 --> <!-- src: crates/hyprlayer-core/src/settings.rs:188 -->
- Seed the free path. A fresh profile has an empty `selectedModels`, so the OpenCode model group is empty. Raw IPC doesn't refresh `bootstrap`, so reload afterwards. <!-- src: src/features/session/SessionChat.tsx:471 --> <!-- src: src/features/session/SessionChat.tsx:483 -->
  ```js
  const inv = window.__TAURI__.core.invoke;
  const s = await inv('get_app_settings');
  const cat = await inv('opencode_catalog', { force: true });
  const free = (cat.providers.find(p => p.id === 'opencode' && p.available !== false)?.models ?? [])
    .map(m => `opencode/${m.id}`).filter(id => id.endsWith('-free'));
  s.providers = { ...(s.providers ?? {}), defaultHarness: 'opencode',
    opencode: { ...(s.providers?.opencode ?? {}), selectedModels: free.slice(0, 3) } };
  s.tabTitles = { enabled: false, model: null };
  s.recaps = { enabled: false };
  await inv('set_app_settings', { settings: s });
  location.reload();
  ```
  The model button in the status bar then reads `Model: <free model name>`. <!-- src: src/features/statusbar/SessionControlsCluster.tsx:352 -->

- **Open the tab.** Click the `Verify map seed` text in the sidebar. `[role="tab"][aria-selected="true"]` reads `Agent`. <!-- src: src/routes/ProjectPage.tsx:1099 -->
- **Type.** Use `webview_keyboard` `type` on `[data-pane-content] textarea` with `Reply with exactly VERIFY-SEND-<nonce>`. `button[aria-label="Start"]` becomes enabled. <!-- src: src/routes/ProjectPage.tsx:1730 --> <!-- src: src/features/session/SessionChat.tsx:1654 -->
- **Start.** Click `button[aria-label="Start"]`. The activity line reads `Connecting to OpenCode…`, then `Writing…` or `Working…`. The prompt shows as a user row. Empty `div[role="status"]` live regions come before the activity line, so read the first one with text: `[...document.querySelectorAll('div[role="status"]')].find(e => e.textContent.trim())`. <!-- src: src/features/session/ActivityBar.tsx:28 --> <!-- src: src/features/session/activityStatus.ts:141 -->
- **Turn end.** Poll until `button[aria-label="Send"]` exists (it reads `Queue message` while the turn is open) and the transcript contains the nonce. <!-- src: src/features/session/SessionChat.tsx:1654 -->
- **Read back.** `list_workstreams` (`{ projectPath, includeArchived: false }`) gives the workstream's `item.id`. `list_sessions` (`{ workstreamId }`) returns one row with `harness: "opencode"` and `stage: "general"`. `get_session_log` (`{ sessionId }`) has a `userMessage` event containing the nonce and a later `turnDone`. <!-- src: src/lib/bindings.ts:151 --> <!-- src: src/lib/bindings.ts:152 -->
- **Follow-up.** Type a second nonce and click `button[aria-label="Send"]`. The log gains a second `userMessage` and a second `turnDone`.
- **Long turn (for queue, tool rows and stop).** Send `Use your shell tool to run: python3 -c 'import time; time.sleep(40)' then reply DONE`. A bare `sleep` is blocked by some CLIs, so use the python form.
- **Queue.** While that turn runs, type `VERIFY-QUEUE` and click `button[aria-label="Queue message"]`. A dashed row reads `Queued` with `button[aria-label="Cancel queued message"]`. Clicking cancel removes it. If you leave it, it's delivered as a normal user row after the turn ends. <!-- src: src/features/session/TranscriptView.tsx:219 --> <!-- src: src/features/session/TranscriptView.tsx:225 -->
- **Tool rows.** The shell call renders as `[data-tooluse-id] > [role="button"]` with `aria-expanded="false"`. Clicking it sets `true` and shows the input `<pre>` and the result. If the call sits in a folded group, first click the group row (`N tool calls`, or `read N files`/`searched for …` for an explore run). <!-- src: src/features/session/TranscriptView.tsx:454 --> <!-- src: src/features/session/TranscriptView.tsx:456 --> <!-- src: src/features/session/TranscriptView.tsx:1141 -->
- **Stop.** Mid-sleep, click `button[aria-label="Stop"]`. Within ~3s the Stop button is gone. If the interrupt didn't settle, the harness is killed and the `Session ended.` bar shows. The log ends with a `turnDone` or an `exited` event. <!-- src: src/features/session/SessionChat.tsx:1627 --> <!-- src: src/features/session/sessionStore.ts:1934 -->
  - **Known bug (OpenCode, seen once on `beam-free`):** the log gets `turnDone` (`turnStatus: "interrupted"`) and then the aborted call's `toolResult`. That late result sets `streaming` back to true, so the tab stays `Working…` with Stop showing. A second Stop kills the harness and shows `Session ended.` Report it, don't count it as a pass. <!-- src: src/features/session/transcript.ts:411 -->
- **Ended.** On `Session ended.`, click `Resume` (`strategy: "text"`). The composer returns, and `list_active_sessions` lists the session. `New session` instead resets the tab to `Start`. On a failed start, Resume retries the launch instead; see session-failed-start.md. <!-- src: src/features/session/SessionChat.tsx:1454 --> <!-- src: src/features/session/SessionChat.tsx:1458 --> <!-- src: src/features/session/SessionChat.tsx:1487 -->
- **Clear context.** Click `button[aria-label="Clear context"]`. While the session is live, a dialog titled `Clear context?` opens; on an ended session the tab resets with no dialog. <!-- src: src/features/session/SessionChat.tsx:1182 --> Click `Clear` (`strategy: "text"`). The transcript empties and `button[aria-label="Start"]` is back. The `list_sessions` row is no longer live (state `killed` or `completed`). <!-- src: src/features/session/SessionChat.tsx:1607 --> <!-- src: src/features/session/SessionChat.tsx:1188 --> <!-- src: src/features/session/SessionChat.tsx:1190 -->
- **Proof.** Screenshot the typed prompt, the streaming activity line, the finished reply, the queued row, the expanded tool row and the ended bar. Save the `list_sessions` and `get_session_log` JSON.

## Gotchas

- `Stop` first withdraws any queued messages back into the composer, then interrupts the turn. <!-- src: src/features/session/sessionStore.ts:1941 -->
- A server restart during a turn no longer just ends the session: the server relaunches it with `--resume` and posts one `[from hyprlayer]` turn (#476). Don't restart the server mid-recipe, or the log gains turns you didn't send.

- The composer `<textarea>` has no label, placeholder or testid. Scope it with `[data-pane-content] textarea`, or `[data-pane="p1"] [data-pane-content] textarea` once panes are split. <!-- src: src/features/session/SessionChat.tsx:1561 -->
- Click `Start`/`Send` rather than pressing Enter. Whether bridge keystrokes reach the textarea's `onKeyDown` isn't established. When the `/` or `@` autocomplete is open, Enter picks the suggestion instead of sending. <!-- src: src/features/session/ComposerAutocomplete.tsx:296 -->
- Mid-turn, Enter and the button both queue. ⌘Enter (Ctrl on other platforms) steers the message into the agent's next step instead, and its row reads `Steering`. <!-- src: src/features/session/SessionChat.tsx:1250 -->
- Streamed text is revealed by a `requestAnimationFrame` loop in 33ms steps. If the window is occluded, the visible reply lags the store, so read the text from `get_session_log`, not the DOM. <!-- src: src/features/session/TranscriptView.tsx:248 -->
- `webview_wait_for` text matches stale transcript text. Use a fresh nonce per step.
- The send button is also disabled while a question is pending, while a `/prompt` still needs its argument, and while the sync guard holds sends. <!-- src: src/features/session/SessionChat.tsx:1635 -->
- When a tab first sees an ended session, it resumes it once on its own, spawning a fresh harness. Reopening an ended tab isn't a no-op. A failed start (`spawn_failed`) waits for Resume instead (session-failed-start.md), and sessions ended by archiving the workstream stay idle after a restore. <!-- src: src/features/session/transcript.ts:269 --> <!-- src: src/features/session/SessionChat.tsx:684 -->
- Picking a `-free` id from the catalog doesn't prove it's served. If the first turn ends in an auth or provider error row, put the next id first in `selectedModels` and retry.
- The Stop button renders only while `busy && started`, so it needs a turn that's still open. <!-- src: src/features/session/SessionChat.tsx:1622 -->
