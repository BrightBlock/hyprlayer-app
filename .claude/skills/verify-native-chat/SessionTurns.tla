---------------------------- MODULE SessionTurns ----------------------------
(* One composer session on claude: SessionService::send / respond_permission /
   interrupt, ClaudeDriver's inbox, write_loop and read_and_wait, the relay and
   the forwarder actor, and one desktop's sessionStore. SKILL.md maps every
   action to the code. *)
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS Composer,   \* messages the desktop's composer sends
          Agent,      \* messages an agent (message_agent, a wake) sends
          Asks,       \* permission request ids claude may ask
          MaxProcs,   \* claude processes: the launch plus resumes
          MaxExits,
          MaxInterrupts

Msgs == Composer \cup Agent

VARIABLES
  \* the claude process
  alive, busy, turn, pipe, queue, cOpen, asked, out,
  \* ClaudeDriver
  dBusy, lifecycle, waiting, pending, stdinOpen, bus,
  \* SessionRegistry, the relay and the actor
  live, procs, exits, inTurn, stored,
  \* the desktop
  vLive, streaming, vOpen, clicked, chip, sends,
  \* bookkeeping
  accepted, failed, delivered, interrupts

vars == <<alive, busy, turn, pipe, queue, cOpen, asked, out,
          dBusy, lifecycle, waiting, pending, stdinOpen, bus,
          live, procs, exits, inTurn, stored,
          vLive, streaming, vOpen, clicked, chip, sends,
          accepted, failed, delivered, interrupts>>

claudeVars == <<alive, busy, turn, pipe, queue, cOpen, asked, out>>
driverVars == <<dBusy, lifecycle, waiting, pending, stdinOpen, bus>>
viewVars == <<vLive, streaming, vOpen, clicked, chip, sends>>

InSeq(x, s) == \E i \in 1..Len(s) : s[i] = x

Init ==
  /\ alive = TRUE /\ busy = FALSE /\ turn = 0 /\ pipe = <<>> /\ queue = <<>>
  /\ cOpen = {} /\ asked = {} /\ out = <<>>
  /\ dBusy = FALSE /\ lifecycle = FALSE /\ waiting = {} /\ pending = {} /\ stdinOpen = TRUE /\ bus = <<>>
  /\ live = TRUE /\ procs = 1 /\ exits = 0 /\ inTurn = FALSE /\ stored = {}
  /\ vLive = TRUE /\ streaming = FALSE /\ vOpen = {} /\ clicked = {}
  /\ chip = [m \in Composer |-> "none"] /\ sends = [m \in Composer |-> 0]
  /\ accepted = {} /\ failed = {} /\ delivered = [m \in Msgs |-> 0]
  /\ interrupts = 0

(* ---- ClaudeDriver::send_user_turn plus write_line, under the op lock ---- *)

\* The inbox holds the echo only when it is not idle() and claude has sent a lifecycle frame.
EchoOrHold(m) ==
  IF (~dBusy /\ waiting = {}) \/ ~lifecycle
    THEN /\ dBusy' = TRUE /\ waiting' = waiting /\ bus' = Append(bus, <<"echo", m>>)
    ELSE /\ dBusy' = dBusy /\ waiting' = waiting \cup {m} /\ bus' = bus

\* write_line succeeds while write_loop's receiver lives; a dead process loses the line.
WriteOk(line) ==
  /\ stdinOpen
  /\ IF alive THEN pipe' = Append(pipe, line) /\ stdinOpen' = TRUE
              ELSE pipe' = pipe /\ stdinOpen' \in BOOLEAN

\* Returns "ok", "err" (stdin closed) or "gone" (no live driver).
SendServer(m, result) ==
  IF ~live
    THEN result = "gone" /\ UNCHANGED <<driverVars, pipe>>
  ELSE IF stdinOpen
    THEN result = "ok" /\ EchoOrHold(m) /\ WriteOk(<<"msg", m>>)
         /\ pending' = pending /\ lifecycle' = lifecycle
  ELSE \* the echo decision runs before the write; Err takes a held echo back
    /\ result = "err"
    /\ IF (~dBusy /\ waiting = {}) \/ ~lifecycle
         THEN dBusy' = TRUE /\ bus' = Append(bus, <<"echo", m>>)
         ELSE dBusy' = dBusy /\ bus' = bus
    /\ waiting' = waiting /\ pending' = pending /\ stdinOpen' = stdinOpen /\ pipe' = pipe
    /\ lifecycle' = lifecycle

Outcome(m, result, kept) ==
  /\ accepted' = IF result = "ok" THEN accepted \cup {m} ELSE accepted
  /\ failed' = IF result = "err" \/ (result = "gone" /\ ~kept) THEN failed \cup {m} ELSE failed

(* ---- the desktop's sends: deliverMessage (direct) and steerMessage ---- *)

\* A first send, or a chip handed back on an exit or relaunch, flushed once the view is idle.
ClientSend(m) ==
  /\ m \in Composer /\ vLive /\ sends[m] < 2
  /\ \/ chip[m] = "none" /\ sends[m] = 0
     \/ chip[m] = "queued" /\ ~streaming
  /\ LET steer == streaming /\ chip[m] = "none" IN
     \E result \in {"ok", "err", "gone"} :
       /\ SendServer(m, result)
       \* steer keeps a chip until its echo; session_not_found keeps it queued
       /\ chip' = [chip EXCEPT ![m] = IF steer /\ result = "ok" THEN "delivered"
                                     ELSE IF steer /\ result = "gone" THEN "queued"
                                     ELSE "none"]
       /\ Outcome(m, result, steer)
       \* direct: an optimistic row, and the view shows the turn running
       /\ streaming' = (streaming \/ (~steer /\ result = "ok"))
  /\ sends' = [sends EXCEPT ![m] = @ + 1]
  /\ UNCHANGED <<alive, busy, turn, queue, cOpen, asked, out, live, procs, exits,
                 inTurn, stored, vLive, vOpen, clicked, delivered,
                 interrupts>>

AgentSend(m) ==
  /\ m \in Agent /\ m \notin accepted \cup failed
  /\ \E result \in {"ok", "err", "gone"} : SendServer(m, result) /\ Outcome(m, result, FALSE)
  /\ UNCHANGED <<alive, busy, turn, queue, cOpen, asked, out, live, procs, exits,
                 inTurn, stored, viewVars, delivered, interrupts>>

(* ---- respond_permission and interrupt ---- *)

\* You answer an ask once.
ClientAnswer(r) ==
  /\ r \in vOpen \ clicked /\ live /\ stdinOpen
  /\ clicked' = clicked \cup {r}
  /\ WriteOk(<<"answer", r>>)
  /\ IF r \in pending
       THEN pending' = pending \ {r} /\ bus' = Append(bus, <<"resolved", r>>)
       ELSE pending' = pending /\ bus' = bus
  /\ UNCHANGED <<alive, busy, turn, queue, cOpen, asked, out, dBusy, lifecycle, waiting,
                 live, procs, exits, inTurn, stored, vLive, streaming, vOpen, chip,
                 sends, accepted, failed, delivered, interrupts>>

\* SessionService::interrupt from anyone: the composer's stop (which declines a parked ask
\* first), an agent's stop_agent, a goal.
Interrupt ==
  /\ interrupts < MaxInterrupts /\ live /\ stdinOpen
  /\ WriteOk(<<"interrupt">>)
  /\ interrupts' = interrupts + 1
  /\ UNCHANGED <<alive, busy, turn, queue, cOpen, asked, out, dBusy, lifecycle, waiting,
                 pending, bus, live, procs, exits, inTurn, stored, viewVars,
                 accepted, failed, delivered>>

(* ---- claude ---- *)

Take(m) == delivered' = [delivered EXCEPT ![m] = @ + 1]

\* Claude settles an ask without us: control_cancel_request, then the tool's result (an error
\* when it was interrupted), as real claude 2.1.330 writes them.
RECURSIVE Withdraw(_)
Withdraw(S) ==
  IF S = {} THEN <<>>
  ELSE LET r == CHOOSE x \in S : TRUE
       IN <<<<"cancel", r>>, <<"toolResult", r>>>> \o Withdraw(S \ {r})

ClaudeRead ==
  /\ alive /\ pipe /= <<>>
  /\ LET line == Head(pipe) IN
     /\ pipe' = Tail(pipe)
     /\ CASE line[1] = "msg" /\ ~busy ->
               /\ busy' = TRUE /\ turn' = turn + 1 /\ Take(line[2])
               /\ out' = out \o <<<<"started", line[2]>>, <<"init">>>>
               /\ UNCHANGED <<queue, cOpen>>
          [] line[1] = "msg" ->
               /\ queue' = Append(queue, line[2])
               /\ UNCHANGED <<busy, turn, out, cOpen, delivered>>
          [] line[1] = "interrupt" /\ busy ->
               \* claude cancels its open asks (control_cancel_request) and ends the turn
               /\ busy' = FALSE /\ cOpen' = {}
               /\ out' = out \o Withdraw(cOpen) \o <<<<"result">>>>
               /\ UNCHANGED <<turn, queue, delivered>>
          [] line[1] = "interrupt" ->
               UNCHANGED <<busy, turn, queue, cOpen, out, delivered>>
          [] line[1] = "answer" /\ line[2] \in cOpen ->
               \* the tool runs, or is denied: either way its result follows
               /\ cOpen' = cOpen \ {line[2]}
               /\ out' = Append(out, <<"toolResult", line[2]>>)
               /\ UNCHANGED <<busy, turn, queue, delivered>>
          [] line[1] = "answer" ->
               UNCHANGED <<busy, turn, queue, cOpen, out, delivered>>
  /\ UNCHANGED <<alive, asked, driverVars, live, procs, exits, inTurn, stored,
                 viewVars, accepted, failed, interrupts>>

ClaudeAsk(r) ==
  /\ alive /\ busy /\ r \notin asked
  /\ asked' = asked \cup {r} /\ cOpen' = cOpen \cup {r}
  /\ out' = Append(out, <<"ask", r>>)
  /\ UNCHANGED <<alive, busy, turn, pipe, queue, driverVars, live, procs, exits,
                 inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts>>

\* A message sent mid-turn joins the turn at claude's next step.
ClaudeTakeQueued ==
  /\ alive /\ busy /\ queue /= <<>> /\ cOpen = {}
  /\ Take(Head(queue)) /\ queue' = Tail(queue)
  /\ out' = Append(out, <<"started", Head(queue)>>)
  /\ UNCHANGED <<alive, busy, turn, pipe, cOpen, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed,
                 interrupts>>

\* Remote Control: the ask is answered on claude.ai, and claude withdraws its can_use_tool.
ClaudeRemoteAnswer(r) ==
  /\ alive /\ r \in cOpen
  /\ cOpen' = cOpen \ {r}
  /\ out' = out \o Withdraw({r})
  /\ UNCHANGED <<alive, busy, turn, pipe, queue, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts>>

ClaudeFinish ==
  /\ alive /\ busy /\ cOpen = {}
  /\ busy' = FALSE /\ out' = Append(out, <<"result">>)
  /\ UNCHANGED <<alive, turn, pipe, queue, cOpen, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts>>

\* claude starts a waiting message right after a result.
ClaudeStartQueued ==
  /\ alive /\ ~busy /\ queue /= <<>>
  /\ busy' = TRUE /\ turn' = turn + 1
  /\ Take(Head(queue)) /\ queue' = Tail(queue)
  /\ out' = out \o <<<<"started", Head(queue)>>, <<"init">>>>
  /\ UNCHANGED <<alive, pipe, cOpen, asked, driverVars, live, procs, exits, inTurn,
                 stored, viewVars, accepted, failed, interrupts>>

\* A crash, a kill, or the process ending: unread lines and claude's queue die with it.
ClaudeExit ==
  /\ alive /\ exits < MaxExits
  /\ alive' = FALSE /\ busy' = FALSE /\ cOpen' = {} /\ pipe' = <<>> /\ queue' = <<>>
  /\ out' = Append(out, <<"eof">>)
  /\ exits' = exits + 1
  /\ UNCHANGED <<turn, asked, driverVars, live, procs, inTurn, stored, viewVars,
                 accepted, failed, delivered, interrupts>>

(* ---- read_and_wait ---- *)

DriverRead ==
  /\ out /= <<>>
  /\ LET line == Head(out) IN
     /\ out' = Tail(out)
     /\ CASE line[1] = "init" ->
               dBusy' = TRUE /\ bus' = Append(bus, <<"init">>) /\ UNCHANGED <<waiting, pending>>
          [] line[1] = "started" ->
               \* Inbox::frame: our own message's frame
               /\ dBusy' = TRUE /\ lifecycle' = TRUE
               /\ IF line[2] \in waiting
                    THEN waiting' = waiting \ {line[2]} /\ bus' = Append(bus, <<"echo", line[2]>>)
                    ELSE waiting' = waiting /\ bus' = bus
               /\ pending' = pending
          [] line[1] = "result" ->
               dBusy' = FALSE /\ bus' = Append(bus, <<"turnDone">>) /\ UNCHANGED <<waiting, pending>>
          [] line[1] = "ask" ->
               pending' = pending \cup {line[2]} /\ bus' = Append(bus, <<"ask", line[2]>>)
               /\ UNCHANGED <<dBusy, waiting>>
          [] line[1] = "cancel" ->
               \* ParsedLine::ControlCancel: the ask leaves pending_permissions; nothing is emitted
               pending' = pending \ {line[2]} /\ UNCHANGED <<dBusy, waiting, bus>>
          [] line[1] = "toolResult" ->
               bus' = Append(bus, <<"toolResult", line[2]>>) /\ UNCHANGED <<dBusy, waiting, pending>>
          [] line[1] = "eof" ->
               bus' = Append(bus, <<"exited">>) /\ UNCHANGED <<dBusy, waiting, pending>>
  /\ Head(out)[1] /= "started" => lifecycle' = lifecycle
  /\ UNCHANGED <<alive, busy, turn, pipe, queue, cOpen, asked, stdinOpen, live,
                 procs, exits, inTurn, stored, viewVars, accepted, failed,
                 delivered, interrupts>>

(* ---- spawn_relay + ActorState::handle_event, and the desktop's applyEvent ----
   One step: the actor's fan-out reaches the desktop in order, and no rule needs the
   desktop to lag the actor (it lags the harness through `out` and `bus`). *)

Relay ==
  /\ bus /= <<>>
  /\ LET ev == Head(bus)
         handBack == [m \in Composer |-> IF chip[m] = "delivered" THEN "queued" ELSE chip[m]]
     IN
     /\ inTurn' = CASE ev[1] = "echo" -> TRUE
                   [] ev[1] \in {"turnDone", "exited"} -> FALSE
                   [] OTHER -> inTurn
     /\ stored' = IF ev[1] \in {"echo", "ask"} THEN stored \cup {ev[2]} ELSE stored
     \* on Exited the relay removes the driver from the registry and ends: later broadcasts reach no one
     /\ live' = IF ev[1] = "exited" THEN FALSE ELSE live
     /\ bus' = IF ev[1] = "exited" THEN <<>> ELSE Tail(bus)
     \* reduceEvent; on exited, expirePendingPrompts and endStream; handedBackBy on exited/relaunched
     \* settleAnsweredPrompt: a tool's result decides its open card and resumes the turn
     /\ streaming' = CASE ev[1] \in {"echo", "ask"} -> TRUE
                      [] ev[1] = "toolResult" /\ ev[2] \in vOpen -> TRUE
                      [] ev[1] \in {"turnDone", "exited"} -> FALSE
                      [] OTHER -> streaming
     /\ vOpen' = CASE ev[1] = "ask" -> vOpen \cup {ev[2]}
                  [] ev[1] \in {"resolved", "toolResult"} -> vOpen \ {ev[2]}
                  [] ev[1] = "exited" -> {}
                  [] OTHER -> vOpen
     /\ vLive' = IF ev[1] = "exited" THEN FALSE ELSE vLive
     /\ chip' = CASE ev[1] = "echo" /\ ev[2] \in Composer -> [chip EXCEPT ![ev[2]] = "none"]
                 [] ev[1] \in {"exited", "relaunched"} -> handBack
                 [] OTHER -> chip
  /\ UNCHANGED <<claudeVars, dBusy, lifecycle, waiting, pending, stdinOpen, procs, exits,
                 clicked, sends, accepted, failed, delivered, interrupts>>

\* A tail read refolds the log: list_turns re-sends the stored asks open_request_ids
\* (pending_requests) names, even when their results lie outside the page, and
\* settleUnwaitedAsks closes every other one.
ClientReload ==
  /\ vLive /\ live
  /\ vOpen' = pending \cap stored
  /\ UNCHANGED <<claudeVars, driverVars, live, procs, exits, inTurn, stored,
                 vLive, streaming, clicked, chip, sends, accepted, failed, delivered,
                 interrupts>>

\* ResumeSession on a session with no live harness: launch_driver, a fresh driver, Relaunched.
ClientResume ==
  /\ ~vLive /\ ~live /\ procs < MaxProcs
  /\ alive' = TRUE /\ busy' = FALSE /\ pipe' = <<>> /\ queue' = <<>> /\ cOpen' = {}
  /\ out' = <<>>
  /\ dBusy' = FALSE /\ lifecycle' = FALSE /\ waiting' = {} /\ pending' = {} /\ stdinOpen' = TRUE
  /\ bus' = <<<<"relaunched">>>>
  /\ live' = TRUE /\ procs' = procs + 1 /\ vLive' = TRUE
  /\ UNCHANGED <<turn, asked, exits, inTurn, stored, streaming, vOpen, clicked, chip,
                 sends, accepted, failed, delivered, interrupts>>

(* ---- the rules ---- *)

\* Nothing left that could still move: no line or event in flight, claude idle or dead.
Quiet ==
  /\ out = <<>> /\ bus = <<>>
  /\ alive => (~busy /\ pipe = <<>> /\ queue = <<>>)

Done == Quiet /\ UNCHANGED vars

Next ==
  \/ \E m \in Composer : ClientSend(m)
  \/ \E m \in Agent : AgentSend(m)
  \/ \E r \in Asks : ClientAnswer(r) \/ ClaudeAsk(r) \/ ClaudeRemoteAnswer(r)
  \/ Interrupt
  \/ ClaudeRead \/ ClaudeTakeQueued \/ ClaudeFinish \/ ClaudeStartQueued \/ ClaudeExit
  \/ DriverRead \/ Relay \/ ClientReload \/ ClientResume \/ Done

Spec == Init /\ [][Next]_vars

OnItsWay(m) ==
  \/ alive /\ (InSeq(<<"msg", m>>, pipe) \/ InSeq(m, queue))
  \/ InSeq(<<"echo", m>>, bus)

\* Accepted (send returned Ok) and then: taken in by claude, still on its way, back in the
\* composer's queue, reported failed, or at least in the transcript.
Kept(m) ==
  \/ m \notin accepted
  \/ delivered[m] > 0
  \/ OnItsWay(m)
  \/ m \in Composer /\ chip[m] /= "none"
  \/ m \in failed
  \/ m \in stored

NoLostUserMessage == \A m \in Composer : Kept(m)
NoLostAgentMessage == \A m \in Agent : Kept(m)

NoDoubleSend == \A m \in Msgs : delivered[m] <= 1

TurnProgress ==
  Quiet => /\ ~inTurn /\ ~streaming
           /\ alive => (~dBusy /\ waiting = {})

\* Every ask the desktop shows open is one claude still waits on.
AskResolution == Quiet => vOpen \subseteq cOpen

\* After an interrupt, a live session settles idle: no turn, no held echo, and no ask that would
\* make the composer queue your next message instead of sending it (isBusy).
InterruptSafe ==
  (Quiet /\ interrupts > 0 /\ alive) =>
    /\ ~inTurn /\ ~streaming /\ ~dBusy /\ waiting = {} /\ vOpen = {}

TypeOK ==
  /\ alive \in BOOLEAN /\ busy \in BOOLEAN /\ turn \in Nat
  /\ cOpen \subseteq Asks /\ asked \subseteq Asks /\ pending \subseteq Asks
  /\ waiting \subseteq Msgs /\ vOpen \subseteq Asks /\ stored \subseteq Msgs \cup Asks
  /\ chip \in [Composer -> {"none", "delivered", "queued"}]
  /\ accepted \subseteq Msgs /\ failed \subseteq Msgs
=============================================================================
