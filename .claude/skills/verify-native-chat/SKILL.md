---
name: verify-native-chat
description: Model-check the native chat (composer) session protocol with TLA+ for each provider (claude, codex app-server, OpenCode): user messages reaching the harness exactly once, permission asks clearing, turns ending, interrupts, server-restart wakes, and a desktop converging on the session's event log. Use before merging any change to claude/driver.rs (Inbox, send_user_turn, respond_permission, interrupt, write_loop, read_and_wait), claude/protocol.rs's control lines, codex/driver.rs (send_user_turn, steer, interrupt, respond_permission, read_and_wait, route_incoming) or codex/protocol.rs's Translator (active_turn, requested_turn, steering, pending_approvals, turn/completed, item_completed), opencode/runtime.rs (OpenCodeDriver send_user_turn, respond_permission, interrupt, track_ask, the Translator's handle/expire, spawn_sse), runtime/sessions/service.rs (send, send_app_turn_held, respond_permission, interrupt, launch_driver, spawn_relay, list_turns), forwarder.rs (handle_event, author_for, close_turn, attach_after, register_after), delivery.rs, wake.rs, store/sessions.rs's settle_stale_sessions, WorkstreamService::archive, hyprlayer-server's forward_session, or src/features/session's sessionStore.ts (dispatch, applyEvent, handedBackBy, deliverMessage, steerMessage, stopSession, pullNow), transcriptSync.ts (route, mergeEntries) and transcript.ts's ask handling, and whenever a message is reported lost or sent twice, an ask will not clear, a session stays working, or a restart wakes a session it should not.
---

# Verify the native chat

Five TLA+ models, each checked by TLC over every order of its steps, up to the
bounds in its `.cfg`. The session turn protocol has one model per provider:
claude's stream-json over stdin with lifecycle frames, codex's JSON-RPC with
turn ids, and OpenCode's HTTP requests plus SSE events from one shared server
differ too much at the harness and driver for one model with an adapter. The
three share the service, relay, actor and desktop steps (copied, so they read
the same) and the same rule names, so each rule is checked per provider.

- `SessionTurns.tla` covers one claude session from the send to the screen: the
  service's sends, answers and interrupts; `ClaudeDriver`'s inbox (`busy`,
  `lifecycle`, held echoes), `write_loop` and `read_and_wait`; the claude
  process (its stdin, its queue of messages sent mid-turn, its asks, its
  exit, and the `control_cancel_request` and tool result it writes when it
  settles an ask itself); `spawn_relay` and the forwarder actor; and one
  desktop's `sessionStore` (direct sends, steered chips, the hand-back on
  exit, asks closed by an answer or by their tool's result, reloads, resume).
- `CodexTurns.tla` covers one codex app-server session: `CodexDriver`'s
  `send_user_turn` (a steer into the turn it knows of, else an echo and
  `turn/start`), `interrupt` (sends only for the turn `turn/started` named;
  before that the stop waits for it), `respond_permission`, and the
  Translator's `active_turn`, `requested_turn` (set from the `turn/start`
  response as the reader reads it), `pending_turn_starts`, `deferred_interrupt`, `steering` and
  `pending_approvals`; codex reading requests, answering a `turn/start` as it
  accepts the turn and beginning it (`turn/started`) a step later, refusing
  an interrupt until then, asking,
  committing steers at its next step, and settling its own approvals on an
  interrupt (`serverRequest/resolved` after `turn/completed`); and the shared
  relay, actor and desktop, where a codex ask names the call it gates
  (`tool_use_id`) and is closed by that call's result.
- `OpenCodeTurns.tla` covers one session on the shared OpenCode server:
  `OpenCodeDriver`'s `send_user_turn` (echo, `begin_turn`, `prompt_async`),
  `respond_permission`, `interrupt` (drains the asks map, marks the sent
  turn, POSTs abort); OpenCode handling each POST in any order, holding a
  prompt behind a running one, aborting, asking, another OpenCode client
  answering, dying, and SSE events the driver never sees; the filter task's
  `track_ask` and Translator turn ending (`sent`, anchor, `answered`,
  `interrupted`, the unplaced idle and its expiry); and the shared relay,
  actor (including `next_turn`) and desktop.
- `RestartWake.tla` covers one session cut off mid-turn by server restarts:
  `settle_stale_sessions` queuing its wake, `wake::check` and `wake_one`, the
  wake turn's echo on its way to the log, and archiving the workstream.
- `SessionSync.tla` covers one desktop following one session's log: the actor
  storing events under consecutive seqs, `LiveAttachment::deliver` and its
  `Lagged` slot, `forward_session`'s re-attach, and the desktop's
  `route`/hold/flush, `pullNow` pages, `mergeEntries` and `trimHeld`; and the
  actor's lifetime: the harness ending and resuming, the desktop closing and
  reopening the session, and an idle actor retiring.

## The rules

`SessionTurns` (claude), `CodexTurns` and `OpenCodeTurns` each check the
first eight; "the harness" is claude, codex or OpenCode.

| Rule | In one sentence | The harm when it breaks |
|---|---|---|
| `NoLostUserMessage` | A message you send from the composer and the server accepts is taken in by the harness, still on its way, back in your queue to resend, reported failed, or at least stored in the transcript. | Your message vanishes: the send said OK, the harness never saw it, and a reload drops it from the screen. |
| `NoLostAgentMessage` | The same for a message an agent or a stage follow-up sends (`SessionService::send` with an agent author: `message_agent`, an agent wake; `send_app_turn`), with one more way to be kept: recorded in `held_messages` for the next relaunch (while the session is not live and quiet), or its sender told it was not delivered. Goal iterations and restart wakes (`send_app_turn_held`) are not in it: their runners send again themselves. | An agent's `message_agent` returns OK and the message is gone; nothing retries it. |
| `NoDoubleSend` | The harness takes in no message twice, across an exit, the hand-back and the resend. | The harness acts on one instruction twice. |
| `TurnProgress` | Once nothing is in flight, the actor is not `in_turn`, the desktop does not show working, and a live driver thinks no turn runs. | "Working" forever after the harness finished or exited; the next send is held behind a turn that is over. |
| `AskResolution` | Once nothing is in flight, every ask the desktop shows open, live or after a reload, is one the harness still waits on. | "Needs approval" stays after the turn ended, and the composer queues your next message instead of sending it (`isBusy`). |
| `AskVisible` | While the harness waits on an ask and nothing is in flight, the desktop shows it. | The harness waits forever on an approval no one can see. |
| `SessionTurns.AskClosesOnAnswer` | Once the driver and relay have caught up, the desktop shows no ask claude stopped waiting on, even while the tool it allowed still runs. | An ask answered on claude.ai keeps its Allow/Deny card and "Waiting for approval" until the tool finishes, and the composer queues your next message (`isBusy`). |
| `SenderTold` | Once an exited session is quiet, no agent message whose sender can be told is still held untold. (`SessionTurns`, `CodexTurns`.) | `message_agent` returns sent, the harness had already exited, and the sender hears nothing until the next relaunch or restart sweep. |
| `SessionTurns.ResolvedOnce` | An ask's resolution reaches the log once, whoever answered it. | A card answered here and elsewhere at once records two answers. |
| `InterruptSafe` | After an interrupt, a live session settles idle: no turn, no held echo, no open ask. | Stop leaves the session looking busy, so the next message does not go out. |
| `InterruptHonored` | An interrupt is never dropped while a message sent before it is still to run (codex: while codex runs or is about to run a turn the driver has no id for). | You press stop and the turn runs to the end anyway. |
| `RestartWake.WakeBounded` | A session cut off mid-turn is woken at most `MAX_RESTART_RESUMES` (3) times before it finishes a turn. | A crash loop: every restart wakes the session again. |
| `RestartWake.NoWakeWhenArchived` | No restart wake starts a harness in an archived workstream. | An archived workstream's agent comes back and works in a worktree the user put away. |
| `SessionSync.NoSkippedSeq` | The desktop never folds in a seq with an earlier one missing. | A turn silently missing from the transcript. |
| `SessionSync.ClientsConverge` | Once the log stops and every frame and pull has landed, a desktop that has the session open holds every stored seq, or still holds another device's items behind the "behind" strip, including after it closed the session, its actor retired, and it opened the session again. | The desktop never catches up after a lag or a reopen, short of a reload. |
| `SessionSync.NoLostEvent` | No event a live harness emits reaches an actor that has retired. | A turn's rows vanish: the relay's send to a retired actor fails and nothing stores them. |
| `*.TypeOK` | The variables stay in their domains. | None, it only guards the model. |

Results at the sync commit, per provider (`expected.txt`):

| Rule | claude | codex | OpenCode |
|---|---|---|---|
| `NoLostUserMessage` | PASS | PASS | FAIL |
| `NoLostAgentMessage` | PASS | PASS | FAIL |
| `NoDoubleSend` | PASS | PASS | PASS |
| `TurnProgress` | PASS | PASS | FAIL |
| `AskResolution` | PASS | PASS | FAIL |
| `AskVisible` | PASS | PASS | FAIL |
| `SenderTold` | PASS | PASS | not modeled |
| `AskClosesOnAnswer` | PASS | not modeled | not modeled |
| `ResolvedOnce` | PASS | not modeled | not modeled |
| `InterruptSafe` | PASS | PASS | FAIL |
| `InterruptHonored` | PASS | PASS | FAIL |

In `OpenCodeTurns` a stored echo keeps a message only while OpenCode has not
left it unread in its history (finding 16): until the driver held a mid-turn
message's echo for the step that reads it, it stored every echo at send, and
the rule passed a message OpenCode never answered.

## Run it

From the repo root:

```sh
.claude/skills/verify-native-chat/check.sh
```

It needs `java` (11+) and `mise`. It installs TLC 1.7.4 through mise, pinned by
checksum (set `TLA2TOOLS=/path/tla2tools.jar` to skip mise, `JAVA=` to pick a
java), and runs TLC with `WORKERS` threads (default 4). One TLC run checks
every rule of a config still unresolved; breadth-first, it stops at the
shortest path that breaks any of them, that rule is FAIL, and TLC runs again
without it, so each FAIL's trace is its own shortest path and the passing rules
share one full run. TLC's deadlock check is on: each model's `Done` action marks
its real end states. It prints `PASS`/`FAIL` per rule, compares the list with
`expected.txt` (a `#` starts a note to the end of the line), and exits
non-zero when they differ.

Timing, all 51 rules: 64 s on the Mac with `WORKERS=12` (18 cores, measured
under a load average of about 20 from other runs; not re-measured on nexus,
where the 33 rules before took 51 s with `WORKERS=4`). Run it on the Mac
when you can (nexus is shared):
`/home/jt/.cache/mac-run.sh <worktree> <run-name> 'JAVA=<mise java> WORKERS=12
sh .claude/skills/verify-native-chat/check.sh'`; that shell has no mise shims on
`PATH`, so name the java binary under `~/.local/share/mise/installs/java/`.
Distinct states in each model's passing run: `SessionTurns` 10.0 M,
`CodexTurns` 0.72 M, `OpenCodeTurns` 2.9 M (`-stop` 2.2 k, `-death` 298),
`SessionSync` 58 k, `RestartWake` 251. `OUT=dir` keeps the logs:
`$OUT/<Model>.<Rule>/tlc.out` for each FAIL, `$OUT/<Model>.pass/tlc.out` for the
run every PASS shares. `check.sh CodexTurns` checks one model and skips the
comparison.

**Read every result with the pstack `/statechart` skill.** Give it the spec, its
`.cfg`, and the view module beside it (`SessionTurns-view.ts`,
`CodexTurns-view.ts`, `OpenCodeTurns-view.ts`, `RestartWake-view.ts`,
`SessionSync-view.ts`), which labels the states and steps
in the user's words. Add a label there for any new action. It prints each broken
rule as its shortest path in plain words. Use it for the report, not as the
verdict: past the first break it bounds each run at 100,000 states and lists
the turn models' passing rules as not checked. `check.sh` gives the verdict.

## Known findings (the baseline in `expected.txt`)

Findings 1 to 5 and 17 are claude's (`SessionTurns`) and the shared models'; 6 to
16 are codex's and OpenCode's. Each path was walked step by step against the
code. A provider FAIL counts as confirmed only once it has been reproduced live
in a debug instance; each says "reproduced live" (by whom, and the evidence) or
"model only". Findings 2 and 5 are fixed, and each was reproduced live
before and after its fix in a debug build with the window widened by a
scratch delay (the PR that fixed them has the evidence).

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
2. **Fixed: `NoLostUserMessage`: a direct send could vanish when claude exited.**
   `deliverMessage` sent with no chip, so nothing handed the message back on
   `exited`. Two paths:
   - The exit window (4 steps): claude exits; `read_and_wait` broadcasts
     `Exited`; you send before `spawn_relay` forwards it and calls
     `registry.remove`. `require_live` still found the driver,
     `send_user_turn` broadcast the echo behind `Exited`, and `write_line`
     returned Ok because `write_loop`'s receiver was still open. The relay
     forwards `Exited` and stops, so the echo was never stored.
   - The held echo (6 steps): an agent's message starts a turn and the
     driver reads its lifecycle frame; your view still shows idle, so you
     send directly; the inbox is not `idle()` and `lifecycle` is set, so the
     echo waits in `Inbox.waiting`; claude exits before its next step. No
     echo, no chip, no error: the optimistic row stayed until a reload
     dropped it.
   Fixed on both sides. `deliverMessage` keeps each direct send in the
   entry's `unechoed` list until its echo; `exited`, a `cancelled`
   `commandLifecycle` naming it, and a fold or pull that brings in an exit or
   relaunch newer than the send (`settleFolded`, by `heldAfterSeq`) take its
   optimistic row out and put the message back at the front of the queue.
   A live `relaunched` does not: it follows the exit that already handed
   them back, and a later one may be this send's own resume (the model finds
   a double send in 20 steps if it does). `read_and_wait` marks the inbox
   `ended` before it declines the held echoes, and `send_user_turn` then
   refuses with `session_not_found`, which `deliverMessage` turns into a
   queued message. The server keeps no copy of a desktop's send, so a
   steered or direct message is resent by the desktop only.
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
5. **Fixed: `WakeBounded`: a restart before the wake turn was stored did not count.**
   `unanswered_wakes` counted stored restart-wake `UserMessage` rows since
   the last `TurnDone`. A wake whose echo was still between `send_user_turn`
   and the actor when the server stopped again was not stored, the row was
   `Running` again (`launch_driver` set it), and its last turn kind was still
   the cut-off `userMessage`, so `settle_stale_sessions` queued another
   wake. Four restarts that each landed in that window woke the session a
   fourth time. Fixed by counting attempts on the session row:
   `wake_one` calls `record_restart_wake` after the resume and before
   `send_app_turn_held`, and `check` reads `unanswered_restart_wakes`, which
   counts from zero again once a `turnDone` is stored after the last
   recorded wake (`restart_wake_seq`).

6. **Fixed: codex `AskResolution`: an approval codex settles itself stayed open.**
   Reproduced live by the "Interrogate codex and opencode chat" workstream in
   the debug app with real codex: after an interrupt, "Turn interrupted" sits
   under a live Allow/Deny card, the status bar keeps "Waiting for
   approval" and Stop, `openRequestIds` keeps naming the ask, and a message
   queued after it never sends. `pending_approvals` lost an id only in
   `respond_permission`, and the Translator dropped the
   `serverRequest/resolved` codex 0.171 sends after `turn/completed` for a
   request it settled itself, so the card and every reload kept it.
   Shortest path (15 steps): an agent's message starts a turn, codex asks,
   someone presses stop, codex ends the turn with the approval settled; the
   desktop shows the ask after `turnDone`. Fixed: `serverRequest/resolved`
   removes the approval or question with that rpc id and emits
   `PermissionResolved {allow: false}` or `QuestionAnswered`; an ask the user
   already answered is gone and emits nothing. `InterruptSafe` failed on
   finding 8's stale turn id until that was fixed too.
7. **Fixed: codex `NoLostAgentMessage`: a steer codex never committed was dropped while the session stayed live.**
   The mechanism was reproduced live by the interrogate workstream (a steer
   codex drops on an interrupt is never resent or reported); the
   end-of-turn path below is model only. An agent's message to a busy session
   goes out as `turn/steer` with its echo in `steering` and the message in
   `held_messages`. If codex accepts it and then completes the turn before
   its next step, `turn/completed` runs `decline_steers`: a `cancelled`
   `CommandLifecycle`, and no echo. Nothing on the server reacted to it, so
   the agent's message stayed held, unread and untold, until the next
   relaunch (13 steps). A refused steer (`expectedTurnId` no longer running)
   declines the same way. The relay now keeps the ids cancelled during a
   turn and, once that turn's `TurnDone` has gone to the actor, sends each
   held one again as the next turn (`held::send_dropped`, `RelayResend`); a
   refused resend is reported. e2e: `agent_spawn_and_message`, step "a
   message its harness dropped when the turn ended is sent as the next
   turn". The rule passes with finding 8 fixed, and fails again (14 steps)
   with the resend removed; before finding 8's fix it still failed on that
   finding's path, where the stale `requested_turn` made the agent's first
   send a steer into a finished turn that codex refused with no `TurnDone`
   after it.
8. **Fixed: codex `TurnProgress`, `InterruptSafe`: a stale `requested_turn` (model only).**
   The `turn/start` task set `requested_turn` from the response whenever it
   ran, after any `turn/completed` the reader had already handled. The
   driver then believed a turn ran: the next message was sent as
   `turn/steer` into the finished turn and declined, and a stop waited for a
   `turn/started` that only the next turn sends. Shortest path (11 steps):
   you send, codex answers, starts and finishes the turn, the reader reads
   all of it, then the task runs. Not reproduced live: in the probe
   transcripts codex 0.171 wrote the response 4 to 33 ms before
   `turn/started` in all 26 turns and `turn/completed` came hundreds of ms
   or more later, so only the task's scheduling could open the window.
   Fixed: `observe_response`, which `route_incoming` calls for every
   response before its waiter, records the turn id (`turn_start_answered`)
   in the order codex wrote the lines; the task only reports a failed
   `turn/start`. A driver test reads the response, `turn/started` and
   `turn/completed` before the task runs.
9. **Fixed: codex `NoLostUserMessage` (model only).** The shortest path (4
   steps) was claude's exit window from finding 2: codex exits, `Exited` is
   broadcast, you send before the relay drops the driver; the echo trails
   `Exited`. Finding 8 added a codex-only path: a direct send steered into a
   stale turn is declined with no chip to hand back. The desktop side of
   finding 2's fix covers both, since `deliverMessage` and `handedBackBy` are
   the same for every harness: the unechoed send goes back to the queue on
   `exited` and on the decline (`declined_message` names its id for codex as
   for claude). `CodexDriver::send_user_turn` does not refuse after its
   stdout ends as claude's does, so codex's window still returns Ok; the
   desktop resends. Not replayed against a live codex.
10. **Fixed: codex `AskVisible`: another command's result closed the card.**
    Reproduced live on the Mac debug app (codex 0.170, gpt-6.2-sol): in Auto
    mode, of two parallel Bash calls `curl` waited on approval while `sleep 3;
    ls -la` ran unasked, and ls's result closed the curl card: no card,
    "Running curl · 45s", `openRequestIds` still naming the ask, codex
    waiting. Denying one of two parallel asks closed the other card too.
    codex's `PermissionAsk` had `tool_use_id: None`, so `settleAnsweredPrompt`
    decided the newest undecided card with the result's tool name. Shortest
    path (11 steps). Fixed: command, file-change and permissions approvals
    name the call they gate (`tool_use_id` is the request's `itemId`, the id
    of the call's `toolUse` and `toolResult`), and the desktop already
    matches a card by `tool_use_id` when it has one. An MCP tool approval
    names no call and is still matched by its `mcp__` name (outside the
    model).
11. **Fixed: codex `InterruptHonored`: a stop before codex started the turn was dropped.**
    Reproduced live on the Mac debug app (codex 0.170, gpt-6.2-sol):
    `interrupt_session` 16 ms after `send_user_message` was dropped and the
    turn ran to completion (about 41 s). `CodexDriver::interrupt` returned
    Ok and sent nothing when neither `active_turn` nor `requested_turn` was
    set: from `turn/start` until its response or `turn/started` is read
    (2 steps: you send, you press stop). A turn id alone is not enough:
    codex 0.171 refuses a `turn/interrupt` sent after the `turn/start`
    response and before `turn/started` ("no active turn to interrupt",
    probe p13) and runs the turn to `completed`; a first fix that sent the
    stop on the response's id failed live that way (13 ms, a 30 s turn), and
    the model now has codex refuse it. Fixed: `interrupt` sends at once only
    for `active_turn`. Before it, while a `turn/start` is unanswered
    (`pending_turn_starts`) or its response named the turn
    (`requested_turn`), the stop reserves an rpc id (`deferred_interrupt`)
    and `route_incoming` sends `turn/interrupt` right after the
    `turn/started` it reads; a refused `turn/start` that leaves no turn and
    no other start drops the waiting stop.
12. **Fixed: OpenCode `TurnProgress`, `AskResolution`: a server death was never reported.**
    `TurnProgress` reproduced live by the interrogate workstream: with `opencode
    serve` killed mid-turn (a Bash call sleeping 90 s), the turn never ended and
    only Stop recovered it. No driver was told when the shared server died:
    `spawn_sse` reconnected forever and no `Exited` was sent, so the sent turn,
    `in_turn` and any open ask stayed. `AskResolution`'s path (an ask open when
    the server dies) is model only. Now `OpenCodeServer::watch_exit` sees the
    child exit and every driver on the server sends `Exited` (crashed), about
    0.1 s after a SIGKILL in the E2E (`ServerExited`). `OpenCodeTurns-death`
    passes both rules and fails both again without `ServerExited`; the main
    config still fails both on findings 13 and 14's paths.
13. **OpenCode `InterruptSafe`, `TurnProgress`, `InterruptHonored` (model only): a late abort.**
    `prompt_async`, the abort and each permission reply are separate HTTP
    requests from separate tasks, so OpenCode can handle them in any order. An
    abort handled after a newer prompt started aborts that prompt; its
    `SentTurn` was reset by `begin_turn`, so the `MessageAbortedError` is not
    "ours", and if OpenCode had not yet created the reply message (no
    `finished` reply to mark it answered) the anchored turn's `session.idle`
    ends nothing: working forever. Shortest path (9 steps, `TurnProgress` 10
    with deaths and drops off): someone presses stop, you send, OpenCode
    handles the prompt and then the abort. `InterruptHonored` (3 steps): an
    abort handled before the prompt it was for does nothing, and the turn runs
    to the end. Not reproduced live: twice on the Mac (stop and an immediate
    send, also fired concurrently) OpenCode aborted the old turn and ran the
    new prompt to its end. Not fixed: ordering the driver's POSTs would queue a
    stop behind a slash command's `/command`, which answers only when its turn
    ends.
14. **OpenCode `AskVisible`, `TurnProgress` (model only): a lost SSE event.** An
    SSE event the driver never sees (a reconnect gap in `spawn_sse`, or the
    filter task's receiver lagging, which it skips with `continue`) leaves an
    ask OpenCode waits on that the desktop never shows (8 steps), or, when the
    lost event is the idle, a turn that never ends (10 steps). Not reproduced
    live: it needs the stream to drop with the server alive or 1024 events of
    lag. Not fixed: the fix is a resync of pending asks (`GET /permission`,
    `GET /question`) and of the session's status on every `server.connected`
    and every lag, more than a hardening of a path no one has seen.
15. **Fixed (found by the model, not reproduced live): OpenCode `AskResolution`,
    `InterruptSafe`: an ask raised as the stop lands stays open.** `interrupt`
    declines the asks the driver holds, but OpenCode can raise one while the
    abort is on its way; the abort then cancels it with no reply event, and the
    card stays after the turn ended. `OpenCodeTurns-stop` (13 steps): you send,
    OpenCode starts and asks, you press stop before the driver reads the ask,
    OpenCode handles the abort. Now the session's `session.idle` declines every
    ask the driver still holds (`decline_all`: an idle OpenCode holds none).
16. **Fixed for messages sent mid-turn: OpenCode `NoLostUserMessage`, `NoLostAgentMessage`: a stopped turn left a queued message unread.**
    Reproduced live (the interrogate workstream, OpenCode 1.18 on
    deepseek-v4.1-flash-free): a message sent past the desktop's queue while
    a turn ran was echoed at once, the turn was stopped, and it was never
    answered nor handed back; OpenCode kept it in the session's history and
    the model read it in a later turn's context. OpenCode holds a prompt sent
    mid-turn for its next step and, on an abort, keeps it unanswered. The
    driver now holds such a message's echo until an assistant step's
    `parentID` is at or past its id (`Turns.waiting`, `read_up_to`). When an
    interrupted turn ends with messages still waiting, `forget_unread`
    deletes each from OpenCode's history (DELETE
    `/session/:id/message/:id`, OpenCode 1.2.14+, retried on 409), then emits
    a `cancelled` lifecycle for each it deleted, then the turn's `TurnDone`
    (`DropUnread`): the relay resends an agent's message and the desktop
    hands back its own, and the delete keeps OpenCode from reading a copy as
    well. The filter task reads nothing more until the deletes are done, so
    a `TurnDone` held for a cut-short reply (`Settle`) also goes out after
    the handed-back lifecycles. One OpenCode would not delete stays waiting
    and echoes when a later step reads it. Two paths remain, both model only:
    - Two prompts in flight at once (8 steps): you send from an idle view, an
      agent's message follows while the driver's turn runs, OpenCode handles
      the agent's POST first and holds yours behind it, someone presses stop.
      Yours was echoed at send as a turn start, so nothing drops it, and it
      stays unread in the history. The agent's path is the mirror. Sending
      the driver's POSTs in order would close it; with OpenCode handling two
      prompts in send order (a scratch copy), `NoLostUserMessage` passes.
    - Finding 13's late abort (13 steps, with prompts in order): stop, then
      an agent's message; the agent's `begin_turn` resets `interrupted`, so
      the abort is not ours, the turn never ends and the held message is
      never dropped or echoed.
    Breaks in scratch copies: with prompts handled in send order, the driver
    before the fix (every echo at send, nothing dropped at a stopped turn's
    end) fails both rules in 9 steps; handing back without the delete fails
    `NoDoubleSend` in 17 steps (the resend and the copy left in the history
    are both read). With the filter task reading on while the deletes run (a
    delete in a task of its own), a config of one composer and one agent
    message, POSTs in order and no lost event or server death fails
    `NoLostAgentMessage` in 18 steps: the stopped turn's held `TurnDone`
    goes out at `SETTLE_WAIT` ahead of the handed-back lifecycle, and the
    relay never sends the agent's message again; awaiting the deletes, that
    config passes it.

17. **Fixed: `AskClosesOnAnswer`: an ask answered elsewhere stayed open until its tool finished.**
    After #520 the cancel took the id out of `pending_permissions` and emitted
    nothing, so nothing was recorded and the live card closed only on the
    tool's result. A deny writes that result with the cancel; an allowed
    Bash command writes it only once the command has run. Reproduced live
    (a PermissionRequest hook answering after 4 s, standing in for claude.ai;
    a 20 s command): the card and "Waiting for approval" stayed 24 s, and the
    log held no resolution. The cancel now emits the same event
    `respond_permission` or `answer_question` would, once: an id already
    answered here is no longer pending, and a later click finds none. A
    permission answered elsewhere records `allow: true`, which removes the
    card; the tool's result shows what ran or the denial. The actor also
    sets the row back to running once the last open ask is resolved
    (`settles_last_ask`), and `wait_agent`'s `asks_after` and the goal gate end
    their wait on a resolution as on a tool result; before, both paths read
    "needs you" until the turn ended.

18. **Fixed: an ended session's actor lived until the server stopped.**
    Every read of a session (a reopened tab, `list_turns`, a tail read)
    spawned its actor, and nothing ever stopped one. Reproduced live: reading
    1,000 ended sessions left 1,003 actors (with the run's own three) four
    minutes later, and the server's RSS went from 68.9 MB to 92.8 MB. An actor
    now retires after `ACTOR_IDLE` (60 s) with nothing attached and no relay
    feeding it, and keeps its revision state in `sessions.actor_state` for the
    next one.

19. **Fixed: `CodexTurns.SenderTold`: an agent's message recorded just after the relay's report went untold.**
    `SessionService::send` checks `require_live`, then records the row and
    sends. If the harness exited in between, the relay's report ran before the
    row existed. Claude's driver refuses a send after eof (#527), so the row
    is released and `message_agent` returns the error; codex's
    `send_user_turn` returns Ok after its stdout ends, so the row stayed until
    the next relaunch or restart sweep and the sender heard nothing.
    Reproduced live with codex and a scratch 10 s delay after `require_live`:
    `message-agent` returned "sent" 10 s after codex was killed, the row
    stayed and no wake reached the sender. The send now settles its own row
    when the driver it used is no longer registered (`recover_late_hold`):
    sent to a relaunched driver if one came up in between (`launch_driver`
    takes no op lock, so a resume can run between `require_live` and the
    hold, and its resend then found no row), else reported to a sender who
    can be told, else kept for the next relaunch. Splitting the model's agent
    send at `require_live` found that relaunch path (`NoLostAgentMessage`, 7
    steps) before the resend branch was added.

20. **Fixed: `NoLostUserMessage`: a desktop reload dropped the sends it had not seen stored.**
    Queued chips, steered chips and direct sends waiting for their echo lived
    only in the entry's memory. Reproduced live: a message queued behind an
    open ask was gone after a reload, and a steered message whose claude was
    killed right after a reload was never resent; the log held only its
    `commandLifecycle cancelled`. They are now kept in `localStorage` per
    session and handed back by the first fold after the reload. With
    `ClientRestart` dropping every chip, `NoLostUserMessage` fails.

Not in the models and found by the interrogate workstream (reproduced live):
a stopped Ship turn still sends its follow-up, on every provider. Fixed and
not in the models: a failed codex turn ended twice (`error` {willRetry:
false}, then a failed `turn/completed` 11 ms later, each a `TurnDone`); `error`
now posts only its text and `turn/completed` ends the turn once (a
`codex::protocol` test). Codex auto-declined an MCP tool's approval request
(`mcpServer/elicitation/request` with `_meta.codex_approval_kind`
`mcp_tool_call`) in `auto_decline`, so no card showed and the tool failed; it
is now a `PermissionAsk` named `mcp__<server>__<tool>`, answered `accept` or
`decline` (a `codex::driver` test). Other elicitations are still declined. An
OpenCode question's reply named no `?directory=`, so OpenCode looked it up in
its own cwd (the home directory), answered 404, and the turn waited on the
question forever; every driver request now names the session's directory
(`OpenCodeDriver::request`).

When one of these is fixed in the code, change the model to match, run
`check.sh`, and change that rule's line in `expected.txt` to `PASS`.

## Does a change touch the verified code?

Run this from the repo root:

```sh
git diff --stat b9c008a0acf0f956d8e444ccfbb6d67a0f72fbec -- \
  crates/hyprlayer-core/src/claude/driver.rs \
  crates/hyprlayer-core/src/claude/protocol.rs \
  crates/hyprlayer-core/src/codex/driver.rs \
  crates/hyprlayer-core/src/codex/protocol.rs \
  crates/hyprlayer-core/src/opencode/runtime.rs \
  crates/hyprlayer-core/src/runtime/sessions/held.rs \
  crates/hyprlayer-core/src/runtime/sessions/service.rs \
  crates/hyprlayer-core/src/runtime/sessions/forwarder.rs \
  crates/hyprlayer-core/src/runtime/sessions/mod.rs \
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
`rev`/reservation arithmetic, codex's tool-call translation, OpenCode's part
handling, catalog or skill routing does not move the models. A
change to when an echo is emitted or held, what `write_line` returns, what
`read_and_wait` emits for a line, when the relay ends or the registry drops a
driver, what opens or closes `in_turn`, which events hand chips back, how an
ask is opened, closed or reloaded, what `check` or `wake_one` read, how a lag
re-attaches, or how the desktop skips, holds or merges seqs does move them;
so does, for codex, when a turn id is set or cleared, what declines a steer,
or what settles an approval, and, for OpenCode, which event ends the sent
turn, what closes an ask, or what reports the server's death.

`SessionTurns.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `EchoOrHold`, `SendServer` | `ClaudeDriver::send_user_turn`, driver.rs:306 (refuses with `session_not_found` once `Inbox.ended`, 324; `Inbox::idle`, 79; `lifecycle`; echo before `write_line`; `take` on Err); `SessionService::send` releases the reservation on that refusal |
| `WriteOk` | `write_line` → `stdin_tx`, and `write_loop`, driver.rs:616 (breaks on the first failed write) |
| `ClientSend` | `sendMessage`/`deliverMessage` (direct: optimistic row and an `unechoed` entry until its echo; `session_not_found` puts it back in the queue), sessionStore.ts:1510/1573; `steerMessage` (chip with a `clientMessageId`), 1655; `handedBackBy`, 532, and `returnUnechoed`, 556 (a live `relaunched` hands back steered chips only); `settleFolded`, 594; the resend of a handed-back chip by `flushQueuedMessage`; server side `SessionService::send`, service.rs:271 |
| `AgentCheck`, `AgentHold` | `SessionService::send` with an agent author (agent_control/wake.rs, `message_agent`) and `send_app_turn` (follow-ups), split at the awaits between `require_live` and `send_user_turn` (the reservation, the archive flag, `held::hold`); each releases the row on Err. `launch_driver` takes no op lock, so a resume can come between. After an Ok send whose driver is no longer the registered one, `held::recover_late_hold` settles this send's own row, if unsent: sent again to a relaunched driver (`send_again`), else reported to a sender who can be told, else kept for the next relaunch. In `SessionTurns` claude's driver refuses once its stdout ended, so that branch is unreachable and left out |
| `ClientAnswer` | `respond_permission`, service.rs:416 → driver.rs:378 (emits `PermissionResolved` only when `pending_permissions` held the id; `answer_question` the same with `QuestionAnswered`); a card is answered once |
| `Interrupt` | `SessionService::interrupt`, service.rs:439 → driver.rs:433; `stopSession`, sessionStore.ts:1993 |
| `ClaudeRead`, `ClaudeAsk`, `ClaudeTakeQueued`, `ClaudeRemoteAnswer`, `ClaudeToolDone`, `ClaudeFinish`, `ClaudeStartQueued`, `ClaudeExit` | claude's own behaviour on stream-json (see the assumptions below); `ClaudeRemoteAnswer` is an answer on claude.ai or from a PermissionRequest hook: a deny writes the cancel and the error result together, an allow writes the cancel and `ClaudeToolDone` writes the result once the tool has run |
| `DriverRead` | `read_and_wait`, driver.rs:641 (`Init` 664, `CommandLifecycle` → `Inbox::frame` 96, `Result` 681, `CanUseTool` 703, `ControlCancel` 717, `Exited` 774); `control_cancel_request` → `ParsedLine::ControlCancel`, protocol.rs:207, which takes the id out of `pending_permissions` and, when it was there, emits the event the answer path emits (`QuestionAnswered` for an `AskUserQuestion`, else `PermissionResolved` with `allow: true`); a tool result passes through as `ToolResult`; at the end of stdout it sets `Inbox.ended`, 750, and declines every echo still in `Inbox.waiting` before `Exited` |
| `Relay` | `spawn_relay`, service.rs (its reader stops at `Exited`; its forwarder releases a held message on its `UserMessage`, keeps the ids of `cancelled` lifecycles, sends each event on, after a `TurnDone` sends the held ones among those ids again (`RelayResend`) before reading on, and after a reportable `Exited` calls `registry.remove` then `held::report_undelivered` for the messages whose sender can be told); `ActorState::handle_event`, forwarder.rs:856 (`author_for` 1056, `close_turn` 1124); the desktop's `applyEvent`, sessionStore.ts:454 (`handedBackBy` 532), `reduceEvent`'s `permissionAsk`/`permissionResolved` (`closeAsk`, transcript.ts:1011), `toolResult` (772, `settleAnsweredPrompt` 403), `turnDone` (853, `parkedState` 351), `exited` (867); `handBack`: after the desktop's Stop (`stopped`, set by `stopSession` and cleared by `sendMessage`) a handed-back chip goes to the composer (`"returned"`) instead of the queue |
| `RelayResend` | `held::send_dropped` → `send_again`, held.rs: `reserve_user_turn` and `send_user_turn` with the held nonce; an Err reports a sender who can be told and drops its row, and keeps one no one would hear about |
| `ClientReload` | `SessionService::list_turns`, service.rs:828 (re-sends the stored asks `pending_requests` names; `open_request_ids`), `settleUnwaitedAsks`, transcript.ts:1031 |
| `ClientRestart` | the desktop window reloading: `restoreSessionIfNeeded` folds the log (`settleUnwaitedAsks` when live, prompts expired when not) and takes the queued, steered and unechoed sends `keepUnsent` wrote to `localStorage` on every `setEntry` that changed them (`unsentFromLastLoad`), then `settleFolded` hands back those an exit or relaunch newer than `heldAfterSeq` dropped. Bounded at one reload (`MaxRestarts`); `CodexTurns` has it too |
| `ClientResume` | `ResumeSession` → `launch_driver`, service.rs (fresh driver and inbox, `Relaunched`, then `held::resend`: each held message once more, in order; one already resent is reported) |

`CodexTurns.tla` (the service, relay, actor and desktop rows are as in `SessionTurns`):

| Model action | Code (at the sync commit) |
|---|---|
| `SendServer`, `WriteOk` | `CodexDriver::send_user_turn`, codex/driver.rs:310 (`running_turn`, protocol.rs:219: `steer`, 373, with the echo in `steering`, protocol.rs:190; none: echo, the rpc id into `pending_turn_starts`, then `turn/start`), `write`, 632, `write_loop`, 800 |
| `ClientAnswer` | `respond_permission`, codex/driver.rs:430 (an id not in `pending_approvals` returns Ok and writes nothing) |
| `Interrupt` | `CodexDriver::interrupt`, codex/driver.rs:480 (`active_turn`: sends `turn/interrupt`; none while `requested_turn` is set or `pending_turn_starts` is not empty: reserves `deferred_interrupt`; neither: returns Ok, sends nothing) |
| `CodexRead`, `CodexBegin`, `CodexAsk`, `CodexOtherTool`, `CodexCommitSteer`, `CodexFinish`, `CodexExit` | codex app-server's own behaviour (see the assumptions below) |
| `DriverRead` | `read_and_wait`, codex/driver.rs:825 → `route_incoming`, 889 → `Translator`: `observe_response`, protocol.rs:242 → `turn_start_answered`, 207, sets `requested_turn` from a `turn/start` response, in line order, and drops its rpc id from `pending_turn_starts` (and `deferred_interrupt` when a refusal leaves no turn and no other start); after each notification, `started_turn_interrupt`, 225, sends the deferred stop once `turn/started` named the turn (`write_line`, driver.rs:957); `turn/started`, 355; `item_completed`'s `userMessage` releases a steer's echo, 717; a refused steer's task takes it and sends `declined_message`; a failed `turn/start`'s task sends a failed `TurnDone`; `turn/completed`, 369, clears both ids and runs `decline_steers`, 200; `server_request`, 482 (`pending_approvals`); `serverRequest/resolved`, 334 → `settle`, 455 (drops the approval with that rpc id, `PermissionResolved {allow: false}`); `Exited`, driver.rs:882 |
| `Relay` | as `SessionTurns`, with `settleAnsweredPrompt`, transcript.ts:403, deciding a codex card by its `tool_use_id`, the approval's `itemId` (`gated_call`, protocol.rs:795) |
| `ClientResume` | `thread/resume` in a fresh app-server and driver, then `held::resend` |

`OpenCodeTurns.tla` (the relay, actor and desktop rows are as in `SessionTurns`):

| Model action | Code (at the sync commit) |
|---|---|
| `SendServer` | `OpenCodeDriver::send_user_turn`, opencode/runtime.rs (`Turns::running`: while the driver's turn runs the echo waits in `Turns.waiting`, otherwise it goes out first; `Turns::begin`, `prompt_async`: a failed POST sends "OpenCode request failed" and a failed `TurnDone`, which the model counts as reported) |
| `ClientAnswer` | `respond_permission`, runtime.rs:1068 (`take_ask`, then a POST either way) |
| `Interrupt` | `OpenCodeDriver::interrupt`, runtime.rs:1135 (drains the asks map as declined, marks the sent turn, POSTs abort) |
| `ServerExited` | `OpenCodeServer::watch_exit` (polls the child every 250 ms; `end_sessions` publishes one `Exited`, crashed), then each driver's filter task forwards it after the SSE events it already has |
| `RelayExited` | `spawn_relay` on `Exited`: drops the driver; the desktop's `exited` reducer ends working and expires the cards; the composer's own unechoed message goes back to its queue (`chip`), and the held agent messages are reported (`held::report_undelivered`, counted as `failed`) |
| `OcRead`, `OcAsk`, `OcElsewhereReply`, `OcFinish`, `OcDown` | OpenCode's own behaviour (see the assumptions below) |
| `SseDrop` | `spawn_sse`, runtime.rs:412 (reconnects after a gap), the filter task's `RecvError::Lagged(_) => continue`, 834 |
| `DriverRead`, `Expire` | the filter task: `admits` (the session's own events, and of the subagent sessions under it only their asks and replies), `track_ask`, 643 (closes an ask a reply took), `decline_all` on the session's idle, then `Translator::handle`, 1407 (`message.updated`, 1411: anchor and `answered` via `reply_finished`, 1291, and an assistant's `parentID` releases the waiting echoes at or before it, `read_up_to`; `session.idle`, 1494; `session.error`, 1516, an abort is ours only if `interrupted`), `expire`, 1379; `end_turn` holds a turn's end while its reply is open (`open_reply`) until that reply's `message.updated` settles it; `Translator::end` moves a stopped turn's waiting messages to `unread`, and the filter task sends the turn's events only after `forget_unread` |
| `Settle` | the held end released by `expire` (`SETTLE_WAIT`, 3 s) or by a newer turn's next event (`Translator::handle`) |
| `DropUnread` | `forget_unread`, awaited by the filter task: `delete_message` for each (409 retried for about 2 s), a `cancelled` lifecycle for each deleted, then the turn's events; one not deleted goes back to `Turns.waiting`. The model deletes when OpenCode is alive and idle, as one step |
| `RelayResend` | as `SessionTurns` (OpenCode's `send_user_turn` never fails) |
| `OcRead`, `OcFinish` (a run) | a prompt's run reads every message a stopped turn left in the history (`orphans`) as well as its own, each copy once |
| `Relay` | as `SessionTurns`, plus `author_for`'s `next_turn` (forwarder.rs:1056); OpenCode's permission names never equal a normalized tool name (`normalize_tool`, 1679), so no tool result decides a card |

`RestartWake.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `Restart` | server stop (harnesses die, the actor with them) and start: `settle_stale_sessions`, store/sessions.rs:698 (`is_working`, 271), from runtime/mod.rs:243; `spawn_after_restart`, wake.rs:28 |
| `WakerCheck` | `wake_stopped` → `check`, wake.rs:49/155 (ended, archived workstream at 166, `unanswered_restart_wakes`, 179, store/sessions.rs:1238, against `MAX_RESTART_RESUMES`) |
| `WakerWake` | `wake_and_forget` → `wake_one`, wake.rs:193 (`resume_if_exited_held`; `launch_driver` holds the workstream lock and refuses an archived one, `hold_unarchived`, workstreams.rs:1845, then sets `Running` and deletes the pending wake; `record_restart_wake`, wake.rs:210, store/sessions.rs:1250, before `send_app_turn_held`) |
| `StoreEcho` | the wake's `UserMessage` through `spawn_relay` to `handle_event` |
| `ClaudeFinishes` | a `TurnDone` row |
| `Archive` | `WorkstreamService::archive` → `archive_inner`, workstreams.rs:1836/1857, under `lock_pair` (`stop_for_archive`, 2063: `end_quietly_and_await_exit`, 2081 → service.rs:895), then the status |

`SessionSync.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `Store` | `handle_event`'s persist then fan-out, forwarder.rs:856; `LiveAttachment::deliver`/`try_deliver`/`mark_lagged`, delivery.rs:130/108/119 |
| `Forward` | `forward_session`, streams.rs:1411 (the `Lagged` arm at 1443 → `attach_after`, forwarder.rs:571, and `register_after`, 626, in one actor turn) |
| `Receive` | `dispatch`, sessionStore.ts:357 → `route`, transcriptSync.ts:267 (skip, hold, flush via `applyHeld`, sessionStore.ts:429, apply) |
| `PullRead`, `PullApply` | `pullNow`, sessionStore.ts:1776 → `list_turns`; `mergeEntries`, transcriptSync.ts:352; `trimHeld`, sessionStore.ts:446 |
| `End`, `Resume` | the relay ending at `Exited` and dropping its ingress sender; `launch_driver` → `spawn_relay` with `SessionActors::ingress`, which spawns an actor when none runs |
| `Close`, `Reopen` | the tab's stream ending (its attachment detaches); `attach_from`(cursor) through `SessionActors::known_actor`, which spawns the actor again, replays the log after the cursor and registers the attachment in one actor turn |
| `Retire` | the actor's `ACTOR_IDLE` timer, 60 s after its last use (a desktop's `rev_state` poll is not a use: with no actor it is answered from the row) → `retire_idle` (sessions/mod.rs), which holds the registry lock while it sends `Control::Retire`; the actor agrees only when `idle` (no open attachment, counting one whose stream closed as gone; no reservation, pending write, open turn or kept stream; a workflow neither armed nor running; nothing queued; and no ingress sender but its handle's, its own and its workflow pump's), saves its `RevSnapshot` to `sessions.actor_state`, and stops; `ActorState::new` restores it when the row's `rev` still matches |
| `Lose` | a relay's `ingress.send` to an actor that stopped (only reachable when `Retire` ignores a live relay) |

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
  to tell. Left out: a second resend (bounded by two processes), and the
  restart sweep.
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
- `CodexTurns`: one composer message, one agent message, one ask id, two
  app-server processes (one exit and one resume), one interrupt, one other
  tool result. `OpenCodeTurns`: one composer and one agent message, one ask
  id, one interrupt, one lost SSE event, one server death; a driver exits only
  when that death is reported, and there is no resume.
  `OpenCodeTurns-stop.cfg` checks the same rules on one composer message,
  one ask and one stop, with OpenCode handling the driver's POSTs in the
  order sent (`InOrder`), no lost SSE event and no server death: the stop and
  ask paths without findings 12 to 14 in the way. `OpenCodeTurns-death.cfg`
  has one composer message, POSTs in order, no lost SSE event and no stop,
  and one server death: finding 12's path alone.
- Assumptions about codex app-server: `turn/start` while a turn runs is
  refused (an error response; codex 0.171 instead takes the message into the
  running turn and answers with that turn's id, probe p10, which the model
  does not follow); a steer is accepted only into the running turn
  it names; codex drops steers it never committed when the turn ends (the
  driver's own comment); it answers `turn/start` as it accepts the turn and
  sends `turn/started` after that (all 26 captured turns, 4 to 33 ms later),
  and refuses `turn/interrupt` in between ("no active turn to interrupt",
  probe p13); on `turn/interrupt` it settles its open approvals,
  writes `turn/completed` and then `serverRequest/resolved` for each, and
  may or may not complete their items. The self-settled approval and its
  `serverRequest/resolved` after `turn/completed` are confirmed against
  codex 0.171 (the interrogate workstream's probes p1, p4, p8a).
- Assumptions about OpenCode: a prompt POSTed while busy runs after the
  running one; an abort ends the running prompt, and the prompts held behind
  it may or may not run; the aborted reply message may or may not exist yet,
  and if it does, its cut-short part and its MessageAbortedError come after
  the abort's error and idle, well inside `SETTLE_WAIT` (under 300 ms against
  1.18.18), so `Settle` runs out only on a reply that will not settle;
  another OpenCode client can answer an ask (`permission.replied`); a
  subagent's ask carries its child session's id, which `session.created`
  names with its parent's (seen against 1.18.18). Only the server death is
  confirmed live.
- Left out of `CodexTurns`: codex errors (`error` posts text, the failed
  `turn/completed` after it ends the turn as `CodexFinish` does),
  `requestUserInput` questions, child threads, MCP elicitations (declined)
  and MCP tool approvals (asked like any approval, but outside the model),
  rewind, collaboration mode, a refused steer while live
  (the end-of-turn decline stands for both).
- Left out of `OpenCodeTurns`: questions (a reply reaches OpenCode only with
  the session's `?directory=`, which every driver request carries),
  `command_async` (slash commands), the 5 s `UNPLACED_WAIT` as time (`Expire`
  may fire any time after), compaction, a subagent's text and tools (its asks
  are `OcAsk`s, which `admits` passes), the kill path (`end`), a deliberate
  kill of a live server (`kill_blocking` on app quit or when `ensure` replaces
  it, which sends `Exited` detached), and a server that comes back.
- Left out of `RestartWake`: orphan wakes (`queue_orphan_wakes`, mid-turn off,
  with open background tasks), the overlay wait and concurrency (a free delay
  between check and wake stands in for them), goals (the goal resumer's
  sessions), the session's own `archived_at` (open to the same race), and
  claude crashing during a wake.
- `SessionSync`'s actor lifetime: one end and resume, one close and reopen.
  The per-author revision state the snapshot keeps (`rev_by_author`,
  `foreign_floor`) is not in any model; `session_runtime`'s
  `ended_sessions_leave_no_actors_and_keep_who_sent_what` checks it.
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
   - `SessionTurns.AskClosesOnAnswer`: `DriverRead` leaves `bus` alone on a
     `cancel` line (the code after #520): a remote allow's card stays open
     while its tool runs.
   - `SessionTurns.ResolvedOnce`: `DriverRead` emits `resolved` on every
     `cancel`, held or not: an ask answered here and elsewhere at once is
     resolved twice.
   - `SessionSync.NoLostEvent`: `Retire` without `~harness` (the sender count
     ignored): the relay sends into a retired actor.
   - `SessionSync.NoSkippedSeq`, `ClientsConverge`: `Reopen` replays from
     `applied + 2` (an `attach_from` that skips the row after the cursor).
   - `SessionTurns.NoLostUserMessage`: `ClientRestart` sets every chip to
     `"none"` (the desktop before it kept unsent messages across a reload).
   - `CodexTurns.NoLostAgentMessage`: `AgentHold` only reports a late row and
     never sends it to a relaunched driver: 7 steps, an agent passes
     `require_live`, codex exits, the driver reads eof, the relay drops the
     driver, you resume (its resend finds no row), the send records its row,
     the relaunch is relayed; a message with no sender stays held in a live,
     quiet session.
   - `CodexTurns.SenderTold`: `AgentHold` keeps a late row held instead of
     reporting it (the code before `recover_late_hold`): 5 steps, an agent
     passes `require_live`, codex exits, the driver reads eof, the relay
     reports and drops the driver, the send records its row.
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
   - `NoLostUserMessage`: `ClientSend`'s direct send keeps no chip (`"none"`
     instead of `"unechoed"`, `deliverMessage` before the fix): a 6-step
     held-echo path ending in claude's exit.
   - `NoDoubleSend`: `Relay` hands `"unechoed"` chips back on `relaunched`
     too: a direct send right after a resume is handed back by the resume's
     own `relaunched` and sent again (20 steps).
   - `CodexTurns.NoLostUserMessage`: the same `"none"` for a direct send in
     `ClientSend`: the 4-step exit window.
   - `WakeBounded`: `WakerWake` leaves `wakes` alone and `StoreEcho` counts
     it instead (`unanswered_wakes` over stored rows, the code before the
     fix).
   - `SessionTurns.AskVisible`: `Relay` closes every open card on any tool
     result (closing by name instead of by `tool_use_id`).
   - `SessionTurns.InterruptHonored`: `Interrupt` writes its line ahead of
     the messages already in the pipe.
   - `CodexTurns.AskResolution`: `DriverRead` ignores `srvResolved` (the
     code before the fix): a 15-step path, an agent's message, codex asks,
     stop, the desktop keeps the ask after `turnDone`.
   - `CodexTurns.AskVisible`: `Relay` closes the newest open card on any
     tool result (by name, the code before the fix): 11 steps, a Bash call
     that asked nothing finishes while another waits on its approval.
   - `CodexTurns.InterruptHonored`: `Interrupt` sends on any turn id it
     knows (`requested_turn` too) and waits only when it knows none (the
     first fix): 6 steps, an agent's message, codex answers the
     `turn/start`, the task stores the id, stop, codex refuses the
     interrupt before `turn/started`.
   - `CodexTurns.TurnProgress`, `InterruptSafe`: the `turn/start` task as a
     step of its own again (`StartTask`, storing the response's id at any
     later point, the code before the fix): 11 and 12 steps.
   - `CodexTurns.NoDoubleSend`: codex keeps an uncommitted steer across the
     end of its turn and commits it later, while the driver declines it.
   - `OpenCodeTurns.NoLostUserMessage`, `NoLostAgentMessage`: `send_user_turn`
     sends no echo.
   - `OpenCodeTurns.NoDoubleSend`: a prompt held behind a running one is also
     taken in at once; or `DropUnread` hands back without deleting (17 steps).
   - `OpenCodeTurns.NoLostUserMessage`, `NoLostAgentMessage`, with OpenCode
     handling prompts in send order: the driver before finding 16's fix (an
     echo at send for every message, nothing dropped at a stopped turn's
     end), 9 steps.
   - `OpenCodeTurns.NoLostAgentMessage`, in a copy with one composer and one
     agent message, POSTs in order and no lost event or server death:
     `DriverRead`, `Expire` and `Settle` not waiting for `DropUnread` (the
     deletes in a task of their own), 18 steps.
   - `OpenCodeTurns-stop.AskVisible`: an `OcAsk` from a subagent's session
     never reaches the driver (the filter before `admits`; in the copy,
     `OcAsk` may leave `out` unchanged).
   - `OpenCodeTurns-death.TurnProgress`, `AskResolution`: no `ServerExited`
     (the driver before `watch_exit`; the copy also drops `Quiet`'s
     `alive \/ ~live`, since nothing is then in flight).
   - `OpenCodeTurns-stop.AskResolution`, `InterruptSafe`: `DriverRead`
     closes nothing on an idle (the filter task before `decline_all`).
   - `OpenCodeTurns-stop.TurnProgress`: `EndTurn` sends the `TurnDone` at
     once while the reply is open (the translator before `end_turn`): the
     stopped reply's last part lands after it and the desktop shows working
     (16 steps).
   - `CodexTurns.NoLostAgentMessage`: `Relay` never fills `resendDue`
     (`send_dropped` gone), a 14-step path where codex ends the turn
     without committing the agent's steer.
   - The OpenCode FAILs fail today; once one passes, add a break for it
     here.
3. Run `check.sh`. Update `expected.txt` only for a rule whose verdict the
   change was meant to move, and say so in the PR.
4. Move the sync commit (the hash in the `git diff` command above and below)
   forward only after you have read the covered code at the new commit and
   confirmed the models describe it.

Last confirmed against: `b9c008a0acf0f956d8e444ccfbb6d67a0f72fbec`.
