---
name: verify-goal-runners
description: Model-check the goal runner protocol (one runner per goal; pause, stop and resume; server restarts; the restart resumer and waker) with TLA+. Use before merging any change to runtime/goals/mod.rs, sessions/goal_loop.rs's run/open/resume/boundary/gate/send/after/judge/check/apply/conclude, or sessions/wake.rs's check, and whenever a goal is reported stuck running, running twice, or ignoring a pause or stop.
---

# Verify the goal runners

`GoalRunners.tla` is a model of how one goal is driven: the runner registry in
`GoalRunners`, the runner task in `goal_loop::run`, the service calls
`pause`/`stop`/`resume`, a server restart, and the two tasks a restarted server
starts (`resume_running` and the session waker). TLC checks every interleaving
of those steps, up to the bounds in `GoalRunners.cfg`.

## The rules

| Rule | In one sentence | The harm when it breaks |
|---|---|---|
| `SingleDriver` | At most one runner drives a goal at a time. | Two runners write the same row and send turns into one session: double spend, scrambled state. |
| `PauseHonored` | After the server tells you a pause was accepted, at most the iteration already being sent goes out. | The goal keeps running and spending after you paused it. |
| `StopHonored` | After a stop is accepted, at most the iteration being sent goes out, and the goal never shows paused. | A stopped goal comes back, or keeps spending. |
| `IterationCap` | A goal never sends more iterations than `max_iterations`. | One turn beyond the limit the user set. |
| `NoStrandedGoal` | Once everything settles, a running goal has a runner. | The UI shows "running" and nothing happens until someone presses resume. |
| `CutOffSessionPickedUp` | A session whose turn a restart cut off is woken, or its goal is resumed. | The session's own turn stays dead after a restart. |
| `TypeOK` | The variables stay in their domains. | None, it only guards the model. |

## Run it

From the repo root:

```sh
.claude/skills/verify-goal-runners/check.sh
```

It needs `java` (11+) and `mise`. It installs TLC 1.7.4 through mise, pinned by
checksum (set `TLA2TOOLS=/path/tla2tools.jar` to skip mise), and runs TLC once per rule.
It prints `PASS`/`FAIL` per rule, compares the list with `expected.txt`, and exits
non-zero when they differ. About 80 s on 16 cores: the two passing rules each
explore about 18 M states, while failing rules stop at their first, shortest
counterexample. `OUT=dir` keeps the logs. Each rule's TLC output is in
`$OUT/<Rule>/tlc.out`.

**Read every result with the pstack `/statechart` skill.** Give it this spec and
the view module `statechart-view.ts` beside it, which labels the states and steps in the user's words. Add a label there for any new action. It prints each
broken rule as its shortest path in plain words. Use it for the report to the
user, not as the verdict: past the first break it bounds each run at 100,000
states, so it can list rules as not checked. `check.sh` gives the verdict.

## Known findings (the baseline in `expected.txt`)

The model fails five rules on the code at the sync commit. These are findings
about the product, not modeling errors. Each path below was walked step by step
against the code. None was replayed in a running server.

1. **`PauseHonored`, `StopHonored`: a pause or stop that is accepted but not yet applied dies with the server.**
   `try_pause`/`try_end` return as soon as `GoalRunners::send` puts the control on
   the runner's channel (`Delivery::Delivered`), and they write nothing to the row.
   The runner reads the channel only at a select point (`working`, `check`,
   `wait_idle`) or in `boundary`'s `drain_controls`. It does not read it while it
   waits in `RunnerChain`, `open`, `gate` (op lock, relaunch) or `send`. A restart
   in that window drops the control, and the resumed goal runs on. Shortest pause
   path: you press pause while the runner is in `gate`, iteration 1 is sent, the
   server restarts, the goal is resumed, and iteration 2 is sent. Shortest stop
   path: a stop is accepted, the server restarts, you press pause, and the
   stopped goal shows paused.
   Writing `Pausing`/`Stopping` into `step` in the service does not fix it. The
   model still fails with that change: `send` rewrites the step to `Working`,
   `after` rewrites it to `Checking`, and a `StartMode::Normal` spawn ignores the
   persisted step. A requested pause or stop needs its own column that only
   `conclude` clears and that `Run::open` reads.
2. **`IterationCap`: a restart that cuts off the last iteration buys one more.**
   After a restart, `Run::open` closes the cut-off iteration and `drive` goes to
   `gate` → `send`, which start `iteration + 1` without comparing it with
   `max_iterations`. Only `decide_boundary` (in `judge`) and `recover` (the crash
   path) check the limit.
3. **`NoStrandedGoal`: a runner that exits from `Run::open` is never marked finishing.**
   `Run::open` returns `Err(Over)` when the goal is not running, and `run` returns
   without `lease.finishing()`. A `spawn` in that window sees a live,
   non-finishing runner and does nothing. Then the runner's `Registration` drops
   and the goal is left running with no runner. Path: a restart, the resumer
   writes `Resuming`, your pause writes `Paused` (no runner yet), the resumer's
   runner opens and sees paused, your resume writes running, your resume's
   `spawn` is a no-op, and the runner exits.
4. **`CutOffSessionPickedUp`: a session whose goal is paused is never woken.**
   `wake::check` skips any session with `session.goal` set ("a running or paused
   goal is the goal resumer's"), but `resume_running` reads only
   `GoalStatus::Running`. This may be intended. It is a finding until the owner
   says so.

When one of these is fixed in the code, change the model to match, run
`check.sh`, and change that rule's line in `expected.txt` to `PASS`.

## Does a change touch the verified code?

Run this from the repo root:

```sh
git diff --stat 63421a2c9f91ee07e1da31545db3088e95800863 -- \
  crates/hyprlayer-core/src/runtime/goals/mod.rs \
  crates/hyprlayer-core/src/runtime/sessions/goal_loop.rs \
  crates/hyprlayer-core/src/runtime/sessions/wake.rs
```

Anything listed means: read the diff against the table below. A change to the
check commands, prompts, spend arithmetic or `decide_boundary`'s order beyond
"met, limit, pause" does not move the model. A change to who writes `status` or
`step`, what holds `transitions`, when a control is read, or what `spawn`,
`Registration` or `finishing` do does move it.

| Model action | Code (at the sync commit) |
|---|---|
| `Spawn` | `GoalRunners::spawn`, goals/mod.rs:378 (no-op on a live runner; chain after a finishing one) |
| `RunnerChain` | the `earlier.changed()` wait in `spawn`'s task |
| `RunnerOpen` | `Run::open`, goal_loop.rs:592 (exits when status ≠ running) |
| `RunnerResume` | `Run::resume`, goal_loop.rs:696 |
| `RunnerApply` | `apply`/`request_pause`, goal_loop.rs:1522/1542, from `drain_controls` (1503) or the selects in `working` (1003) and `check` (1375) |
| `RunnerToGate`, `RunnerGate` | `boundary` (802) → `gate` (836) → `send` (936) |
| `RunnerTurnEnds` | `after` → `close_iteration(.., Some(Checking))`, goal_loop.rs:1127 |
| `RunnerJudge` | `judge` (1217) → `decide_boundary` → `follow` |
| `RunnerFinishing` … `RunnerWrite` | `conclude`, goal_loop.rs:1620 (`finishing`, `transition`, drain, write) |
| `RunnerLift`, `RunnerExit` | `lift_cap` (1635); `Registration::drop`, goals/mod.rs:359 |
| `Pause*` | `GoalService::pause` (698) → `try_pause` (1055), `wait_exit` (442) |
| `Stop*` | `stop` (747) → `end` (1041) → `try_end` (1080) |
| `Resume*` | `GoalService::resume` (713), `running_again` (292) |
| `Restart` | server exit + `stop_all`; `spawn_resumer`/`spawn_waker` from runtime/mod.rs:480 |
| `Srv*` | `resume_running` (475) → `resume_one` (516) |
| `WakerCheck`, `WakerWake` | `wake::check`, wake.rs:144–150; `wake_one` |

Granularity: an action is one step only where the code makes it one. That means
a section under the `live` mutex (`spawn`, `send`, `finishing`,
`Registration::drop`), one DB write, or a service call's load-and-send while it
holds `transitions`, during which only the runner's `conclude` could change
`status`, and `conclude` needs the same lock.

## Bounds and what the model leaves out

- One goal, `MaxIter = 2`, `MaxRestarts = 2`, at most one pause, one stop and one
  resume press, at most `2 + MaxRestarts` runner generations.
- Left out: harness crashes and `recover`, the `NEEDS-HUMAN`, turn-failure,
  stall, spend and time limits (the iteration limit stands in for them),
  `Reload`/`UserInterrupt` controls, `Waiting` (merged into `Working`), the
  `restart_resumes` counter (its pause after 3 never triggers within 2 restarts),
  `stop_for_workstream` and `note_session_killed`, multiple goals and sessions,
  and timing. A `wait_exit` timeout and every select race are free choices, so
  the model allows more orders than the code produces, never fewer.
- A PASS covers these bounds only.

## Real bug or modeling error?

For each FAIL, take its trace (`/statechart` gives it in words, or `$OUT/<Rule>/tlc.out`)
and walk it action by action through the code in the table above. If every
step is possible in the code, it is a bug. Report it with the path in the
code's words. Do not weaken the rule or the model to make it pass. If a step is
impossible in the code (the code holds a lock the model does not, or reads a
field the model forgets), the model is wrong: fix the action, re-run, and say
what you changed.

## When a change is intended

1. Change the actions that mirror the changed code, keeping one action per
   indivisible code step, and update the table.
2. Prove the rule you touched can still fail. Break that code path in the model on
   purpose (for `SingleDriver`, make `RunnerChain`'s guard `TRUE`, which removes
   the wait for the earlier runner) and see `check.sh` report it, then undo the break.
3. Run `check.sh`. Update `expected.txt` only for a rule whose verdict the
   change was meant to move, and say so in the PR.
4. Move the sync commit (the hash in the `git diff` command above and below)
   forward only after you have read the covered code at the new commit and
   confirmed the model describes it.

Last confirmed against: `63421a2c9f91ee07e1da31545db3088e95800863`.
