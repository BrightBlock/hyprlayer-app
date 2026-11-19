# Permission prompts and agent questions

When the agent wants a tool its permission mode doesn't allow, the transcript shows a `Permission required` card naming the tool and its file or command. The card offers `Allow once`, `Allow for this agent` and `Deny`. While it waits, the activity line reads `Waiting for approval`. A denied call collapses to `<tool> · denied`, and an allowed one disappears. When the agent asks the user a question, a card above the composer reads `The agent is asking`. It lists the options, with `Other…` for a typed answer, plus `Back`/`Next` for multi-question asks, `Submit answer(s)` and `Skip`. The composer is disabled until the question is answered.

## Sub-features

- `perm-card` renders the card for a pending tool call.
- `perm-allow-once` runs the call once.
- `perm-allow-session` allows the call and every later call to the same tool in this session, including parallel ones already waiting.
- `perm-deny` declines the call.
- `ask-single` picks an option and submits.
- `ask-other` types a custom answer.
- `ask-skip` dismisses the question with no answer.
- `ask-blocks-composer` disables the composer textarea while a question is pending.

## How to get to it (user POV)

- Run a Claude session in `Ask each time` mode and ask for something that writes a file. The card appears in the transcript.
- Ask the agent to ask you something. The question card sits above the composer.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance with the `Verify map seed` workstream ([workstream-create.md](./workstream-create.md)), on a fresh `Agent` tab in mode `General`. General launches Claude in default mode with only read-only tools pre-allowed, so `Write` asks. <!-- src: crates/hyprlayer-core/src/pipeline.rs:99 -->
- **Spends real Claude turns.** Use `Haiku 4.5` and `Ask each time` (session-pickers.md). Expect one turn per step, a few cents each. Turn off tab titles and recaps (session-send.md seed) or add one Haiku title call. No OpenCode path here: the OpenCode runtime forwards `permission.asked` and `question.asked`, but nothing in this repo makes OpenCode ask, and OpenCode has no permission-mode menu. <!-- src: crates/hyprlayer-core/src/opencode/runtime.rs:1138 --> <!-- src: crates/hyprlayer-core/src/opencode/runtime.rs:1155 --> <!-- src: src/features/session/SessionChat.tsx:1500 -->
- The seed folder `<SEED>` (the workstream's `worktreePath`) is where the agent writes.

- **Card.** Send `Use the Write tool to create perm-once.txt containing ok. Do nothing else.` Within one turn, the transcript shows `Permission required` with `Write` and a chip for `perm-once.txt`, and `div[role="status"]` reads `Waiting for approval · Write`. <!-- src: src/features/session/TranscriptView.tsx:570 --> <!-- src: src/features/session/activityStatus.ts:153 -->
- **Allow once.** Click `Allow once` (`strategy: "text"`). The card disappears and the turn finishes. `ls <SEED>` shows `perm-once.txt`. <!-- src: src/features/session/TranscriptView.tsx:584 -->
- **Deny.** Send the same prompt for `perm-deny.txt` and click `Deny`. The card collapses to `Write · denied`, and `perm-deny.txt` doesn't exist. <!-- src: src/features/session/TranscriptView.tsx:590 --> <!-- src: src/features/session/TranscriptView.tsx:561 -->
- **Allow for this agent.** Send `Use the Write tool to create a1.txt, then a2.txt, each containing ok.` Click `Allow for this agent` on the first card. No second card appears, and both files exist. <!-- src: src/features/session/TranscriptView.tsx:587 --> <!-- src: src/features/session/sessionStore.ts:1537 -->
- **Permission read-back.** In `get_session_log` (`{ sessionId }`), each card is a `permissionAsk` event whose `requestId` the answer used. The call that `Allow for this agent` approved without a card still logs its own `permissionAsk`, so the four steps above leave four. <!-- src: src/lib/bindings.ts:1927 -->
- **Question.** Send `Use the AskUserQuestion tool to ask me to choose between Red and Blue, then reply with only my choice.` A card reads `The agent is asking`, with `Red`, `Blue` and `Other…`. `[data-pane-content] textarea` is `disabled`, and the activity line reads `Waiting for your answer`. <!-- src: src/features/session/SessionChat.tsx:423 --> <!-- src: src/features/session/SessionChat.tsx:1734 --> <!-- src: src/features/session/activityStatus.ts:146 -->
- **Pick and submit.** Click `Blue` (`strategy: "text"`); its button gets `aria-pressed="true"`. Click `Submit answer`. The card goes away, and the reply is `Blue`. The log has a `questionAsk` and a later `assistantText` with `Blue`. <!-- src: src/features/session/SessionChat.tsx:449 --> <!-- src: src/features/session/SessionChat.tsx:536 -->
- **Other.** Repeat the ask, click `Other…`, type into `input[aria-label="Custom answer"]` (placeholder `Type your answer…`) `Green`, then click `Submit answer`. The reply is `Green`. <!-- src: src/features/session/SessionChat.tsx:523 --> <!-- src: src/features/session/SessionChat.tsx:500 -->
- **Skip.** Repeat the ask and click `Skip`. The card closes with an empty answer, and the composer is enabled again. <!-- src: src/features/session/SessionChat.tsx:539 -->
- **Proof.** Screenshot each card before and after the click. Save the `ls` output after allow and deny, and the log's `permissionAsk`/`questionAsk` events.

## Gotchas

- `Submit answer` stays disabled until an option is chosen or `Other…` has text. It reads `Next` until the last question of a multi-question ask. <!-- src: src/features/session/SessionChat.tsx:535 -->
- Enter in the `Custom answer` input advances. Clicking `Submit answer` is safer under the bridge. <!-- src: src/features/session/SessionChat.tsx:505 -->
- `Allow for this agent` only lasts for this session. A relaunch (a model or mode switch) clears it, so the next call asks again. <!-- src: src/features/session/sessionStore.ts:1071 -->
- `Stop` during a pending card declines it (or answers a pending question with nothing) before interrupting. <!-- src: src/features/session/sessionStore.ts:1574 -->
- In `Auto`, `Accept edits` or `Bypass permissions`, `Write` may not ask. Check the pill reads `Permission mode: Ask each time` before sending. <!-- src: src/lib/domain/permissions.ts:14 -->
- A card whose request the harness no longer holds renders `· expired` with no buttons. <!-- src: src/features/session/TranscriptView.tsx:551 -->
- Delete `perm-once.txt`, `a1.txt` and `a2.txt` before the next run. Cleanup removes the run dir anyway.
