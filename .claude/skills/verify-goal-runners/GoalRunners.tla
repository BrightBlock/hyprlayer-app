------------------------------ MODULE GoalRunners ------------------------------
(* One goal, its runner registry, the service calls that pause, stop and resume
   it, server restarts, and the two tasks that pick work up after a restart.
   Each action names the code it stands for; see SKILL.md for the coverage map. *)
EXTENDS Naturals, Sequences

CONSTANTS MaxIter, MaxRestarts

MaxGen == 2 + MaxRestarts
Gens == 1..MaxGen
Free == 0
P == MaxGen + 1
S == MaxGen + 2
R == MaxGen + 3
SR == MaxGen + 4

Dead == {"absent", "exited", "dead"}
Driving == {"open", "resume", "boundary", "gate", "working", "checking",
            "c_fin", "c_lock", "c_drain", "c_write", "lift"}

VARIABLES
  status, step, iter,
  liveGen, nextGen,
  rpc, mode, rstep, earlier, fin, pend, inbox, how,
  lock,
  ppc, ptries, pwait,
  spc, stries, swait,
  rspc,
  srv, srvStep,
  waker, cut, woken, goalResumed,
  restarts,
  pauseAcked, sendsSincePause, stopAcked, sendsSinceStop

runnerVars == <<rpc, mode, rstep, earlier, fin, pend, inbox, how>>
vars == <<status, step, iter, liveGen, nextGen, runnerVars, lock,
          ppc, ptries, pwait, spc, stries, swait, rspc, srv, srvStep,
          waker, cut, woken, goalResumed, restarts,
          pauseAcked, sendsSincePause, stopAcked, sendsSinceStop>>
ackVars == <<pauseAcked, sendsSincePause, stopAcked, sendsSinceStop>>
restartVars == <<srv, srvStep, waker, cut, woken, goalResumed, restarts>>
svcVars == <<ppc, ptries, pwait, spc, stries, swait, rspc>>

Init ==
  /\ status = "running" /\ step = "working" /\ iter = 0
  /\ liveGen = 1 /\ nextGen = 2
  /\ rpc = [g \in Gens |-> IF g = 1 THEN "chain" ELSE "absent"]
  /\ mode = [g \in Gens |-> "normal"]
  /\ rstep = [g \in Gens |-> "none"]
  /\ earlier = [g \in Gens |-> 0]
  /\ fin = [g \in Gens |-> FALSE]
  /\ pend = [g \in Gens |-> FALSE]
  /\ inbox = [g \in Gens |-> <<>>]
  /\ how = [g \in Gens |-> "none"]
  /\ lock = Free
  /\ ppc = "idle" /\ ptries = 0 /\ pwait = 0
  /\ spc = "idle" /\ stries = 0 /\ swait = 0
  /\ rspc = "idle"
  /\ srv = "off" /\ srvStep = "none"
  /\ waker = "off" /\ cut = FALSE /\ woken = FALSE /\ goalResumed = FALSE
  /\ restarts = 0
  /\ pauseAcked = FALSE /\ sendsSincePause = 0
  /\ stopAcked = FALSE /\ sendsSinceStop = 0

(* GoalRunners::spawn, under the `live` lock: a live runner is left alone; a
   finishing one is followed by a new runner that waits for its exit. *)
Spawn(m, s) ==
  IF liveGen # 0 /\ ~fin[liveGen]
  THEN UNCHANGED <<liveGen, nextGen, runnerVars>>
  ELSE LET g == nextGen IN
       /\ g <= MaxGen
       /\ liveGen' = g
       /\ nextGen' = g + 1
       /\ rpc' = [rpc EXCEPT ![g] = "chain"]
       /\ earlier' = [earlier EXCEPT ![g] = liveGen]
       /\ mode' = [mode EXCEPT ![g] = m]
       /\ rstep' = [rstep EXCEPT ![g] = s]
       /\ UNCHANGED <<fin, pend, inbox, how>>

Goto(g, pc) == rpc' = [rpc EXCEPT ![g] = pc]

(* Run::conclude is entered *)
Conclude(g, h) == /\ Goto(g, "c_fin") /\ how' = [how EXCEPT ![g] = h]

CountSend ==
  /\ sendsSincePause' = IF pauseAcked THEN sendsSincePause + 1 ELSE sendsSincePause
  /\ sendsSinceStop' = IF stopAcked THEN sendsSinceStop + 1 ELSE sendsSinceStop

----------------------------------------------------------------------------
(* The runner task, goal_loop::run *)

(* TrackedTask body: `while earlier.changed().await.is_ok() {}` *)
RunnerChain(g) ==
  /\ rpc[g] = "chain"
  /\ IF earlier[g] = 0 THEN TRUE ELSE rpc[earlier[g]] \in Dead
  /\ Goto(g, "open")
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, inbox, how, lock, svcVars, restartVars, ackVars>>

(* Run::open: under `transitions`, a goal that is not running has nothing to
   drive, and the runner is marked finishing before the lock drops. *)
RunnerOpen(g) ==
  /\ rpc[g] = "open"
  /\ lock = Free
  /\ IF status # "running"
     THEN /\ Goto(g, "exit")
          /\ fin' = [fin EXCEPT ![g] = TRUE]
     ELSE /\ Goto(g, IF mode[g] = "resume" THEN "resume" ELSE "boundary")
          /\ UNCHANGED fin
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier,
                 pend, inbox, how, lock, svcVars, restartVars, ackVars>>

(* Run::resume: picks up from the step the goal went down on *)
RunnerResume(g) ==
  /\ rpc[g] = "resume"
  /\ CASE rstep[g] = "checking" /\ iter > 0 -> Goto(g, "checking") /\ UNCHANGED how
       [] rstep[g] = "pausing" -> Conclude(g, "pause")
       [] rstep[g] = "stopping" -> Conclude(g, "end_user")
       [] OTHER -> Goto(g, "boundary") /\ UNCHANGED how
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, inbox, lock, svcVars, restartVars, ackVars>>

(* Run::apply for the control at the head of the inbox, from boundary's
   drain_controls or the select in working/check.
   Pause -> request_pause: Pausing is written only over Working (or Waiting).
   Stop  -> writes Stopping and aborts into conclude. *)
RunnerApply(g) ==
  /\ rpc[g] \in {"boundary", "working", "checking"}
  /\ inbox[g] # <<>>
  /\ inbox' = [inbox EXCEPT ![g] = Tail(inbox[g])]
  /\ IF Head(inbox[g]) = "pause"
     THEN /\ pend' = [pend EXCEPT ![g] = TRUE]
          /\ step' = IF step = "working" THEN "pausing" ELSE step
          /\ UNCHANGED <<rpc, how>>
     ELSE /\ step' = "stopping"
          /\ Conclude(g, "end_user")
          /\ UNCHANGED pend
  /\ UNCHANGED <<status, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 lock, svcVars, restartVars, ackVars>>

RunnerToGate(g) ==
  /\ rpc[g] = "boundary"
  /\ inbox[g] = <<>>
  /\ Goto(g, "gate")
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, inbox, how, lock, svcVars, restartVars, ackVars>>

(* Run::gate: a goal at its iteration limit ends; a pending pause concludes;
   otherwise Run::send starts iteration iter+1 and writes step Working. *)
RunnerGate(g) ==
  /\ rpc[g] = "gate"
  /\ CASE iter >= MaxIter ->
            /\ Conclude(g, "end_other")
            /\ UNCHANGED <<step, iter, sendsSincePause, sendsSinceStop>>
       [] pend[g] ->
            /\ Conclude(g, "pause")
            /\ UNCHANGED <<step, iter, sendsSincePause, sendsSinceStop>>
       [] OTHER ->
            /\ iter' = iter + 1
            /\ step' = "working"
            /\ Goto(g, "working")
            /\ CountSend
            /\ UNCHANGED how
  /\ UNCHANGED <<status, liveGen, nextGen, mode, rstep, earlier, fin, pend, inbox,
                 lock, svcVars, restartVars, pauseAcked, stopAcked>>

(* Run::after -> close_iteration(.., Some(Checking)): the step is set to
   Checking whatever it was, Pausing included. *)
RunnerTurnEnds(g) ==
  /\ rpc[g] = "working"
  /\ step' = "checking"
  /\ Goto(g, "checking")
  /\ UNCHANGED <<status, iter, liveGen, nextGen, mode, rstep, earlier, fin, pend,
                 inbox, how, lock, svcVars, restartVars, ackVars>>

(* Run::judge -> decide_boundary: met, then the iteration limit, then a pause *)
RunnerJudge(g) ==
  /\ rpc[g] = "checking"
  /\ \/ Conclude(g, "end_other")
     \/ /\ iter >= MaxIter /\ Conclude(g, "end_other")
     \/ /\ iter < MaxIter /\ pend[g] /\ Conclude(g, "pause")
     \/ /\ iter < MaxIter /\ ~pend[g] /\ Goto(g, "boundary") /\ UNCHANGED how
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, inbox, lock, svcVars, restartVars, ackVars>>

(* Lease::finishing, under the `live` lock *)
RunnerFinishing(g) ==
  /\ rpc[g] = "c_fin"
  /\ fin' = [fin EXCEPT ![g] = TRUE]
  /\ Goto(g, "c_lock")
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, pend,
                 inbox, how, lock, svcVars, restartVars, ackVars>>

(* Lease::transition: the shared `transitions` mutex *)
RunnerLock(g) ==
  /\ rpc[g] = "c_lock"
  /\ lock = Free
  /\ lock' = g
  /\ Goto(g, "c_drain")
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, inbox, how, svcVars, restartVars, ackVars>>

(* conclude's try_recv loop: a stop that arrived while pausing wins *)
RunnerDrain(g) ==
  /\ rpc[g] = "c_drain"
  /\ how' = [how EXCEPT ![g] =
       IF how[g] = "pause" /\ \E i \in 1..Len(inbox[g]) : inbox[g][i] = "stop"
       THEN "end_user" ELSE how[g]]
  /\ inbox' = [inbox EXCEPT ![g] = <<>>]
  /\ Goto(g, "c_write")
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, lock, svcVars, restartVars, ackVars>>

(* conclude's write of Conclusion::update, then the guard drops *)
RunnerWrite(g) ==
  /\ rpc[g] = "c_write"
  /\ status' = IF how[g] = "pause" THEN "paused" ELSE "ended"
  /\ step' = "none"
  /\ lock' = Free
  /\ Goto(g, "lift")
  /\ UNCHANGED <<iter, liveGen, nextGen, mode, rstep, earlier, fin, pend, inbox,
                 how, svcVars, restartVars, ackVars>>

(* Run::lift_cap runs after the conclusion, before the task ends *)
RunnerLift(g) ==
  /\ rpc[g] = "lift"
  /\ Goto(g, "exit")
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, mode, rstep, earlier, fin,
                 pend, inbox, how, lock, svcVars, restartVars, ackVars>>

(* Registration::drop removes the entry if it is still this generation's;
   dropping `done` closes `exited`. *)
RunnerExit(g) ==
  /\ rpc[g] = "exit"
  /\ liveGen' = IF liveGen = g THEN 0 ELSE liveGen
  /\ Goto(g, "exited")
  /\ UNCHANGED <<status, step, iter, nextGen, mode, rstep, earlier, fin, pend,
                 inbox, how, lock, svcVars, restartVars, ackVars>>

----------------------------------------------------------------------------
(* GoalService::pause -> try_pause, at most twice, with wait_exit between *)

PauseStart ==
  /\ ppc = "idle"
  /\ ppc' = "lock"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, ptries,
                 pwait, spc, stries, swait, rspc, restartVars, ackVars>>

PauseLock ==
  /\ ppc = "lock" /\ lock = Free
  /\ lock' = P /\ ppc' = "act"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, ptries, pwait,
                 spc, stries, swait, rspc, restartVars, ackVars>>

AckPause == pauseAcked' = TRUE /\ sendsSincePause' = 0

(* Load and send are one step: under `transitions` only this call can change
   the status; the runner changes only the step. *)
PauseAct ==
  /\ ppc = "act"
  /\ lock' = Free
  /\ CASE status = "paused" ->
            /\ ppc' = "done" /\ AckPause
            /\ UNCHANGED <<status, step, inbox, pwait>>
       [] status = "running" /\ liveGen = 0 ->
            /\ status' = "paused" /\ step' = "none"
            /\ ppc' = "done" /\ AckPause
            /\ UNCHANGED <<inbox, pwait>>
       [] status = "running" /\ liveGen # 0 /\ fin[liveGen] ->
            /\ ppc' = "wait" /\ pwait' = liveGen
            /\ UNCHANGED <<status, step, inbox, pauseAcked, sendsSincePause>>
       [] status = "running" /\ liveGen # 0 ->
            /\ inbox' = [inbox EXCEPT ![liveGen] = Append(@, "pause")]
            /\ ppc' = "done" /\ AckPause
            /\ UNCHANGED <<status, step, pwait>>
       [] OTHER ->
            /\ ppc' = "done"
            /\ UNCHANGED <<status, step, inbox, pwait, pauseAcked, sendsSincePause>>
  /\ UNCHANGED <<iter, liveGen, nextGen, rpc, mode, rstep, earlier, fin, pend, how,
                 ptries, spc, stries, swait, rspc, restartVars, stopAcked,
                 sendsSinceStop>>

(* wait_exit returns on the runner's exit or after FINISHING_WAIT *)
PauseWait ==
  /\ ppc = "wait"
  /\ ppc' = IF ptries < 1 THEN "lock" ELSE "done"
  /\ ptries' = ptries + 1
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, pwait,
                 spc, stries, swait, rspc, restartVars, ackVars>>

----------------------------------------------------------------------------
(* GoalService::stop -> end(StoppedByUser) -> try_end *)

StopStart ==
  /\ spc = "idle"
  /\ spc' = "lock"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, ppc,
                 ptries, pwait, stries, swait, rspc, restartVars, ackVars>>

StopLock ==
  /\ spc = "lock" /\ lock = Free
  /\ lock' = S /\ spc' = "act"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, ppc, ptries,
                 pwait, stries, swait, rspc, restartVars, ackVars>>

AckStop == stopAcked' = TRUE /\ sendsSinceStop' = 0

StopAct ==
  /\ spc = "act"
  /\ lock' = Free
  /\ CASE status = "ended" ->
            /\ spc' = "done" /\ AckStop
            /\ UNCHANGED <<status, step, inbox, swait>>
       [] status = "running" /\ liveGen # 0 /\ fin[liveGen] ->
            /\ spc' = "wait" /\ swait' = liveGen
            /\ UNCHANGED <<status, step, inbox, stopAcked, sendsSinceStop>>
       [] status = "running" /\ liveGen # 0 ->
            /\ inbox' = [inbox EXCEPT ![liveGen] = Append(@, "stop")]
            /\ spc' = "done" /\ AckStop
            /\ UNCHANGED <<status, step, swait>>
       [] OTHER ->
            /\ status' = "ended" /\ step' = "none"
            /\ spc' = "done" /\ AckStop
            /\ UNCHANGED <<inbox, swait>>
  /\ UNCHANGED <<iter, liveGen, nextGen, rpc, mode, rstep, earlier, fin, pend, how,
                 ppc, ptries, pwait, stries, rspc, restartVars, pauseAcked,
                 sendsSincePause>>

StopWait ==
  /\ spc = "wait"
  /\ spc' = IF stries < 1 THEN "lock" ELSE "done"
  /\ stries' = stries + 1
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, ppc,
                 ptries, pwait, swait, rspc, restartVars, ackVars>>

----------------------------------------------------------------------------
(* GoalService::resume: under `transitions`, a paused goal is written running;
   the spawn follows outside the lock. *)

ResumeStart ==
  /\ rspc = "idle"
  /\ rspc' = "lock"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, ppc,
                 ptries, pwait, spc, stries, swait, restartVars, ackVars>>

ResumeLock ==
  /\ rspc = "lock" /\ lock = Free
  /\ lock' = R /\ rspc' = "act"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, ppc, ptries,
                 pwait, spc, stries, swait, restartVars, ackVars>>

ResumeAct ==
  /\ rspc = "act"
  /\ lock' = Free
  /\ CASE status = "paused" ->
            /\ status' = "running" /\ step' = "working"
            /\ pauseAcked' = FALSE /\ sendsSincePause' = 0
            /\ rspc' = "spawn"
       [] status = "running" ->
            /\ rspc' = "spawn"
            /\ UNCHANGED <<status, step, pauseAcked, sendsSincePause>>
       [] OTHER ->
            /\ rspc' = "done"
            /\ UNCHANGED <<status, step, pauseAcked, sendsSincePause>>
  /\ UNCHANGED <<iter, liveGen, nextGen, runnerVars, ppc, ptries, pwait, spc,
                 stries, swait, restartVars, stopAcked, sendsSinceStop>>

ResumeSpawn ==
  /\ rspc = "spawn"
  /\ Spawn("normal", "none")
  /\ rspc' = "done"
  /\ UNCHANGED <<status, step, iter, lock, ppc, ptries, pwait, spc, stries, swait,
                 restartVars, ackVars>>

----------------------------------------------------------------------------
(* A server restart: runners, channels, locks and calls in flight are gone;
   the database row stays. The new server starts spawn_resumer and spawn_waker.
   `cut` is whether a turn of the person's own was cut off in the session. *)

Restart ==
  /\ restarts < MaxRestarts
  /\ restarts' = restarts + 1
  /\ rpc' = [g \in Gens |-> IF rpc[g] \in Dead THEN rpc[g] ELSE "dead"]
  /\ liveGen' = 0
  /\ inbox' = [g \in Gens |-> <<>>]
  /\ lock' = Free
  /\ ppc' = IF ppc = "idle" THEN "idle" ELSE "done"
  /\ spc' = IF spc = "idle" THEN "idle" ELSE "done"
  /\ rspc' = IF rspc = "idle" THEN "idle" ELSE "done"
  /\ srv' = "lock"
  /\ waker' = "check"
  /\ cut' \in BOOLEAN
  /\ woken' = FALSE
  /\ goalResumed' = FALSE
  /\ UNCHANGED <<status, step, iter, nextGen, mode, rstep, earlier, fin, pend, how,
                 ptries, pwait, stries, swait, srvStep, ackVars>>

(* resume_one: under `transitions`, a running goal with no runner is written
   Resuming and spawned in Resume mode with the step it read. *)
SrvLock ==
  /\ srv = "lock" /\ lock = Free
  /\ lock' = SR /\ srv' = "act"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, svcVars,
                 srvStep, waker, cut, woken, goalResumed, restarts, ackVars>>

SrvAct ==
  /\ srv = "act"
  /\ lock' = Free
  /\ IF status = "running" /\ liveGen = 0
     THEN /\ srvStep' = step /\ step' = "resuming" /\ srv' = "spawn"
     ELSE /\ srv' = "done" /\ UNCHANGED <<srvStep, step>>
  /\ UNCHANGED <<status, iter, liveGen, nextGen, runnerVars, svcVars, waker, cut,
                 woken, goalResumed, restarts, ackVars>>

SrvSpawn ==
  /\ srv = "spawn"
  /\ Spawn("resume", srvStep)
  /\ srv' = "done" /\ goalResumed' = TRUE
  /\ UNCHANGED <<status, step, iter, lock, svcVars, srvStep, waker, cut, woken,
                 restarts, ackVars>>

(* wake::check: a session whose goal is running or paused is the goal resumer's *)
WakerCheck ==
  /\ waker = "check"
  /\ waker' = IF cut /\ status \notin {"running", "paused"} THEN "wake" ELSE "done"
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, svcVars,
                 srv, srvStep, cut, woken, goalResumed, restarts, ackVars>>

WakerWake ==
  /\ waker = "wake"
  /\ waker' = "done" /\ woken' = TRUE
  /\ UNCHANGED <<status, step, iter, liveGen, nextGen, runnerVars, lock, svcVars,
                 srv, srvStep, cut, goalResumed, restarts, ackVars>>

----------------------------------------------------------------------------

Quiet ==
  /\ \A g \in Gens : rpc[g] \in Dead
  /\ ppc \in {"idle", "done"} /\ spc \in {"idle", "done"} /\ rspc \in {"idle", "done"}
  /\ srv \in {"off", "done"} /\ waker \in {"off", "done"}

Done == Quiet /\ UNCHANGED vars

Next ==
  \/ \E g \in Gens :
       \/ RunnerChain(g) \/ RunnerOpen(g) \/ RunnerResume(g) \/ RunnerApply(g)
       \/ RunnerToGate(g) \/ RunnerGate(g) \/ RunnerTurnEnds(g) \/ RunnerJudge(g)
       \/ RunnerFinishing(g) \/ RunnerLock(g) \/ RunnerDrain(g) \/ RunnerWrite(g)
       \/ RunnerLift(g) \/ RunnerExit(g)
  \/ PauseStart \/ PauseLock \/ PauseAct \/ PauseWait
  \/ StopStart \/ StopLock \/ StopAct \/ StopWait
  \/ ResumeStart \/ ResumeLock \/ ResumeAct \/ ResumeSpawn
  \/ Restart \/ SrvLock \/ SrvAct \/ SrvSpawn \/ WakerCheck \/ WakerWake
  \/ Done

Spec == Init /\ [][Next]_vars

----------------------------------------------------------------------------
(* The rules *)

TypeOK ==
  /\ status \in {"running", "paused", "ended"}
  /\ liveGen \in 0..MaxGen
  /\ lock \in 0..SR

(* At most one runner drives the goal: it is the only writer of its state. *)
SingleDriver ==
  \A g, h \in Gens : g # h => ~(rpc[g] \in Driving /\ rpc[h] \in Driving)

(* A pause the person was told about lets at most the iteration being sent
   go out; then the goal pauses. *)
PauseHonored == pauseAcked => sendsSincePause <= 1

(* A stop the person was told about lets at most the iteration being sent go
   out, and the goal never shows paused after it. *)
StopHonored == stopAcked => (sendsSinceStop <= 1 /\ status # "paused")

(* The goal never runs more iterations than its limit. *)
IterationCap == iter <= MaxIter

(* Once everything has settled, a running goal has a runner. *)
NoStrandedGoal == Quiet => status # "running"

(* A session cut off by a restart mid-turn is picked up again: woken by the
   waker, or resumed by the goal resumer. *)
CutOffSessionPickedUp == (Quiet /\ cut) => (woken \/ goalResumed)

=============================================================================
