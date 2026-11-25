---
name: verify-workstream-archive
description: Model-check that archiving a workstream lets go of everything the app holds open in the worktree before it moves it, and that the move survives a transient outside holder, with TLA+. Use before merging any change to WorkstreamService's archive_inner or stop_for_archive, worktree/manager.rs's move_worktree, index/watcher.rs's WatchHandle, terminal/mod.rs's PtySession kill or released, process/job.rs, process/mod.rs's SupervisedChild::wait, the terminal or script registries' teardown, or anything new that opens files or runs processes inside a worktree. Also use whenever an archive fails with "Permission denied" (usually on Windows).
---

# Verify workstream archive

Windows refuses to rename a directory while any handle inside it is open, and
`git worktree move` is a rename. `ArchiveMove.tla` models one archive: every app
holder of the worktree goes `held → releasing → free`, an outside holder (Defender,
the search indexer) may open the tree for a while, and the move retries with backoff.
`ArchiveMove-v0_9_0.cfg` is the code as shipped in v0.9.0, which asked holders to
let go without waiting. `ArchiveMove-fixed.cfg` is the code now.

## The rules

| Rule | In one sentence | The harm when it breaks |
|---|---|---|
| `NoAppHolderAtMove` | A move is never refused because of something the app itself still holds open. | "was not archived: … Permission denied", after the agents were already stopped. |
| `NeverRolledBack` | With the app's holders gone and outside holders transient, the archive succeeds. | The same error from an indexer or virus scanner touching the tree. |
| `RollbackRearms` | A rolled-back archive leaves the workstream watched again. | The workstream stays active but stops showing file changes. |
| `TypeOK` | The variables stay in their domains. | None, it only guards the model. |

## Run it

```sh
.claude/skills/verify-workstream-archive/check.sh
```

Needs `java` 11+ and `mise` (or `TLA2TOOLS=/path/tla2tools.jar`). It runs TLC once
per rule per config, compares `<PASS|FAIL> <design> <rule>` with `expected.txt`, and
exits non-zero on a difference. About 5 s. `OUT=dir` keeps the logs. Read a failing
path with the pstack `/statechart` skill and `statechart-view.ts`.

`v0_9_0` must keep failing `NoAppHolderAtMove` and `NeverRolledBack`: it proves the
rules still catch the bug.

## Holders

`AppHolders` in the spec lists everything the app keeps open in a worktree. A new
one goes in the spec and in this table, with how archive confirms it is gone.

| Holder | Released by | Confirmed by |
|---|---|---|
| `watch` | `WatchHandle::release`, index/watcher.rs:33 | `configure` after the unwatches; notify's Windows thread answers it only after closing their handles (watcher.rs:43) |
| `terminal` | `TerminalRegistry::remove_for_workstream` (terminal_registry.rs:103) → `PtySession::kill` (terminal/mod.rs:802) ends the terminal's job | `PtySession::released` (terminal/mod.rs:823): the job has no processes; awaited in `stop_for_archive` (workstreams.rs:2135) |
| `script` | `ScriptRegistry::clear_workstream` (script_registry.rs:153), a `PtySession` | as `terminal` |
| `harness` | `end_quietly_and_await_exit` (workstreams.rs:2125) | `SupervisedChild::wait` (process/mod.rs:533) returns only once the harness's job is empty, before the session leaves the registry |

| Model action | Code |
|---|---|
| `Start` | `stop_for_archive`, workstreams.rs:2107, then the watch release at workstreams.rs:1985 |
| `Released(h)` | the OS closing the handles: a job emptying, notify's thread closing a directory handle |
| `Stopped` | the waits above |
| `Move` with retries | `move_worktree`, worktree/manager.rs:572; `MOVE_RETRY_BACKOFFS` (manager.rs:16) |
| rollback re-arm | `watch.insert` on a failed move, workstreams.rs:2003 |

## What it leaves out

- Waits are bounded (5 s per terminal set and per harness tree). A process that
  outlives its bound reaches the move, which then retries and may roll back.
- A process a terminal started with `CREATE_BREAKAWAY_FROM_JOB` leaves the job on
  purpose, and a grandchild spawned before the job is joined (the first instant of a
  shell) escapes it.
- OpenCode sessions share one server (cwd home) that is not stopped per workstream,
  so a watcher or tool process it keeps in the worktree is not a holder here. Add it
  if a trace ever names it.
- The model proves the order of steps, not Windows itself. Prove a change on a real
  Windows machine with `archive_moves_a_worktree_a_terminal_is_still_running_in`
  in `crates/hyprlayer-core/tests/workstream_runtime.rs`. It passes trivially on
  macOS and Linux, which allow the rename.

## When a change is intended

1. Change the matching action and the tables.
2. Break the fixed design on purpose (for example, leave `"watch"` out of `Stopped`'s
   wait) and see `check.sh` report it, then undo the break.
3. Run `check.sh`, and update `expected.txt` only for verdicts the change meant to move.

Last confirmed against: `d890e31b`.
