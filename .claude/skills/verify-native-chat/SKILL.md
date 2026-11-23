---
name: verify-native-chat
description: Model-check the native chat (composer) session protocol with TLA+: user messages reaching claude exactly once, permission asks clearing, turns ending, interrupts, server-restart wakes, and a desktop converging on the session's event log. Use before merging any change to claude/driver.rs (Inbox, send_user_turn, respond_permission, interrupt, write_loop, read_and_wait), claude/protocol.rs's control lines, runtime/sessions/service.rs (send, send_app_turn_held, respond_permission, interrupt, launch_driver, spawn_relay, list_turns), forwarder.rs (handle_event, author_for, close_turn, attach_after, register_after), delivery.rs, wake.rs, store/sessions.rs's settle_stale_sessions, WorkstreamService::archive, hyprlayer-server's forward_session, or src/features/session's sessionStore.ts (dispatch, applyEvent, handedBackBy, deliverMessage, steerMessage, stopSession, pullNow), transcriptSync.ts (route, mergeEntries) and transcript.ts's ask handling, and whenever a message is reported lost or sent twice, an ask will not clear, a session stays working, or a restart wakes a session it should not.
---

# Verify the native chat

Three TLA+ models, each checked by TLC over every order of its steps, up to the
bounds in its `.cfg`:

- `SessionTurns.tla` covers one claude session from the send to the screen: the
  service's sends, answers and interrupts; `ClaudeDriver`'s inbox (`busy`,
  `lifecycle`, held echoes), `write_loop` and `read_and_wait`; the claude
  process (its stdin, its queue of messages sent mid-turn, its asks, its
  exit, and the `control_cancel_request` and tool result it writes when it
  settles an ask itself); `spawn_relay` and the forwarder actor; and one
  desktop's `sessionStore` (direct sends, steered chips, the hand-back on
  exit, asks closed by an answer or by their tool's result, reloads, resume).
- `RestartWake.tla` covers one session cut off mid-turn by server restarts:
  `settle_stale_sessions` queuing its wake, `wake::check` and `wake_one`, the
  wake turn's echo on its way to the log, and archiving the workstream.
- `SessionSync.tla` covers one desktop following one session's log: the actor
  storing events under consecutive seqs, `LiveAttachment::deliver` and its
  `Lagged` slot, `forward_session`'s re-attach, and the desktop's
  `route`/hold/flush, `pullNow` pages, `mergeEntries` and `trimHeld`.

## The rules

| Rule | In one sentence | The harm when it breaks |
|---|---|---|
| `SessionTurns.NoLostUserMessage` | A message you send from the composer and the server accepts is taken in by claude, still on its way, back in your queue to resend, reported failed, or at least stored in the transcript. | Your message vanishes: the send said OK, claude never saw it, and a reload drops it from the screen. |
| `SessionTurns.NoLostAgentMessage` | The same for a message an agent or a stage follow-up sends (`SessionService::send` with an agent author: `message_agent`, an agent wake; `send_app_turn`), with one more way to be kept: recorded in `held_messages` for the next relaunch, or its sender told it was not delivered. Goal iterations and restart wakes (`send_app_turn_held`) are not in it: their runners send again themselves. | An agent's `message_agent` returns OK and the message is gone; nothing retries it. |
| `SessionTurns.NoDoubleSend` | Claude takes in no message twice, across an exit, the hand-back and the resend. | Claude acts on one instruction twice. |
| `SessionTurns.TurnProgress` | Once nothing is in flight, the actor is not `in_turn`, the desktop does not show working, and a live driver's inbox is idle. | "Working" forever after the harness finished or exited; the next send is held behind a turn that is over. |
| `SessionTurns.AskResolution` | Once nothing is in flight, every ask the desktop shows open, live or after a reload, is one claude still waits on. | "Needs approval" stays after the turn ended, and the composer queues your next message instead of sending it (`isBusy`). |
| `SessionTurns.InterruptSafe` | After an interrupt, a live session settles idle: no turn, no held echo, no open ask. | Stop leaves the session looking busy, so the next message does not go out. |
| `RestartWake.WakeBounded` | A session cut off mid-turn is woken at most `MAX_RESTART_RESUMES` (3) times before it finishes a turn. | A crash loop: every restart wakes the session again. |
| `RestartWake.NoWakeWhenArchived` | No restart wake starts a harness in an archived workstream. | An archived workstream's agent comes back and works in a worktree the user put away. |
| `SessionSync.NoSkippedSeq` | The desktop never folds in a seq with an earlier one missing. | A turn silently missing from the transcript. |
| `SessionSync.ClientsConverge` | Once the log stops and every frame and pull has landed, the desktop holds every stored seq, or still holds another device's items behind the "behind" strip. | The desktop never catches up after a lag, short of a reload. |
| `*.TypeOK` | The variables stay in their domains. | None, it only guards the model. |

## Run it

From the repo root:

```sh
.claude/skills/verify-native-chat/check.sh
```

It needs `java` (11+) and `mise`. It installs TLC 1.7.4 through mise, pinned by
checksum (set `TLA2TOOLS=/path/tla2tools.jar` to skip mise), and runs TLC once
per rule with `WORKERS` threads (default 4; nexus is shared, so keep it low
there, or run it on the Mac). TLC's deadlock check is on: each model's `Done`
action marks its real end states. It prints `PASS`/`FAIL` per rule, compares
the list with `expected.txt`, and exits non-zero when they differ. About 20 s in
all with 4 workers: `SessionTurns` has 1.71 M distinct states, `SessionSync`
58 k, `RestartWake` about 500. `OUT=dir` keeps the logs: `$OUT/<Model>.<Rule>/tlc.out`.
`check.sh SessionTurns` checks one model and skips the comparison.

**Read every result with the pstack `/statechart` skill.** Give it the spec, its
`.cfg`, and the view module beside it (`SessionTurns-view.ts`,
`RestartWake-view.ts`, `SessionSync-view.ts`), which labels the states and steps
in the user's words. Add a label there for any new action. It prints each broken
rule as its shortest path in plain words. Use it for the report, not as the
verdict: past the first break it bounds each run at 100,000 states and lists
`SessionTurns`' passing rules as not checked. `check.sh` gives the verdict.

## Known findings (the baseline in `expected.txt`)

Two rules fail on the code at the sync commit. Each is a product bug, not a
modeling error: each path below was walked step by step against the code; none
was replayed in a running server by this skill. Three findings were fixed and
are kept for the record.

1. **Fixed: `AskResolution`, `InterruptSafe`: an ask claude settled itself reopened on a reload.**
   Claude writes `control_cancel_request` for a `can_use_tool` it settles
   without us (answered on claude.ai over Remote Control, allowed by a hook,
   cut off by an interrupt), then the tool's result. Until #520,
   `parse_line` made the cancel a `System` line and `pending_permissions`
   kept the id for the life of the process. The live card closed on the
   tool's result (`settleAnsweredPrompt`), but every later tail read named
   the id in `open_request_ids`, so `list_turns` re-sent the ask and
   `settleUnwaitedAsks` kept it: the reopened session showed "needs
   approval", `isBusy` held back your next message, and the composer's stop
   could not decline an ask claude no longer had. Shortest path (18 steps):
   you send, someone presses stop, claude asks, reads the interrupt,
   withdraws the ask and ends the turn; the desktop shows the ask, then its
   result, then `turnDone`; you reload and the ask is back. #520 (56dd7016)
   parses the cancel as `ParsedLine::ControlCancel` and drops the id from
   `pending_permissions`. The model follows the fixed code and passes; with
   the cancel ignored again it fails both rules.
   An earlier version of this skill, without tool results in the model,
   said the live view stayed stuck too. It does not: real claude 2.1.330
   writes the tool's result right after the cancel (the interrogate
   workstream's trace), and that closes the card.
2. **`NoLostUserMessage`: a direct send can vanish when claude exits.**
   Still open after the `held_messages` fix: a desktop's sends stay out of
   it, because a steered chip is resent by the desktop itself and a second
   copy would break `NoDoubleSend`. The `cancelled` row `read_and_wait` now
   writes for a held direct send names its `clientMessageId`, but
   `deliverMessage` keeps no chip, so nothing hands it back.
   `deliverMessage` sends with no chip, so nothing hands the message back on
   `exited`. Two paths:
   - The exit window (the shortest, 4 steps): claude exits; `read_and_wait`
     broadcasts `Exited`; you send before `spawn_relay` forwards it and calls
     `registry.remove`. `require_live` still finds the driver,
     `send_user_turn` broadcasts the echo behind `Exited`, and `write_line`
     returns Ok because `write_loop`'s receiver is still open (its write then
     fails). The relay forwards `Exited` and stops, so the echo is never
     stored. The window runs from `Exited` to `registry.remove`, normally
     milliseconds.
   - The held echo (5 steps, with the window closed in a scratch copy): an
     agent's message starts a turn and the driver reads its lifecycle frame;
     your view still shows idle, so you send directly; the inbox is not
     `idle()` and `lifecycle` is set, so the echo waits in `Inbox.waiting`;
     claude exits before its next step. No echo, no chip, no error: the
     optimistic row stays until a reload drops it. A steered message
     (`steerMessage`) is safe here: its chip goes back to the queue on
     `exited` and is resent.
3. **Fixed: `NoLostAgentMessage`: an agent's message to a session whose claude exits before taking it in was lost.**
   The same two paths, and no sender retries: `message_agent` and agent wakes go
   through `SessionService::send`, restart wakes and follow-ups through
   `send_app_turn_held`, and none keeps a copy to resend. The common case is
   the held echo: an agent messages a session mid-turn, then the session is
   stopped, crashes or is killed before claude's next step.
   Fixed by recording each such message in `held_messages` (`held::hold`,
   from `SessionService::send` for an agent author and from `send_app_turn`)
   until the relay sees its echo (`held::release`). After a crash or kill the
   relay tells each sender it can (`message_agent`'s caller) with a wake turn
   ("Your message to session … was not delivered: the session exited before
   reading it") and forgets those; a message with no one to tell stays. A
   relaunch (`launch_driver` with `resume`, including a server restart's
   wake) sends what is left once more, in order (`held::resend`); one
   already resent is reported instead. Ending a session quietly (archive)
   and the restart sweep (`sweep_after_restart`, for sessions no wake
   relaunched) report and forget the rest. `read_and_wait` also declines
   each echo still waiting when the process ends (`commandLifecycle`
   `cancelled`). e2e: `agent_spawn_and_message`, steps "a message its
   harness held when it crashed goes back to its sender" and "the restart
   resends the held message once".
4. **Fixed: `NoWakeWhenArchived`: a wake checked before an archive still resumed the session.**
   `wake::check` reads the workstream's status once (wake.rs:163), and
   `wake_one` re-reads neither it nor `ended_at`; `archive`'s
   `end_quietly_and_await_exit` does nothing for a session with no live
   harness. Path at 3e779f61: the server restarts, the waker checks the
   session, you archive the workstream, the waker resumes claude and sends
   the wake turn. #517 (08e0c131) fixed it: `launch_driver` now takes the
   workstream's lock and refuses an archived workstream (`hold_unarchived`),
   and archive holds the same lock while it stops sessions and writes the
   status. The model follows the fixed code and passes.
5. **`WakeBounded`: a restart before the wake turn is stored does not count.**
   `unanswered_wakes` counts stored restart-wake `UserMessage` rows since the
   last `TurnDone`. A wake whose echo is still between `send_user_turn` and the
   actor when the server stops again is not stored, the row is `Running` again
   (`launch_driver` set it), and its last turn kind is still the cut-off
   `userMessage`, so `settle_stale_sessions` queues another wake. Four
   restarts that each land in that window wake the session a fourth time.
   The window is the relay and the actor's write, normally milliseconds, so
   this needs a server that dies right after each wake.

When one of these is fixed in the code, change the model to match, run
`check.sh`, and change that rule's line in `expected.txt` to `PASS`.

## Does a change touch the verified code?

Run this from the repo root:

```sh
git diff --stat 07967f19234db6c95c9b6ceff6419b038a008ebd -- \
  crates/hyprlayer-core/src/claude/driver.rs \
  crates/hyprlayer-core/src/claude/protocol.rs \
  crates/hyprlayer-core/src/runtime/sessions/service.rs \
  crates/hyprlayer-core/src/runtime/sessions/forwarder.rs \
  crates/hyprlayer-core/src/runtime/sessions/delivery.rs \
  crates/hyprlayer-core/src/runtime/sessions/wake.rs \
  crates/hyprlayer-core/src/store/sessions.rs \
  crates/hyprlayer-core/src/runtime/workstreams.rs \
  crates/hyprlayer-server/src/streams.rs \
  src/features/session/sessionStore.ts \
  src/features/session/transcriptSync.ts \
  src/features/session/transcript.ts
```

Anything listed means: read the diff against the tables below. A change to
event content, prompts, titles, attribution, partial-text coalescing,
elicitations (declined in `read_and_wait`), the order of queued questions, the
`rev`/reservation arithmetic, codex or opencode does not move the models. A
change to when an echo is emitted or held, what `write_line` returns, what
`read_and_wait` emits for a line, when the relay ends or the registry drops a
driver, what opens or closes `in_turn`, which events hand chips back, how an
ask is opened, closed or reloaded, what `check` or `wake_one` read, how a lag
re-attaches, or how the desktop skips, holds or merges seqs does move them.

`SessionTurns.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `EchoOrHold`, `SendServer` | `ClaudeDriver::send_user_turn`, driver.rs:304 (`Inbox::idle`, 77; `lifecycle`; echo before `write_line`, 591; `take` on Err) |
| `WriteOk` | `write_line` → `stdin_tx`, and `write_loop`, driver.rs:609 (breaks on the first failed write) |
| `ClientSend` | `sendMessage`/`deliverMessage` (direct, optimistic row), sessionStore.ts:1480/1543; `steerMessage` (chip with a `clientMessageId`), 1607; the resend of a handed-back chip by `flushQueuedMessage`; server side `SessionService::send`, service.rs:271 |
| `AgentSend` | `SessionService::send` with an agent author (agent_control/wake.rs, `message_agent`), `send_app_turn` (follow-ups), each recording the message with `held::hold` before `send_user_turn` and releasing it on Err |
| `ClientAnswer` | `respond_permission`, service.rs:403 → driver.rs:371 (emits `PermissionResolved` only when `pending_permissions` held the id); a card is answered once |
| `Interrupt` | `SessionService::interrupt`, service.rs:426 → driver.rs:426; `stopSession`, sessionStore.ts:1945 |
| `ClaudeRead`, `ClaudeAsk`, `ClaudeTakeQueued`, `ClaudeRemoteAnswer`, `ClaudeFinish`, `ClaudeStartQueued`, `ClaudeExit` | claude's own behaviour on stream-json (see the assumptions below) |
| `DriverRead` | `read_and_wait`, driver.rs:634 (`Init` 657, `CommandLifecycle` → `Inbox::frame` 94, `Result` 674, `CanUseTool` 696, `ControlCancel` 710, `Exited` 763); `control_cancel_request` → `ParsedLine::ControlCancel`, protocol.rs:207; a tool result passes through as `ToolResult`; at the end of stdout it declines every echo still in `Inbox.waiting` before `Exited` |
| `Relay` | `spawn_relay`, service.rs (its reader stops at `Exited`; its forwarder releases a held message on its `UserMessage`, sends each event on, and after a reportable `Exited` calls `registry.remove` then `held::report_undelivered` for the messages whose sender can be told); `ActorState::handle_event`, forwarder.rs:856 (`author_for` 1056, `close_turn` 1124); the desktop's `applyEvent`, sessionStore.ts:452 (`handedBackBy` 527), `reduceEvent`'s `permissionAsk`/`permissionResolved` (`closeAsk`, transcript.ts:1011), `toolResult` (772, `settleAnsweredPrompt` 403), `turnDone` (853, `parkedState` 351), `exited` (867) |
| `ClientReload` | `SessionService::list_turns`, service.rs:815 (re-sends the stored asks `pending_requests` names; `open_request_ids`), `settleUnwaitedAsks`, transcript.ts:1031 |
| `ClientResume` | `ResumeSession` → `launch_driver`, service.rs (fresh driver and inbox, `Relaunched`, then `held::resend`: each held message once more, in order; one already resent is reported) |

`RestartWake.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `Restart` | server stop (harnesses die, the actor with them) and start: `settle_stale_sessions`, store/sessions.rs:687 (`is_working`, 264), from runtime/mod.rs:243; `spawn_after_restart`, wake.rs:28 |
| `WakerCheck` | `wake_stopped` → `check`, wake.rs:49/155 (ended, archived workstream at 166, `unanswered_wakes` 300 against `MAX_RESTART_RESUMES`) |
| `WakerWake` | `wake_and_forget` → `wake_one`, wake.rs:192 (`resume_if_exited_held`; `launch_driver` holds the workstream lock and refuses an archived one, `hold_unarchived`, workstreams.rs:1845, then sets `Running` and deletes the pending wake, service.rs:1747/1808; `send_app_turn_held`) |
| `StoreEcho` | the wake's `UserMessage` through `spawn_relay` to `handle_event` |
| `ClaudeFinishes` | a `TurnDone` row |
| `Archive` | `WorkstreamService::archive` → `archive_inner`, workstreams.rs:1836/1857, under `lock_pair` (`stop_for_archive`, 2063: `end_quietly_and_await_exit`, 2081 → service.rs:882), then the status |

`SessionSync.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `Store` | `handle_event`'s persist then fan-out, forwarder.rs:856; `LiveAttachment::deliver`/`try_deliver`/`mark_lagged`, delivery.rs:130/108/119 |
| `Forward` | `forward_session`, streams.rs:1411 (the `Lagged` arm at 1443 → `attach_after`, forwarder.rs:571, and `register_after`, 626, in one actor turn) |
| `Receive` | `dispatch`, sessionStore.ts:355 → `route`, transcriptSync.ts:267 (skip, hold, flush via `applyHeld` 427, apply) |
| `PullRead`, `PullApply` | `pullNow`, sessionStore.ts:1728 → `list_turns`; `mergeEntries`, transcriptSync.ts:352; `trimHeld`, sessionStore.ts:444 |

Granularity: an action is one step only where the code makes it one. That
means a service call under the session's op lock with the driver's inbox mutex
(`send_user_turn` decides the echo and writes in one hold), one line read by
`read_and_wait`, one event forwarded by the relay and handled by the actor
(the actor's fan-out and the desktop's fold are merged into that step; no rule
needs the desktop to lag the actor, and it still lags claude through `out` and
`bus`), and one actor turn (`attach_after` reads the log and registers the new
attachment together).

## Bounds and what the models leave out

- `SessionTurns`: one composer message (sent at most twice: once, and once
  more after a hand-back), one agent message, two ask ids, two claude
  processes (one exit and one resume), one interrupt. `RestartWake`:
  `MAX_RESTART_RESUMES = 3`, five restarts. `SessionSync`: five events, an
  attachment queue of one plus the `Lagged` slot, pages of two rows, two lags.
- Assumptions about claude 2.1.330 that the code relies on and the model takes
  as given: claude sends a `command_lifecycle` `started` frame for every
  message it takes in, including one read while idle (not checked); it keeps
  queued messages across an interrupt and starts one right after a `result`
  (not checked); it writes a tool result for every ask once it is answered,
  denied or withdrawn, and an interrupt or a Remote Control answer withdraws
  an open ask with `control_cancel_request` first (both seen against real
  claude by the interrogate workstream).
- `held_messages` in `SessionTurns`: every exit is a crash, so a detach
  (server stop, `kill_for_relaunch`) keeps all held messages as a crash keeps
  the untold ones; `toldSet` picks at Init which agent messages have a sender
  to tell. Left out: an agent send that passes `require_live` before the relay
  drops the driver and records its message after the relay's report (the
  message stays in `held_messages` until the next relaunch or restart sweep),
  a second resend (bounded by two processes), and the restart sweep.
- Left out of `SessionTurns`: relay lag (since #521 `spawn_relay` drains the
  broadcast into an unbounded buffer, so it lags only if that reader falls
  `EVENT_CHANNEL_CAP` behind; the model's relay never lags), subagent asks,
  another desktop and its holds, questions as their own kind,
  `cancel_queued_message`, a desktop reconnecting after a server restart
  (`settleFolded` hands its chips back since #524), rewind, `kill_for_relaunch` and the budget-cap
  relaunch, follow-ups' own logic, reservations and `StaleTranscript`, a
  failed persist in the actor, codex and opencode, and an interrupt landing on
  the turn that starts right after the one it was meant for (the composer's
  `stopSession` withdraws queued chips first and accepts the rest by design).
- Left out of `RestartWake`: orphan wakes (`queue_orphan_wakes`, mid-turn off,
  with open background tasks), the overlay wait and concurrency (a free delay
  between check and wake stands in for them), goals (the goal resumer's
  sessions), the session's own `archived_at` (open to the same race), and
  claude crashing during a wake.
- Left out of `SessionSync`: rewinds and epochs, unpersisted partial text and
  `drop`, the sync state's `rev` arithmetic behind the "behind" strip
  (`ClientsConverge` accepts held items as "the strip shows" without checking
  `transition`/`settle`), `replay` user messages, and a server without seqs.
- A PASS covers these bounds and assumptions only.

## Real bug or modeling error?

For each FAIL, take its trace (`/statechart` gives it in words, or
`$OUT/<Model>.<Rule>/tlc.out`) and walk it action by action through the code
in the tables above. If every step is possible in the code, it is a bug.
Report it with the path in the code's words. Do not weaken the rule or the
model to make it pass. If a step is impossible in the code (the code holds a
lock the model does not, or emits an event the model forgets), the model is
wrong: fix the action, re-run, and say what you changed.

## When a change is intended

1. Change the actions that mirror the changed code, keeping one action per
   indivisible code step, and update the tables.
2. Prove the rule you touched can still fail. Break that code path in a copy
   of the model and its `.cfg` in a scratch directory and run
   `SRC=<that dir> check.sh <Model>`; see it report the break, then delete the
   copy. Breaks that were each confirmed to fail their rule:
   - `NoDoubleSend`: `Relay` leaves the chip in place on its echo (the
     `settleDelivered` call gone), so an exit hands back a message claude
     already took in.
   - `TurnProgress`: `Relay` closes `inTurn` only on `exited`, not `turnDone`
     (`close_turn` dropped for `TurnDone`).
   - `NoSkippedSeq`, `ClientsConverge`: `Forward`'s replay starts at
     `f[2] + 2` (an `attach_after` off by one), or `Receive`'s flush applies
     the own item without the held queue (`applyHeld` skipped), or `Receive`
     skips at `s <= applied + 1` (tried as a real one-line change to `route`'s
     skip in transcriptSync.ts, mirrored in the model).
   - `NoWakeWhenArchived`: `WakerWake` launches without the `~wsArchived`
     test (`hold_unarchived` gone).
   - `AskResolution`, `InterruptSafe`: `DriverRead` leaves `pending` alone on
     a `cancel` line (the code before #520). Both fail on an 18-step path
     ending in a reload.
   - `NoLostAgentMessage`: `Relay` forgets the held messages on `exited`
     without telling their senders (a 5-step path: claude exits, an agent
     sends into the window before the relay drops the driver); or
     `ClientResume` resends nothing and keeps them held (`held::resend`
     gone).
   - `NoLostUserMessage`, `WakeBounded` fail today; once one passes, add a
     break for it here.
3. Run `check.sh`. Update `expected.txt` only for a rule whose verdict the
   change was meant to move, and say so in the PR.
4. Move the sync commit (the hash in the `git diff` command above and below)
   forward only after you have read the covered code at the new commit and
   confirmed the models describe it.

Last confirmed against: `07967f19234db6c95c9b6ceff6419b038a008ebd`.
