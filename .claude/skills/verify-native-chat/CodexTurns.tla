---------------------------- MODULE CodexTurns ----------------------------
(* One composer session on codex app-server: SessionService::send / respond_permission /
   interrupt, CodexDriver (send_user_turn, steer, interrupt, the Translator's
   active_turn / requested_turn / steering / pending_approvals, read_and_wait), codex
   itself as JSON-RPC requests in and responses and notifications out, the relay and
   the forwarder actor, and one desktop's sessionStore. The service, relay, actor and
   desktop steps are SessionTurns' (claude); SKILL.md maps every action to the code. *)
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS Composer, Agent, Asks, MaxProcs, MaxExits, MaxInterrupts,
          MaxOtherTools  \* results of tool calls that asked nothing, on the root thread

Msgs == Composer \cup Agent

VARIABLES
  \* codex app-server: the turn it runs, requests not yet read, accepted steers not yet
  \* committed, open approvals, turn/start responses not yet written, lines out
  alive, busy, turn, pipe, queue, cOpen, asked, respond, otherTools, out,
  \* CodexDriver: Translator.active_turn / requested_turn (0 = none), steering, pending_approvals
  dActive, dReq, waiting, pending, stdinOpen, bus,
  live, procs, exits, inTurn, stored,
  \* the desktop; vSeq is the open permission cards, oldest first
  vLive, streaming, vSeq, clicked, chip, sends,
  held, resent, toldSet,
  accepted, failed, delivered, interrupts, droppedStop

vars == <<alive, busy, turn, pipe, queue, cOpen, asked, respond, otherTools, out,
          dActive, dReq, waiting, pending, stdinOpen, bus,
          live, procs, exits, inTurn, stored,
          vLive, streaming, vSeq, clicked, chip, sends,
          held, resent, toldSet,
          accepted, failed, delivered, interrupts, droppedStop>>

codexVars == <<alive, busy, turn, pipe, queue, cOpen, asked, respond, otherTools, out>>
driverVars == <<dActive, dReq, waiting, pending, stdinOpen, bus>>
viewVars == <<vLive, streaming, vSeq, clicked, chip, sends>>
heldVars == <<held, resent, toldSet>>

InSeq(x, s) == \E i \in 1..Len(s) : s[i] = x
Range(s) == {s[i] : i \in 1..Len(s)}
Remove(s, x) == SelectSeq(s, LAMBDA y : y /= x)

RECURSIVE SetToSeq(_)
SetToSeq(S) == IF S = {} THEN <<>> ELSE LET x == CHOOSE y \in S : TRUE IN <<x>> \o SetToSeq(S \ {x})

vOpen == Range(vSeq)

Init ==
  /\ alive = TRUE /\ busy = FALSE /\ turn = 0 /\ pipe = <<>> /\ queue = <<>>
  /\ cOpen = {} /\ asked = {} /\ respond = {} /\ otherTools = 0 /\ out = <<>>
  /\ dActive = 0 /\ dReq = 0 /\ waiting = {} /\ pending = {} /\ stdinOpen = TRUE /\ bus = <<>>
  /\ live = TRUE /\ procs = 1 /\ exits = 0 /\ inTurn = FALSE /\ stored = {}
  /\ vLive = TRUE /\ streaming = FALSE /\ vSeq = <<>> /\ clicked = {}
  /\ chip = [m \in Composer |-> "none"] /\ sends = [m \in Composer |-> 0]
  /\ held = {} /\ resent = {} /\ toldSet \in SUBSET Agent
  /\ accepted = {} /\ failed = {} /\ delivered = [m \in Msgs |-> 0]
  /\ interrupts = 0 /\ droppedStop = FALSE

(* ---- CodexDriver::send_user_turn, steer and write, under the op lock ---- *)

\* write: the line reaches codex while write_loop's receiver lives; a dead process loses it.
WriteOk(line) ==
  /\ stdinOpen
  /\ IF alive THEN pipe' = Append(pipe, line) /\ stdinOpen' = TRUE
              ELSE pipe' = pipe /\ stdinOpen' \in BOOLEAN

Running == IF dActive /= 0 THEN dActive ELSE dReq

\* A turn the driver knows of: steer into it, with the echo held in `steering` until codex
\* commits the message. None: echo now, then turn/start.
SendServer(m, result) ==
  IF ~live
    THEN result = "gone" /\ UNCHANGED <<driverVars, pipe>>
  ELSE IF Running /= 0
    THEN IF stdinOpen
           THEN /\ result = "ok" /\ waiting' = waiting \cup {m}
                /\ WriteOk(<<"steer", m, Running>>)
                /\ UNCHANGED <<dActive, dReq, pending, bus>>
           ELSE result = "err" /\ UNCHANGED <<driverVars, pipe>>
  ELSE /\ bus' = Append(bus, <<"echo", m>>)
       /\ IF stdinOpen
            THEN result = "ok" /\ WriteOk(<<"start", m>>)
            ELSE result = "err" /\ UNCHANGED <<pipe, stdinOpen>>
       /\ UNCHANGED <<dActive, dReq, waiting, pending>>

Outcome(m, result, kept) ==
  /\ accepted' = IF result = "ok" THEN accepted \cup {m} ELSE accepted
  /\ failed' = IF result = "err" \/ (result = "gone" /\ ~kept) THEN failed \cup {m} ELSE failed

(* ---- the desktop's sends: deliverMessage (direct) and steerMessage ---- *)

ClientSend(m) ==
  /\ m \in Composer /\ vLive /\ sends[m] < 2
  /\ \/ chip[m] = "none" /\ sends[m] = 0
     \/ chip[m] = "queued" /\ ~streaming
  /\ LET steer == streaming /\ chip[m] = "none" IN
     \E result \in {"ok", "err", "gone"} :
       /\ SendServer(m, result)
       \* a direct send is kept as unechoed until its echo; session_not_found queues either
       /\ chip' = [chip EXCEPT ![m] = IF result = "ok" /\ steer THEN "delivered"
                                     ELSE IF result = "ok" THEN "unechoed"
                                     ELSE IF result = "gone" THEN "queued"
                                     ELSE "none"]
       /\ Outcome(m, result, TRUE)
       /\ streaming' = (streaming \/ (~steer /\ result = "ok"))
  /\ sends' = [sends EXCEPT ![m] = @ + 1]
  /\ UNCHANGED <<heldVars, alive, busy, turn, queue, cOpen, asked, respond, otherTools, out,
                 live, procs, exits, inTurn, stored, vLive, vSeq, clicked, delivered,
                 interrupts, droppedStop>>

AgentSend(m) ==
  /\ m \in Agent /\ m \notin accepted \cup failed
  /\ \E result \in {"ok", "err", "gone"} :
       /\ SendServer(m, result) /\ Outcome(m, result, FALSE)
       /\ held' = IF result = "ok" THEN held \cup {m} ELSE held
  /\ UNCHANGED <<alive, busy, turn, queue, cOpen, asked, respond, otherTools, out, live,
                 procs, exits, inTurn, stored, viewVars, resent, toldSet, delivered,
                 interrupts, droppedStop>>

(* ---- respond_permission and interrupt ---- *)

\* respond_permission: an approval the Translator still holds is resolved and replied to;
\* any other id returns Ok and writes nothing.
ClientAnswer(r) ==
  /\ r \in vOpen \ clicked /\ live /\ stdinOpen
  /\ clicked' = clicked \cup {r}
  /\ IF r \in pending
       THEN /\ pending' = pending \ {r} /\ bus' = Append(bus, <<"resolved", r>>)
            /\ WriteOk(<<"answer", r>>)
       ELSE UNCHANGED <<pending, bus, pipe, stdinOpen>>
  /\ UNCHANGED <<heldVars, alive, busy, turn, queue, cOpen, asked, respond, otherTools, out,
                 dActive, dReq, waiting, live, procs, exits, inTurn, stored, vLive,
                 streaming, vSeq, chip, sends, accepted, failed, delivered, interrupts,
                 droppedStop>>

\* CodexDriver::interrupt needs a turn id: with none known it returns Ok and sends nothing,
\* though codex may already run (or be about to run) a turn the driver has not heard of.
Interrupt ==
  /\ interrupts < MaxInterrupts /\ live /\ stdinOpen
  /\ interrupts' = interrupts + 1
  /\ IF Running /= 0
       THEN WriteOk(<<"interrupt", Running>>) /\ UNCHANGED droppedStop
       ELSE /\ UNCHANGED <<pipe, stdinOpen>>
            /\ droppedStop' = (droppedStop \/ (alive /\ (busy \/ \E i \in 1..Len(pipe) : pipe[i][1] = "start")))
  /\ UNCHANGED <<heldVars, alive, busy, turn, queue, cOpen, asked, respond, otherTools, out,
                 dActive, dReq, waiting, pending, bus, live, procs, exits, inTurn, stored,
                 viewVars, accepted, failed, delivered>>

(* ---- codex app-server ---- *)

Take(m) == delivered' = [delivered EXCEPT ![m] = @ + 1]

\* codex reads one request. turn/start while a turn runs is either refused or started
\* after it (the model allows both); a steer is accepted only into the turn it names.
CodexRead ==
  /\ alive /\ pipe /= <<>>
  /\ LET line == Head(pipe) IN
     /\ pipe' = Tail(pipe)
     /\ CASE line[1] = "start" /\ ~busy ->
               /\ busy' = TRUE /\ turn' = turn + 1 /\ Take(line[2])
               /\ respond' = respond \cup {turn + 1}
               /\ out' = out \o <<<<"turnStarted", turn + 1>>, <<"userMsg", line[2]>>>>
               /\ UNCHANGED <<queue, cOpen>>
          [] line[1] = "start" ->
               /\ out' = Append(out, <<"startErr", line[2]>>)
               /\ UNCHANGED <<busy, turn, respond, queue, cOpen, delivered>>
          [] line[1] = "steer" /\ busy /\ line[3] = turn ->
               /\ queue' = Append(queue, line[2])
               /\ UNCHANGED <<busy, turn, respond, out, cOpen, delivered>>
          [] line[1] = "steer" ->
               /\ out' = Append(out, <<"steerErr", line[2]>>)
               /\ UNCHANGED <<busy, turn, respond, queue, cOpen, delivered>>
          [] line[1] = "interrupt" /\ busy /\ line[2] = turn ->
               \* codex settles its open approvals itself: turn/completed, then
               \* serverRequest/resolved for each; whether each one's item completes
               \* first (a declined result) is left open
               /\ busy' = FALSE /\ cOpen' = {} /\ queue' = <<>>
               /\ \E settled \in SUBSET cOpen :
                    out' = out \o SetToSeq({<<"toolResult", r>> : r \in settled})
                               \o <<<<"turnCompleted">>>>
                               \o SetToSeq({<<"srvResolved", r>> : r \in cOpen})
               /\ UNCHANGED <<turn, respond, delivered>>
          [] line[1] = "interrupt" ->
               UNCHANGED <<busy, turn, respond, queue, cOpen, out, delivered>>
          [] line[1] = "answer" /\ line[2] \in cOpen ->
               /\ cOpen' = cOpen \ {line[2]}
               /\ out' = Append(out, <<"toolResult", line[2]>>)
               /\ UNCHANGED <<busy, turn, respond, queue, delivered>>
          [] line[1] = "answer" ->
               UNCHANGED <<busy, turn, respond, queue, cOpen, out, delivered>>
  /\ UNCHANGED <<heldVars, alive, asked, otherTools, driverVars, live, procs, exits, inTurn,
                 stored, viewVars, accepted, failed, interrupts, droppedStop>>

\* The turn/start response, written by the request handler; it can trail the turn's notifications.
CodexRespond ==
  /\ alive /\ respond /= {}
  /\ \E t \in respond :
       /\ respond' = respond \ {t}
       /\ out' = Append(out, <<"startOk", t>>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, cOpen, asked, otherTools,
                 driverVars, live, procs, exits, inTurn, stored, viewVars, accepted, failed,
                 delivered, interrupts, droppedStop>>

CodexAsk(r) ==
  /\ alive /\ busy /\ r \notin asked
  /\ asked' = asked \cup {r} /\ cOpen' = cOpen \cup {r}
  /\ out' = Append(out, <<"ask", r>>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, respond, otherTools, driverVars,
                 live, procs, exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

\* A Bash call that needed no approval completes while others wait on theirs.
CodexOtherTool ==
  /\ alive /\ busy /\ otherTools < MaxOtherTools
  /\ otherTools' = otherTools + 1
  /\ out' = Append(out, <<"toolResult", "none">>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, cOpen, asked, respond, driverVars,
                 live, procs, exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

\* A steer is committed at codex's next step: item/completed userMessage with its clientId.
CodexCommitSteer ==
  /\ alive /\ busy /\ queue /= <<>> /\ cOpen = {}
  /\ Take(Head(queue)) /\ queue' = Tail(queue)
  /\ out' = Append(out, <<"userMsg", Head(queue)>>)
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, cOpen, asked, respond, otherTools,
                 driverVars, live, procs, exits, inTurn, stored, viewVars, accepted, failed,
                 interrupts, droppedStop>>

\* The turn ends; codex drops steers it never committed (the driver's own assumption).
CodexFinish ==
  /\ alive /\ busy /\ cOpen = {}
  /\ busy' = FALSE /\ queue' = <<>> /\ out' = Append(out, <<"turnCompleted">>)
  /\ UNCHANGED <<heldVars, alive, turn, pipe, cOpen, asked, respond, otherTools, driverVars,
                 live, procs, exits, inTurn, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop>>

CodexExit ==
  /\ alive /\ exits < MaxExits
  /\ alive' = FALSE /\ busy' = FALSE /\ cOpen' = {} /\ pipe' = <<>> /\ queue' = <<>>
  /\ respond' = {}
  /\ out' = Append(out, <<"eof">>)
  /\ exits' = exits + 1
  /\ UNCHANGED <<heldVars, turn, asked, otherTools, driverVars, live, procs, inTurn, stored,
                 viewVars, accepted, failed, delivered, interrupts, droppedStop>>

(* ---- read_and_wait -> route_incoming -> Translator ---- *)

DriverRead ==
  /\ out /= <<>>
  /\ LET line == Head(out) IN
     /\ out' = Tail(out)
     /\ CASE line[1] = "startOk" ->
               \* the spawned turn/start task sets requested_turn from the response
               dReq' = line[2] /\ UNCHANGED <<dActive, waiting, pending, bus>>
          [] line[1] = "turnStarted" ->
               dActive' = line[2] /\ UNCHANGED <<dReq, waiting, pending, bus>>
          [] line[1] = "userMsg" ->
               \* item_completed: a steer's echo waits for this commit
               /\ IF line[2] \in waiting
                    THEN waiting' = waiting \ {line[2]} /\ bus' = Append(bus, <<"echo", line[2]>>)
                    ELSE UNCHANGED <<waiting, bus>>
               /\ UNCHANGED <<dActive, dReq, pending>>
          [] line[1] = "steerErr" ->
               \* the steer task: take_steer, then declined_message
               /\ IF line[2] \in waiting
                    THEN waiting' = waiting \ {line[2]} /\ bus' = Append(bus, <<"declined", line[2]>>)
                    ELSE UNCHANGED <<waiting, bus>>
               /\ UNCHANGED <<dActive, dReq, pending>>
          [] line[1] = "startErr" ->
               \* the turn/start task: "Codex request failed", then a failed TurnDone
               bus' = Append(bus, <<"turnDone">>) /\ UNCHANGED <<dActive, dReq, waiting, pending>>
          [] line[1] = "turnCompleted" ->
               \* decline_steers, then TurnDone; both turn ids cleared
               /\ dActive' = 0 /\ dReq' = 0 /\ waiting' = {}
               /\ bus' = bus \o SetToSeq({<<"declined", w>> : w \in waiting}) \o <<<<"turnDone">>>>
               /\ UNCHANGED pending
          [] line[1] = "ask" ->
               pending' = pending \cup {line[2]} /\ bus' = Append(bus, <<"ask", line[2]>>)
               /\ UNCHANGED <<dActive, dReq, waiting>>
          [] line[1] = "srvResolved" ->
               \* settle: an approval still pending closes as declined; an answered one is gone
               /\ IF line[2] \in pending
                    THEN pending' = pending \ {line[2]} /\ bus' = Append(bus, <<"resolved", line[2]>>)
                    ELSE UNCHANGED <<pending, bus>>
               /\ UNCHANGED <<dActive, dReq, waiting>>
          [] line[1] = "toolResult" ->
               bus' = Append(bus, <<"toolResult", line[2]>>) /\ UNCHANGED <<dActive, dReq, waiting, pending>>
          [] line[1] = "eof" ->
               \* replies still pending fail; their tasks' events trail Exited, which ends the relay
               bus' = Append(bus, <<"exited">>) /\ UNCHANGED <<dActive, dReq, waiting, pending>>
  /\ UNCHANGED <<heldVars, alive, busy, turn, pipe, queue, cOpen, asked, respond, otherTools,
                 stdinOpen, live, procs, exits, inTurn, stored, viewVars, accepted, failed,
                 delivered, interrupts, droppedStop>>

(* ---- spawn_relay + ActorState::handle_event, and the desktop's applyEvent ---- *)

Relay ==
  /\ bus /= <<>>
  /\ LET ev == Head(bus)
         handBack(which) == [m \in Composer |-> IF chip[m] \in which THEN "queued" ELSE chip[m]]
     IN
     /\ inTurn' = CASE ev[1] = "echo" -> TRUE
                   [] ev[1] \in {"turnDone", "exited"} -> FALSE
                   [] OTHER -> inTurn
     /\ stored' = IF ev[1] \in {"echo", "ask"} THEN stored \cup {ev[2]} ELSE stored
     /\ live' = IF ev[1] = "exited" THEN FALSE ELSE live
     /\ bus' = IF ev[1] = "exited" THEN <<>> ELSE Tail(bus)
     \* codex's asks carry no tool_use_id, so settleAnsweredPrompt decides the newest
     \* undecided card with the result's tool name (all of the model's asks are Bash)
     /\ streaming' = CASE ev[1] \in {"echo", "ask"} -> TRUE
                      [] ev[1] = "toolResult" /\ vSeq /= <<>> -> TRUE
                      [] ev[1] \in {"turnDone", "exited"} -> FALSE
                      [] OTHER -> streaming
     /\ vSeq' = CASE ev[1] = "ask" -> Append(vSeq, ev[2])
                 [] ev[1] = "resolved" -> Remove(vSeq, ev[2])
                 [] ev[1] = "toolResult" /\ vSeq /= <<>> -> SubSeq(vSeq, 1, Len(vSeq) - 1)
                 [] ev[1] = "exited" -> <<>>
                 [] OTHER -> vSeq
     /\ vLive' = IF ev[1] = "exited" THEN FALSE ELSE vLive
     /\ chip' = CASE ev[1] = "echo" /\ ev[2] \in Composer -> [chip EXCEPT ![ev[2]] = "none"]
                 [] ev[1] = "declined" /\ ev[2] \in Composer /\ chip[ev[2]] \in {"delivered", "unechoed"} ->
                      [chip EXCEPT ![ev[2]] = "queued"]
                 [] ev[1] = "exited" -> handBack({"delivered", "unechoed"})
                 \* a live relaunch follows its exit; a direct send after it may be the resume's own
                 [] ev[1] = "relaunched" -> handBack({"delivered"})
                 [] OTHER -> chip
     /\ held' = CASE ev[1] = "echo" -> held \ {ev[2]}
                 [] ev[1] = "exited" -> held \ toldSet
                 [] OTHER -> held
     /\ failed' = IF ev[1] = "exited" THEN failed \cup (held \cap toldSet) ELSE failed
  /\ UNCHANGED <<codexVars, dActive, dReq, waiting, pending, stdinOpen, procs, exits,
                 clicked, sends, resent, toldSet, accepted, delivered, interrupts, droppedStop>>

\* A tail read: list_turns re-sends the stored asks pending_requests names; settleUnwaitedAsks
\* closes the rest.
ClientReload ==
  /\ vLive /\ live
  /\ vSeq' = SetToSeq(pending \cap stored)
  /\ UNCHANGED <<heldVars, codexVars, driverVars, live, procs, exits, inTurn, stored,
                 vLive, streaming, clicked, chip, sends, accepted, failed, delivered,
                 interrupts, droppedStop>>

\* ResumeSession: thread/resume in a fresh app-server and driver, Relaunched; held::resend
\* sends each held message once to the idle driver, which echoes it and starts a turn.
ClientResume ==
  LET again == held \ resent
      lost == held \cap resent
  IN
  /\ ~vLive /\ ~live /\ procs < MaxProcs
  /\ alive' = TRUE /\ busy' = FALSE /\ queue' = <<>> /\ cOpen' = {} /\ respond' = {}
  /\ pipe' = SetToSeq({<<"start", m>> : m \in again})
  /\ out' = <<>>
  /\ dActive' = 0 /\ dReq' = 0 /\ waiting' = {} /\ pending' = {} /\ stdinOpen' = TRUE
  /\ bus' = <<<<"relaunched">>>> \o SetToSeq({<<"echo", m>> : m \in again})
  /\ live' = TRUE /\ procs' = procs + 1 /\ vLive' = TRUE
  /\ held' = again /\ resent' = resent \cup again
  /\ failed' = failed \cup (lost \cap toldSet)
  /\ UNCHANGED <<turn, asked, otherTools, exits, inTurn, stored, streaming, vSeq, clicked,
                 chip, sends, toldSet, accepted, delivered, interrupts, droppedStop>>

(* ---- the rules ---- *)

Quiet ==
  /\ out = <<>> /\ bus = <<>>
  /\ alive => (~busy /\ pipe = <<>> /\ queue = <<>> /\ respond = {})

Done == Quiet /\ UNCHANGED vars

Next ==
  \/ \E m \in Composer : ClientSend(m)
  \/ \E m \in Agent : AgentSend(m)
  \/ \E r \in Asks : ClientAnswer(r) \/ CodexAsk(r)
  \/ Interrupt
  \/ CodexRead \/ CodexRespond \/ CodexOtherTool \/ CodexCommitSteer \/ CodexFinish \/ CodexExit
  \/ DriverRead \/ Relay \/ ClientReload \/ ClientResume \/ Done

Spec == Init /\ [][Next]_vars

OnItsWay(m) ==
  \/ alive /\ (\E i \in 1..Len(pipe) : pipe[i][1] \in {"start", "steer"} /\ pipe[i][2] = m)
  \/ alive /\ InSeq(m, queue)
  \/ InSeq(<<"echo", m>>, bus)

Kept(m) ==
  \/ m \notin accepted
  \/ delivered[m] > 0
  \/ OnItsWay(m)
  \/ m \in Composer /\ chip[m] /= "none"
  \/ m \in failed
  \/ m \in stored
  \/ m \in held /\ ~(Quiet /\ live)

NoLostUserMessage == \A m \in Composer : Kept(m)
NoLostAgentMessage == \A m \in Agent : Kept(m)

NoDoubleSend == \A m \in Msgs : delivered[m] <= 1

TurnProgress ==
  Quiet => /\ ~inTurn /\ ~streaming
           /\ alive => (Running = 0 /\ waiting = {})

AskResolution == Quiet => vOpen \subseteq cOpen

\* While codex waits on an approval and nothing is in flight, the desktop shows it.
AskVisible == (out = <<>> /\ bus = <<>> /\ pipe = <<>>) => cOpen \subseteq vOpen

InterruptSafe ==
  (Quiet /\ interrupts > 0 /\ alive) =>
    /\ ~inTurn /\ ~streaming /\ Running = 0 /\ waiting = {} /\ vOpen = {}

\* An interrupt is never dropped while codex runs, or is about to run, a turn.
InterruptHonored == ~droppedStop

TypeOK ==
  /\ alive \in BOOLEAN /\ busy \in BOOLEAN /\ turn \in Nat
  /\ cOpen \subseteq Asks /\ asked \subseteq Asks /\ pending \subseteq Asks
  /\ waiting \subseteq Msgs /\ vOpen \subseteq Asks /\ stored \subseteq Msgs \cup Asks
  /\ chip \in [Composer -> {"none", "delivered", "unechoed", "queued"}]
  /\ accepted \subseteq Msgs /\ failed \subseteq Msgs
  /\ held \subseteq Agent /\ resent \subseteq Agent /\ toldSet \subseteq Agent
=============================================================================
