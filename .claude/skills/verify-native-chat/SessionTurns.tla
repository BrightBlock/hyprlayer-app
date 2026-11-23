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
          MaxInterrupts,
          MaxRestarts  \* desktop reloads

Msgs == Composer \cup Agent

VARIABLES
  \* the claude process
  alive, busy, turn, pipe, queue, cOpen, cRun, asked, out,
  \* ClaudeDriver
  dBusy, lifecycle, waiting, pending, stdinOpen, bus, eofSeen,
  \* SessionRegistry, the relay and the actor
  live, procs, exits, inTurn, stored,
  \* the desktop
  vLive, streaming, vOpen, clicked, chip, sends,
  \* held_messages: agent messages recorded until their echo, resent once, and those whose sender is told
  held, resent, toldSet,
  \* bookkeeping
  accepted, failed, delivered, interrupts, droppedStop,
  \* the relay: messages a harness declined this turn, and held ones it is sending again;
  \* the desktop: set by its Stop until you send again
  dropped, resendDue, stopped,
  \* how many times each ask's resolution reached the log
  resolves,
  \* SessionService::send past require_live and not yet sent: the message and the process it saw
  checking, checkProc,
  restarts

vars == <<alive, busy, turn, pipe, queue, cOpen, cRun, asked, out,
          dBusy, lifecycle, waiting, pending, stdinOpen, bus, eofSeen,
          live, procs, exits, inTurn, stored,
          vLive, streaming, vOpen, clicked, chip, sends,
          held, resent, toldSet,
          accepted, failed, delivered, interrupts, droppedStop,
          dropped, resendDue, stopped, resolves, checking, checkProc, restarts>>

claudeVars == <<alive, busy, turn, pipe, queue, cOpen, cRun, asked, out>>
driverVars == <<dBusy, lifecycle, waiting, pending, stdinOpen, bus, eofSeen>>
viewVars == <<vLive, streaming, vOpen, clicked, chip, sends>>
heldVars == <<held, resent, toldSet>>

InSeq(x, s) == \E i \in 1..Len(s) : s[i] = x

RECURSIVE SetToSeq(_)
SetToSeq(S) == IF S = {} THEN <<>> ELSE LET x == CHOOSE y \in S : TRUE IN <<x>> \o SetToSeq(S \ {x})

Init ==
  /\ alive = TRUE /\ busy = FALSE /\ turn = 0 /\ pipe = <<>> /\ queue = <<>>
  /\ cOpen = {} /\ cRun = {} /\ asked = {} /\ out = <<>>
  /\ dBusy = FALSE /\ lifecycle = FALSE /\ waiting = {} /\ pending = {} /\ stdinOpen = TRUE /\ bus = <<>>
  /\ eofSeen = FALSE
  /\ live = TRUE /\ procs = 1 /\ exits = 0 /\ inTurn = FALSE /\ stored = {}
  /\ vLive = TRUE /\ streaming = FALSE /\ vOpen = {} /\ clicked = {}
  /\ chip = [m \in Composer |-> "none"] /\ sends = [m \in Composer |-> 0]
  /\ held = {} /\ resent = {} /\ toldSet \in SUBSET Agent
  /\ accepted = {} /\ failed = {} /\ delivered = [m \in Msgs |-> 0]
  /\ interrupts = 0 /\ droppedStop = FALSE
  /\ dropped = {} /\ resendDue = {} /\ stopped = FALSE /\ resolves = [r \in Asks |-> 0]
  /\ checking = {} /\ checkProc = 0 /\ restarts = 0

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

\* Returns "ok", "err" (stdin closed) or "gone" (no live driver, or its stdout already ended).
SendServer(m, result) ==
  IF ~live \/ eofSeen
    THEN result = "gone" /\ UNCHANGED <<driverVars, pipe>>
  ELSE IF stdinOpen
    THEN result = "ok" /\ EchoOrHold(m) /\ WriteOk(<<"msg", m>>)
         /\ pending' = pending /\ lifecycle' = lifecycle /\ eofSeen' = eofSeen
  ELSE \* the echo decision runs before the write; Err takes a held echo back
    /\ result = "err"
    /\ IF (~dBusy /\ waiting = {}) \/ ~lifecycle
         THEN dBusy' = TRUE /\ bus' = Append(bus, <<"echo", m>>)
         ELSE dBusy' = dBusy /\ bus' = bus
    /\ waiting' = waiting /\ pending' = pending /\ stdinOpen' = stdinOpen /\ pipe' = pipe
    /\ lifecycle' = lifecycle /\ eofSeen' = eofSeen

Outcome(m, result, kept) ==
  /\ accepted' = IF result = "ok" THEN accepted \cup {m} ELSE accepted
  /\ failed' = IF result = "err" \/ (result = "gone" /\ ~kept) THEN failed \cup {m} ELSE failed

(* ---- the desktop's sends: deliverMessage (direct) and steerMessage ---- *)

\* A first send, or a chip handed back on an exit or relaunch, flushed once the view is idle.
\* Both keep a chip until the echo (a direct send's is unechoed, with its optimistic row);
\* session_not_found puts the message back in the queue.
ClientSend(m) ==
  /\ m \in Composer /\ vLive /\ sends[m] < 2
  /\ \/ chip[m] = "none" /\ sends[m] = 0
     \/ chip[m] = "queued" /\ ~streaming
  /\ LET steer == streaming /\ chip[m] = "none" IN
     \E result \in {"ok", "err", "gone"} :
       /\ SendServer(m, result)
       /\ chip' = [chip EXCEPT ![m] = IF result = "ok" /\ steer THEN "delivered"
                                     ELSE IF result = "ok" THEN "unechoed"
                                     ELSE IF result = "gone" THEN "queued"
                                     ELSE "none"]
       /\ Outcome(m, result, TRUE)
       \* direct: an optimistic row, and the view shows the turn running
       /\ streaming' = (streaming \/ (~steer /\ result = "ok"))
  /\ sends' = [sends EXCEPT ![m] = @ + 1]
  /\ stopped' = FALSE /\ UNCHANGED <<dropped, resendDue, resolves, checking, checkProc, restarts>>
  /\ UNCHANGED <<heldVars, alive, busy, turn, queue, cOpen, cRun, asked, out, live, procs, exits,
                 inTurn, stored, vLive, vOpen, clicked, delivered,
                 interrupts, droppedStop>>

\* SessionService::send under the op lock: require_live, then the held_messages row and
\* send_user_turn on the driver it found, which it removes again on Err. launch_driver takes no
\* op lock, so a resume can come between; the driver it found then refuses (Inbox.ended).
AgentCheck(m) ==
  /\ m \in Agent /\ m \notin accepted \cup failed \cup checking
  /\ IF live
       THEN checking' = checking \cup {m} /\ checkProc' = procs /\ UNCHANGED <<failed>>
       ELSE failed' = failed \cup {m} /\ UNCHANGED <<checking, checkProc>>
  /\ UNCHANGED <<claudeVars, driverVars, live, procs, exits, inTurn, stored, viewVars,
                 held, resent, toldSet, accepted, delivered, interrupts, droppedStop,
                 dropped, resendDue, stopped, resolves, restarts>>

AgentHold(m) ==
  /\ m \in checking
  /\ checking' = checking \ {m} /\ checkProc' = 0
  /\ IF procs = checkProc
       THEN \E result \in {"ok", "err", "gone"} :
              /\ SendServer(m, result) /\ Outcome(m, result, FALSE)
              /\ held' = IF result = "ok" THEN held \cup {m} ELSE held
       ELSE /\ Outcome(m, "gone", FALSE) /\ UNCHANGED <<driverVars, pipe, held>>
  /\ UNCHANGED <<alive, busy, turn, queue, cOpen, cRun, asked, out, live, procs, exits,
                 inTurn, stored, viewVars, resent, toldSet, delivered, interrupts, droppedStop,
                 dropped, resendDue, stopped, resolves, restarts>>

(* ---- respond_permission and interrupt ---- *)

\* You answer an ask once.
ClientAnswer(r) ==
  /\ r \in vOpen \ clicked /\ live /\ stdinOpen
  /\ clicked' = clicked \cup {r}
  /\ WriteOk(<<"answer", r>>)
  /\ IF r \in pending
       THEN pending' = pending \ {r} /\ bus' = Append(bus, <<"resolved", r>>)
       ELSE pending' = pending /\ bus' = bus
  /\ UNCHANGED <<heldVars, alive, busy, turn, queue, cOpen, cRun, asked, out, dBusy, lifecycle, waiting,
                 eofSeen, live, procs, exits, inTurn, stored, vLive, streaming, vOpen, chip,
                 sends, accepted, failed, delivered, interrupts, droppedStop>>

\* SessionService::interrupt from anyone: the composer's stop (which declines a parked ask
\* first), an agent's stop_agent, a goal.
Interrupt ==
  /\ interrupts < MaxInterrupts /\ live /\ stdinOpen
  \* the line carries the messages written before it, which the stop is for
  /\ WriteOk(<<"interrupt", {pipe[i][2] : i \in {j \in 1..Len(pipe) : pipe[j][1] = "msg"}}>>)
  /\ interrupts' = interrupts + 1
  \* the composer's Stop, or another's
  /\ stopped' \in {stopped, TRUE} /\ UNCHANGED <<dropped, resendDue, resolves, checking, checkProc, restarts>>
  /\ UNCHANGED <<heldVars, alive, busy, turn, queue, cOpen, cRun, asked, out, dBusy, lifecycle, waiting, eofSeen,
                 pending, bus, live, procs, exits, inTurn, stored, viewVars,
                 accepted, failed, delivered, droppedStop>>

(* ---- claude ---- *)

Take(m) == delivered' = [delivered EXCEPT ![m] = @ + 1]

\* Claude settles an ask without us: control_cancel_request, then the tool's result (an error
\* when it was interrupted or denied), as real claude 2.1.330 writes them.
RECURSIVE Withdraw(_)
Withdraw(S) ==
  IF S = {} THEN <<>>
  ELSE LET r == CHOOSE x \in S : TRUE
       IN <<<<"cancel", r>>, <<"toolResult", r>>>> \o Withdraw(S \ {r})

\* An allowed tool's result, written once the tool has run.
RECURSIVE Results(_)
Results(S) ==
  IF S = {} THEN <<>>
  ELSE LET r == CHOOSE x \in S : TRUE IN <<<<"toolResult", r>>>> \o Results(S \ {r})

ClaudeRead ==
  /\ alive /\ pipe /= <<>>
  /\ LET line == Head(pipe) IN
     /\ pipe' = Tail(pipe)
     /\ droppedStop' = (droppedStop \/ (line[1] = "interrupt" /\ ~busy
                          /\ \E j \in 2..Len(pipe) : pipe[j][1] = "msg" /\ pipe[j][2] \in line[2]))
     /\ CASE line[1] = "msg" /\ ~busy ->
               /\ busy' = TRUE /\ turn' = turn + 1 /\ Take(line[2])
               /\ out' = out \o <<<<"started", line[2]>>, <<"init">>>>
               /\ UNCHANGED <<queue, cOpen, cRun>>
          [] line[1] = "msg" ->
               /\ queue' = Append(queue, line[2])
               /\ UNCHANGED <<busy, turn, out, cOpen, cRun, delivered>>
          [] line[1] = "interrupt" /\ busy ->
               \* claude cancels its open asks (control_cancel_request), stops running tools, ends the turn
               /\ busy' = FALSE /\ cOpen' = {} /\ cRun' = {}
               /\ out' = out \o Withdraw(cOpen) \o Results(cRun) \o <<<<"result">>>>
               /\ UNCHANGED <<turn, queue, delivered>>
          [] line[1] = "interrupt" ->
               UNCHANGED <<busy, turn, queue, cOpen, cRun, out, delivered>>
          [] line[1] = "answer" /\ line[2] \in cOpen ->
               \* the tool runs, or is denied: either way its result follows
               /\ cOpen' = cOpen \ {line[2]}
               /\ out' = Append(out, <<"toolResult", line[2]>>)
               /\ UNCHANGED <<busy, turn, queue, cRun, delivered>>
          [] line[1] = "answer" ->
               UNCHANGED <<busy, turn, queue, cOpen, cRun, out, delivered>>
  /\ UNCHANGED <<heldVars, alive, asked, driverVars, live, procs, exits, inTurn, stored,
                 viewVars, accepted, failed, interrupts>>

ClaudeAsk(r) ==
  /\ alive /\ busy /\ r \notin asked
  /\ asked' = asked \cup {r} /\ cOpen' = cOpen \cup {r}
  /\ out' = Append(out, <<"ask", r>>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, cRun, driverVars, live, procs, exits,
                 inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

\* A message sent mid-turn joins the turn at claude's next step.
ClaudeTakeQueued ==
  /\ alive /\ busy /\ queue /= <<>> /\ cOpen = {} /\ cRun = {}
  /\ Take(Head(queue)) /\ queue' = Tail(queue)
  /\ out' = Append(out, <<"started", Head(queue)>>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, cOpen, cRun, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed,
                 interrupts, droppedStop>>

\* Remote Control (or a PermissionRequest hook): the ask is answered elsewhere and claude withdraws
\* its can_use_tool. A deny writes the error result with the cancel; an allowed tool runs first.
ClaudeRemoteAnswer(r) ==
  /\ alive /\ r \in cOpen
  /\ cOpen' = cOpen \ {r}
  /\ \/ out' = out \o Withdraw({r}) /\ cRun' = cRun
     \/ out' = Append(out, <<"cancel", r>>) /\ cRun' = cRun \cup {r}
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

ClaudeToolDone(r) ==
  /\ alive /\ r \in cRun
  /\ cRun' = cRun \ {r}
  /\ out' = Append(out, <<"toolResult", r>>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, cOpen, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

ClaudeFinish ==
  /\ alive /\ busy /\ cOpen = {} /\ cRun = {}
  /\ busy' = FALSE /\ out' = Append(out, <<"result">>)
  /\ UNCHANGED <<heldVars, alive, turn, pipe, queue, cOpen, cRun, asked, driverVars, live, procs,
                 exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

\* claude starts a waiting message right after a result.
ClaudeStartQueued ==
  /\ alive /\ ~busy /\ queue /= <<>>
  /\ busy' = TRUE /\ turn' = turn + 1
  /\ Take(Head(queue)) /\ queue' = Tail(queue)
  /\ out' = out \o <<<<"started", Head(queue)>>, <<"init">>>>
  /\ UNCHANGED <<heldVars, alive, pipe, cOpen, cRun, asked, driverVars, live, procs, exits, inTurn,
                 stored, viewVars, accepted, failed, interrupts, droppedStop>>

\* A crash, a kill, or the process ending: unread lines and claude's queue die with it.
ClaudeExit ==
  /\ alive /\ exits < MaxExits
  /\ alive' = FALSE /\ busy' = FALSE /\ cOpen' = {} /\ cRun' = {} /\ pipe' = <<>> /\ queue' = <<>>
  /\ out' = Append(out, <<"eof">>)
  /\ exits' = exits + 1
  /\ UNCHANGED <<heldVars, turn, asked, driverVars, live, procs, inTurn, stored, viewVars,
                 accepted, failed, delivered, interrupts, droppedStop>>

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
               \* ParsedLine::ControlCancel: an ask still in pending_permissions leaves it, resolved
               /\ pending' = pending \ {line[2]}
               /\ bus' = IF line[2] \in pending THEN Append(bus, <<"resolved", line[2]>>) ELSE bus
               /\ UNCHANGED <<dBusy, waiting>>
          [] line[1] = "toolResult" ->
               bus' = Append(bus, <<"toolResult", line[2]>>) /\ UNCHANGED <<dBusy, waiting, pending>>
          [] line[1] = "eof" ->
               \* read_and_wait marks the inbox ended (send_user_turn refuses from here),
               \* declines every echo still waiting, then broadcasts Exited
               /\ bus' = bus \o SetToSeq({<<"declined", w>> : w \in waiting}) \o <<<<"exited">>>>
               /\ waiting' = {}
               /\ UNCHANGED <<dBusy, pending>>
  /\ Head(out)[1] /= "started" => lifecycle' = lifecycle
  /\ eofSeen' = (eofSeen \/ Head(out)[1] = "eof")
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, cOpen, cRun, asked, stdinOpen, live,
                 procs, exits, inTurn, stored, viewVars, accepted, failed,
                 delivered, interrupts, droppedStop>>

(* ---- spawn_relay + ActorState::handle_event, and the desktop's applyEvent ----
   One step: the actor's fan-out reaches the desktop in order, and no rule needs the
   desktop to lag the actor (it lags the harness through `out` and `bus`). *)

Relay ==
  /\ bus /= <<>> /\ resendDue = {}
  /\ LET ev == Head(bus)
         back == IF stopped THEN "returned" ELSE "queued"
         handBack(which) == [m \in Composer |-> IF chip[m] \in which THEN back ELSE chip[m]]
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
                 [] ev[1] = "declined" /\ ev[2] \in Composer /\ chip[ev[2]] \in {"delivered", "unechoed"} ->
                      [chip EXCEPT ![ev[2]] = back]
                 [] ev[1] = "exited" -> handBack({"delivered", "unechoed"})
                 \* a live relaunch follows its exit; a direct send after it may be the resume's own
                 [] ev[1] = "relaunched" -> handBack({"delivered"})
                 [] OTHER -> chip
     \* the relay releases a held message on its echo; after an exit it tells the senders it can
     \* (the model's exits are all crashes, never a detach) and keeps the rest for the next relaunch
     /\ held' = CASE ev[1] = "echo" -> held \ {ev[2]}
                 [] ev[1] = "exited" -> held \ toldSet
                 [] OTHER -> held
     /\ failed' = IF ev[1] = "exited" THEN failed \cup (held \cap toldSet) ELSE failed
     \* send_dropped: after the turn's TurnDone reaches the actor, before the next event
     /\ dropped' = CASE ev[1] = "declined" -> dropped \cup {ev[2]}
                    [] ev[1] \in {"turnDone", "exited"} -> {}
                    [] OTHER -> dropped
     /\ resendDue' = IF ev[1] = "turnDone" THEN dropped \cap held ELSE {}
     /\ resolves' = IF ev[1] = "resolved" THEN [resolves EXCEPT ![ev[2]] = @ + 1] ELSE resolves
  /\ UNCHANGED <<claudeVars, dBusy, lifecycle, waiting, pending, stdinOpen, eofSeen, procs, exits,
                 clicked, sends, resent, toldSet, accepted, delivered, interrupts, droppedStop,
                 stopped, checking, checkProc, restarts>>

\* held::send_again: a held message the harness dropped goes out again, to the live driver; one
\* it refuses is reported to a sender who can be told, and otherwise kept for the next relaunch.
RelayResend ==
  \E m \in resendDue :
    /\ resendDue' = resendDue \ {m}
    /\ \E result \in {"ok", "err", "gone"} :
         /\ SendServer(m, result)
         /\ held' = IF result /= "ok" /\ m \in toldSet THEN held \ {m} ELSE held
         /\ failed' = IF result /= "ok" /\ m \in toldSet THEN failed \cup {m} ELSE failed
    /\ UNCHANGED <<alive, busy, turn, queue, cOpen, cRun, asked, out, live, procs, exits, inTurn,
                   stored, viewVars, resent, toldSet, accepted, delivered, interrupts,
                   droppedStop, dropped, stopped, resolves, checking, checkProc, restarts>>

\* A tail read refolds the log: list_turns re-sends the stored asks open_request_ids
\* (pending_requests) names, even when their results lie outside the page, and
\* settleUnwaitedAsks closes every other one.
ClientReload ==
  /\ vLive /\ live
  /\ vOpen' = pending \cap stored
  /\ UNCHANGED <<heldVars, claudeVars, driverVars, live, procs, exits, inTurn, stored,
                 vLive, streaming, clicked, chip, sends, accepted, failed, delivered,
                 interrupts, droppedStop>>

\* The desktop window reloads: its view refolds from the log (open asks as in ClientReload; an
\* ended session expires its asks) and reattaches if the session is live. Queued, steered and
\* unechoed sends were kept in localStorage, come back with the fold, and settleFolded hands back
\* those an exit it missed dropped (the Relay's exited step does the same when the exit reaches
\* the reloaded view).
ClientRestart ==
  /\ restarts < MaxRestarts
  /\ restarts' = restarts + 1
  /\ vLive' = live /\ streaming' = (live /\ inTurn)
  /\ vOpen' = IF live THEN pending \cap stored ELSE {}
  /\ UNCHANGED <<heldVars, claudeVars, driverVars, live, procs, exits, inTurn, stored,
                 clicked, chip, sends, accepted, failed, delivered, interrupts, droppedStop,
                 dropped, resendDue, stopped, resolves, checking, checkProc>>

\* ResumeSession on a session with no live harness: launch_driver, a fresh driver, Relaunched.
\* held::resend then sends each held message once to the fresh driver, which is idle and has seen no
\* lifecycle frame, so it echoes at once; one already resent is reported lost instead.
ClientResume ==
  LET again == held \ resent
      lost == held \cap resent
  IN
  /\ ~vLive /\ ~live /\ procs < MaxProcs
  /\ alive' = TRUE /\ busy' = FALSE /\ queue' = <<>> /\ cOpen' = {} /\ cRun' = {}
  /\ pipe' = SetToSeq({<<"msg", m>> : m \in again})
  /\ out' = <<>>
  /\ dBusy' = (again /= {}) /\ lifecycle' = FALSE /\ waiting' = {} /\ pending' = {} /\ stdinOpen' = TRUE
  /\ eofSeen' = FALSE
  /\ bus' = <<<<"relaunched">>>> \o SetToSeq({<<"echo", m>> : m \in again})
  /\ live' = TRUE /\ procs' = procs + 1 /\ vLive' = TRUE
  /\ held' = again /\ resent' = resent \cup again
  /\ failed' = failed \cup (lost \cap toldSet)
  /\ UNCHANGED <<turn, asked, exits, inTurn, stored, streaming, vOpen, clicked, chip,
                 sends, toldSet, accepted, delivered, interrupts, droppedStop>>

(* ---- the rules ---- *)

\* Nothing left that could still move: no line or event in flight, claude idle or dead.
Quiet ==
  /\ out = <<>> /\ bus = <<>> /\ resendDue = {} /\ checking = {}
  /\ alive => (~busy /\ pipe = <<>> /\ queue = <<>>)

Done == Quiet /\ UNCHANGED vars

\* The steps that leave the relay's resends and the desktop's Stop as they are.
Frame(A) == A /\ UNCHANGED <<dropped, resendDue, stopped, resolves, checking, checkProc, restarts>>

Next ==
  \/ \E m \in Composer : ClientSend(m)
  \/ \E m \in Agent : AgentCheck(m) \/ AgentHold(m)
  \/ \E r \in Asks : Frame(ClientAnswer(r) \/ ClaudeAsk(r) \/ ClaudeRemoteAnswer(r) \/ ClaudeToolDone(r))
  \/ Interrupt
  \/ Frame(ClaudeRead \/ ClaudeTakeQueued \/ ClaudeFinish \/ ClaudeStartQueued \/ ClaudeExit)
  \/ Frame(DriverRead \/ ClientReload \/ ClientResume)
  \/ Relay \/ RelayResend \/ ClientRestart \/ Done

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
  \* recorded in held_messages for the next relaunch: a live, quiet session has already taken it in
  \/ m \in held /\ ~(Quiet /\ live)

NoLostUserMessage == \A m \in Composer : Kept(m)
NoLostAgentMessage == \A m \in Agent : Kept(m)

NoDoubleSend == \A m \in Msgs : delivered[m] <= 1

\* Once an exited session is quiet, no message whose sender can be told is still held untold.
SenderTold == (Quiet /\ ~live) => held \cap toldSet = {}

TurnProgress ==
  Quiet => /\ ~inTurn /\ ~streaming
           /\ alive => (~dBusy /\ waiting = {})

\* Every ask the desktop shows open is one claude still waits on.
AskResolution == Quiet => vOpen \subseteq cOpen

\* While claude waits on an ask and nothing is in flight, the desktop shows it.
AskVisible == (out = <<>> /\ bus = <<>> /\ pipe = <<>>) => cOpen \subseteq vOpen

\* Once the driver and the relay have caught up, no ask claude stopped waiting on shows open,
\* even while the tool it allowed still runs.
AskClosesOnAnswer == (out = <<>> /\ bus = <<>>) => vOpen \subseteq cOpen

\* An ask's resolution reaches the log once, whoever answered it.
ResolvedOnce == \A r \in Asks : resolves[r] <= 1

\* An interrupt is never handled while idle ahead of a message written before it.
InterruptHonored == ~droppedStop

\* After an interrupt, a live session settles idle: no turn, no held echo, and no ask that would
\* make the composer queue your next message instead of sending it (isBusy).
InterruptSafe ==
  (Quiet /\ interrupts > 0 /\ alive) =>
    /\ ~inTurn /\ ~streaming /\ ~dBusy /\ waiting = {} /\ vOpen = {}

TypeOK ==
  /\ alive \in BOOLEAN /\ busy \in BOOLEAN /\ turn \in Nat
  /\ cOpen \subseteq Asks /\ cRun \subseteq Asks /\ asked \subseteq Asks /\ pending \subseteq Asks
  /\ waiting \subseteq Msgs /\ vOpen \subseteq Asks /\ stored \subseteq Msgs \cup Asks
  /\ chip \in [Composer -> {"none", "delivered", "unechoed", "queued", "returned"}]
  /\ accepted \subseteq Msgs /\ failed \subseteq Msgs
  /\ held \subseteq Agent /\ resent \subseteq Agent /\ toldSet \subseteq Agent /\ checking \subseteq Agent
=============================================================================
