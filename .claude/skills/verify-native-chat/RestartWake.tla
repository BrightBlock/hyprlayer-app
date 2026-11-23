---------------------------- MODULE RestartWake ----------------------------
(* One composer session cut off mid-turn by server restarts: settle_stale_sessions
   queuing its wake, wake_stopped's check and wake_one, the wake turn's echo on its
   way to the log, claude finishing a turn, and archiving the workstream
   (WorkstreamService::archive). SKILL.md maps every action to the code. *)
EXTENDS Integers, Sequences

CONSTANTS MaxRestartResumes, MaxRestarts

VARIABLES
  up,            \* the server is running
  restarts,
  alive,         \* a claude harness is live for the session
  rowLive,       \* sessions.state is starting/running/awaiting_*
  lastKind,      \* the newest userMessage/turnDone row: "user", "wake" or "done"
  logWakes,      \* restart-wake rows since the last turnDone (unanswered_wakes)
  echoing,       \* a wake turn's echo broadcast but not yet stored by the actor
  ended,         \* sessions.ended_at is set
  pendingWake,   \* a pending_wakes row with mid_turn
  waker,         \* the restart task: "idle", "start", "due" (checked, waiting its turn), "done"
  wsArchived,
  received,      \* wake turns claude took in since it last finished a turn
  wokeArchived   \* a wake launched a harness in an archived workstream

vars == <<up, restarts, alive, rowLive, lastKind, logWakes, echoing, ended,
          pendingWake, waker, wsArchived, received, wokeArchived>>

Init ==
  /\ up = TRUE /\ restarts = 0
  /\ alive = TRUE /\ rowLive = TRUE /\ lastKind = "user" /\ logWakes = 0
  /\ echoing = FALSE /\ ended = FALSE /\ pendingWake = FALSE
  /\ waker = "idle" /\ wsArchived = FALSE /\ received = 0 /\ wokeArchived = FALSE

Working == rowLive /\ lastKind \in {"user", "wake"}

\* The server stops (its harnesses die with it, the actor with any echo it had not
\* stored) and starts again: settle_stale_sessions queues a wake for a working row.
Restart ==
  /\ up /\ restarts < MaxRestarts
  /\ restarts' = restarts + 1
  /\ alive' = FALSE /\ echoing' = FALSE
  /\ pendingWake' = (pendingWake \/ Working)
  /\ rowLive' = FALSE
  /\ waker' = "start"
  /\ UNCHANGED <<up, lastKind, logWakes, ended, wsArchived, received, wokeArchived>>

\* wake::check, after the overlay wait.
WakerCheck ==
  /\ waker = "start"
  /\ IF pendingWake /\ ~ended /\ ~wsArchived /\ logWakes < MaxRestartResumes
       THEN waker' = "due" /\ pendingWake' = pendingWake
       ELSE waker' = "done" /\ pendingWake' = FALSE
  /\ UNCHANGED <<up, restarts, alive, rowLive, lastKind, logWakes, echoing, ended,
                 wsArchived, received, wokeArchived>>

\* wake_one under the op lock: resume_if_exited_held, then send_app_turn_held; then forget.
\* launch_driver holds the workstream's lock and refuses an archived one (hold_unarchived);
\* archive takes the same lock, so the two never interleave.
WakerWake ==
  /\ waker = "due"
  /\ waker' = "done" /\ pendingWake' = FALSE
  /\ IF ~alive /\ ~wsArchived
       THEN /\ alive' = TRUE /\ rowLive' = TRUE
            /\ wokeArchived' = (wokeArchived \/ wsArchived)
            /\ echoing' = TRUE /\ received' = received + 1
       ELSE UNCHANGED <<alive, rowLive, wokeArchived, echoing, received>>
  /\ UNCHANGED <<up, restarts, lastKind, logWakes, ended, wsArchived>>

\* The relay and the actor store the wake's UserMessage.
StoreEcho ==
  /\ echoing
  /\ echoing' = FALSE /\ lastKind' = "wake" /\ logWakes' = logWakes + 1
  /\ UNCHANGED <<up, restarts, alive, rowLive, ended, pendingWake, waker,
                 wsArchived, received, wokeArchived>>

ClaudeFinishes ==
  /\ alive /\ ~echoing /\ lastKind \in {"user", "wake"}
  /\ lastKind' = "done" /\ logWakes' = 0 /\ received' = 0
  /\ UNCHANGED <<up, restarts, alive, rowLive, echoing, ended, pendingWake, waker,
                 wsArchived, wokeArchived>>

\* archive, under the workstream's lock: stop_for_archive (end_quietly_and_await_exit on each
\* live session: settle idle, ended_at), then the status.
Archive ==
  /\ ~wsArchived
  /\ wsArchived' = TRUE
  /\ IF alive THEN alive' = FALSE /\ ended' = TRUE /\ rowLive' = FALSE /\ echoing' = FALSE
              ELSE UNCHANGED <<alive, ended, rowLive, echoing>>
  /\ UNCHANGED <<up, restarts, lastKind, logWakes, pendingWake, waker, received,
                 wokeArchived>>

\* Nothing more happens without another restart: the waker is done and the echo stored.
Done == waker \in {"idle", "done"} /\ ~echoing /\ UNCHANGED vars

Next == Restart \/ WakerCheck \/ WakerWake \/ StoreEcho \/ ClaudeFinishes \/ Archive \/ Done

Spec == Init /\ [][Next]_vars

\* A session cut off mid-turn is woken at most MaxRestartResumes times before it finishes a turn.
WakeBounded == received <= MaxRestartResumes

\* No wake starts a harness in an archived workstream.
NoWakeWhenArchived == ~wokeArchived

TypeOK ==
  /\ up \in BOOLEAN /\ alive \in BOOLEAN /\ rowLive \in BOOLEAN
  /\ lastKind \in {"user", "wake", "done"} /\ logWakes \in Nat
  /\ waker \in {"idle", "start", "due", "done"} /\ received \in Nat
=============================================================================
