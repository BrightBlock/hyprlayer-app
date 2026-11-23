---------------------------- MODULE OpenCodeTurns ----------------------------
(* One composer session on the shared OpenCode HTTP server: SessionService::send /
   respond_permission / interrupt, OpenCodeDriver (send_user_turn's echo and
   prompt_async, the asks map, interrupt's drain and abort) and its filter task over the
   server's SSE stream (track_ask and the Translator's turn ending: sent turn, anchor,
   answered, interrupted, unplaced idle), OpenCode itself, the relay and the forwarder
   actor, and one desktop's sessionStore. The service, relay, actor and desktop steps are
   SessionTurns' (claude); SKILL.md maps every action to the code. *)
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS Composer, Agent, Asks,
          MaxInterrupts,
          MaxDrops,      \* SSE events the driver never sees (a stream reconnect, a lagged receiver)
          MaxDowns,      \* the OpenCode server dying
          InOrder        \* OpenCode handles the driver's POSTs in the order it sent them

Msgs == Composer \cup Agent

VARIABLES
  \* OpenCode: up, the prompt it runs, prompts it holds behind it, POSTs not yet handled,
  \* open asks, SSE events out
  alive, busy, current, pipe, queue, cOpen, asked, out, drops, downs,
  \* OpenCodeDriver: the asks map, the sent turn and the Translator's view of it (its open
  \* reply, and a turn end held until that reply settles)
  sent, anchored, answered, interrupted, ended, unplaced, replying, held, pending, bus,
  \* the forwarder actor: in_turn, and a message that arrived mid-turn (next_turn)
  live, inTurn, parked, stored,
  vLive, streaming, vOpen, clicked, sends,
  accepted, failed, delivered, interrupts, droppedStop, stopFor

vars == <<alive, busy, current, pipe, queue, cOpen, asked, out, drops, downs,
          sent, anchored, answered, interrupted, ended, unplaced, replying, held, pending, bus,
          live, inTurn, parked, stored,
          vLive, streaming, vOpen, clicked, sends,
          accepted, failed, delivered, interrupts, droppedStop, stopFor>>

ocVars == <<alive, busy, current, pipe, queue, cOpen, asked, out, drops, downs>>
turnVars == <<sent, anchored, answered, interrupted, ended, unplaced, replying, held>>
viewVars == <<vLive, streaming, vOpen, clicked, sends>>

None == "none"

InSeq(x, s) == \E i \in 1..Len(s) : s[i] = x
RemoveAt(s, i) == SubSeq(s, 1, i - 1) \o SubSeq(s, i + 1, Len(s))

RECURSIVE SetToSeq(_)
SetToSeq(S) == IF S = {} THEN <<>> ELSE LET x == CHOOSE y \in S : TRUE IN <<x>> \o SetToSeq(S \ {x})

Init ==
  /\ alive = TRUE /\ busy = FALSE /\ current = None /\ pipe = <<>> /\ queue = <<>>
  /\ cOpen = {} /\ asked = {} /\ out = <<>> /\ drops = 0 /\ downs = 0
  /\ sent = None /\ anchored = FALSE /\ answered = FALSE /\ interrupted = FALSE
  /\ ended = FALSE /\ unplaced = FALSE /\ replying = FALSE /\ held = FALSE
  /\ pending = {} /\ bus = <<>>
  /\ live = TRUE /\ inTurn = FALSE /\ parked = FALSE /\ stored = {}
  /\ vLive = TRUE /\ streaming = FALSE /\ vOpen = {} /\ clicked = {}
  /\ sends = [m \in Composer |-> 0]
  /\ accepted = {} /\ failed = {} /\ delivered = [m \in Msgs |-> 0]
  /\ interrupts = 0 /\ droppedStop = FALSE /\ stopFor = {}

(* ---- send_user_turn: echo, begin_turn, prompt_async ---- *)

\* Always Ok: the echo goes out first, the sent turn is replaced, and the POST runs on its own.
SendServer(m) ==
  /\ live
  /\ bus' = Append(bus, <<"echo", m>>)
  /\ sent' = m /\ anchored' = FALSE /\ answered' = FALSE /\ interrupted' = FALSE
  /\ ended' = FALSE /\ unplaced' = FALSE /\ UNCHANGED <<replying, held>>
  /\ pipe' = Append(pipe, <<"prompt", m>>)
  /\ accepted' = accepted \cup {m}

\* The composer steers only claude and codex; mid-turn its message waits in the local queue,
\* so a send goes out when the view shows idle (which can lag OpenCode).
ClientSend(m) ==
  /\ m \in Composer /\ vLive /\ ~streaming /\ sends[m] = 0
  /\ SendServer(m)
  /\ sends' = [sends EXCEPT ![m] = 1]
  /\ streaming' = TRUE
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, pending,
                 live, inTurn, parked, stored, vLive, vOpen, clicked, failed, delivered,
                 interrupts, droppedStop, stopFor>>

AgentSend(m) ==
  /\ m \in Agent /\ m \notin accepted
  /\ SendServer(m)
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, pending,
                 live, inTurn, parked, stored, viewVars, failed, delivered, interrupts, droppedStop, stopFor>>

(* ---- respond_permission and interrupt ---- *)

\* take_ask: an ask still in the map is closed at once; the reply is POSTed either way.
ClientAnswer(r) ==
  /\ r \in vOpen \ clicked /\ live
  /\ clicked' = clicked \cup {r}
  /\ IF r \in pending
       THEN pending' = pending \ {r} /\ bus' = Append(bus, <<"resolved", r>>)
       ELSE UNCHANGED <<pending, bus>>
  /\ pipe' = Append(pipe, <<"reply", r>>)
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, turnVars,
                 live, inTurn, parked, stored, vLive, streaming, vOpen, sends, accepted,
                 failed, delivered, interrupts, droppedStop, stopFor>>

\* interrupt: every ask the map holds is closed as declined, the sent turn marked, abort POSTed.
Interrupt ==
  /\ interrupts < MaxInterrupts /\ live
  /\ interrupts' = interrupts + 1
  /\ bus' = bus \o SetToSeq({<<"resolved", r>> : r \in pending})
  /\ pending' = {}
  /\ vOpen' = vOpen
  /\ interrupted' = (sent /= None)
  /\ pipe' = Append(pipe, <<"abort">>)
  \* the prompts POSTed before it, which the stop is for
  /\ stopFor' = {pipe[i][2] : i \in {j \in 1..Len(pipe) : pipe[j][1] = "prompt"}}
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, sent,
                 anchored, answered, ended, unplaced, replying, held, live, inTurn, parked, stored, vLive,
                 streaming, clicked, sends, accepted, failed, delivered, droppedStop>>

(* ---- OpenCode ---- *)

Take(m) == delivered' = [delivered EXCEPT ![m] = @ + 1]

\* OpenCode handles one POST, in any order: each is its own request from its own task. A
\* prompt's user message is created at once; it runs now, or after the prompt it is busy
\* with. An abort ends the running prompt; whether the ones held behind it run is left
\* open. A dead server fails every POST.
OcRead ==
  \E i \in 1..Len(pipe) :
  LET line == pipe[i] IN
  /\ InOrder => i = 1
  /\ pipe' = RemoveAt(pipe, i)
  /\ droppedStop' = (droppedStop \/ (alive /\ line[1] = "abort" /\ ~busy
                                     /\ \E j \in 1..Len(pipe) : pipe[j][1] = "prompt" /\ pipe[j][2] \in stopFor))
  /\ IF ~alive
          THEN \* prompt_async's request fails: "OpenCode request failed", then a failed TurnDone,
               \* which no one hears once the relay has dropped the driver
               /\ bus' = IF line[1] = "prompt" /\ live THEN bus \o <<<<"text">>, <<"turnDone">>>> ELSE bus
               /\ UNCHANGED <<busy, current, queue, cOpen, out, delivered>>
          ELSE /\ UNCHANGED bus
               /\ CASE line[1] = "prompt" /\ ~busy ->
                         /\ busy' = TRUE /\ current' = line[2] /\ Take(line[2])
                         /\ out' = out \o <<<<"userMsg", line[2]>>, <<"text", line[2]>>>>
                         /\ UNCHANGED <<queue, cOpen>>
                    [] line[1] = "prompt" ->
                         /\ queue' = Append(queue, line[2])
                         /\ out' = Append(out, <<"userMsg", line[2]>>)
                         /\ UNCHANGED <<busy, current, cOpen, delivered>>
                    [] line[1] = "abort" /\ busy ->
                         \* the error and an idle first; then, if OpenCode had created the running
                         \* reply, its cut-short part settles and the reply ends with MessageAbortedError
                         \E keep, replied \in BOOLEAN :
                           LET aborted == IF replied
                                            THEN <<<<"abortErr">>, <<"idle">>, <<"text", current>>,
                                                   <<"finished", current>>>>
                                            ELSE <<<<"abortErr">>>>
                           IN
                           /\ cOpen' = {}
                           /\ IF keep /\ queue /= <<>>
                                THEN /\ current' = Head(queue) /\ queue' = Tail(queue)
                                     /\ Take(Head(queue))
                                     /\ out' = out \o aborted \o <<<<"text", Head(queue)>>>>
                                     /\ UNCHANGED busy
                                ELSE /\ busy' = FALSE /\ current' = None /\ queue' = <<>>
                                     /\ out' = out \o aborted \o <<<<"idle">>>>
                                     /\ UNCHANGED delivered
                    [] line[1] = "abort" ->
                         UNCHANGED <<busy, current, queue, cOpen, out, delivered>>
                    [] line[1] = "reply" /\ line[2] \in cOpen ->
                         /\ cOpen' = cOpen \ {line[2]}
                         /\ out' = Append(out, <<"replied", line[2]>>)
                         /\ UNCHANGED <<busy, current, queue, delivered>>
                    [] line[1] = "reply" ->
                         UNCHANGED <<busy, current, queue, cOpen, out, delivered>>
  /\ UNCHANGED <<alive, asked, drops, downs, turnVars, pending, live, inTurn, parked, stored,
                 viewVars, accepted, failed, interrupts, stopFor>>

OcAsk(r) ==
  /\ alive /\ busy /\ r \notin asked
  /\ asked' = asked \cup {r} /\ cOpen' = cOpen \cup {r}
  /\ out' = Append(out, <<"ask", r>>)
  /\ UNCHANGED <<alive, busy, current, pipe, queue, drops, downs, turnVars, pending, bus,
                 live, inTurn, parked, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop, stopFor>>

\* Another OpenCode client (its TUI or web UI) answers the ask: permission.replied.
OcElsewhereReply(r) ==
  /\ alive /\ r \in cOpen
  /\ cOpen' = cOpen \ {r}
  /\ out' = Append(out, <<"replied", r>>)
  /\ UNCHANGED <<alive, busy, current, pipe, queue, asked, drops, downs, turnVars, pending,
                 bus, live, inTurn, parked, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop, stopFor>>

\* The reply finishes; the next held prompt starts at once, or the session goes idle.
OcFinish ==
  /\ alive /\ busy /\ cOpen = {}
  /\ IF queue /= <<>>
       THEN /\ current' = Head(queue) /\ queue' = Tail(queue) /\ Take(Head(queue))
            /\ out' = out \o <<<<"finished", current>>, <<"text", Head(queue)>>>>
            /\ UNCHANGED busy
       ELSE /\ busy' = FALSE /\ current' = None /\ queue' = <<>>
            /\ out' = out \o <<<<"finished", current>>, <<"idle">>>>
            /\ UNCHANGED delivered
  /\ UNCHANGED <<alive, pipe, cOpen, asked, drops, downs, turnVars, pending, bus, live,
                 inTurn, parked, stored, viewVars, accepted, failed, interrupts, droppedStop, stopFor>>

\* The server dies: its work and its event stream with it.
OcDown ==
  /\ alive /\ downs < MaxDowns
  /\ alive' = FALSE /\ downs' = downs + 1
  /\ busy' = FALSE /\ current' = None /\ queue' = <<>> /\ cOpen' = {} /\ out' = <<>>
  /\ UNCHANGED <<pipe, asked, drops, turnVars, pending, bus, live, inTurn, parked, stored,
                 viewVars, accepted, failed, delivered, interrupts, droppedStop, stopFor>>

\* watch_exit sees the child gone and every driver on the server sends Exited (crashed).
ServerExited ==
  /\ ~alive /\ live /\ ~InSeq(<<"exited">>, bus)
  /\ bus' = Append(bus, <<"exited">>)
  /\ UNCHANGED <<ocVars, turnVars, pending, live, inTurn, parked, stored, viewVars, accepted,
                 failed, delivered, interrupts, droppedStop, stopFor>>

\* An SSE event the driver never sees: a reconnect gap, or its broadcast receiver lagging.
SseDrop ==
  /\ out /= <<>> /\ drops < MaxDrops
  /\ out' = Tail(out) /\ drops' = drops + 1
  /\ UNCHANGED <<alive, busy, current, pipe, queue, cOpen, asked, downs, turnVars, pending,
                 bus, live, inTurn, parked, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop, stopFor>>

(* ---- the filter task: track_ask, then Translator::handle, expire ---- *)

Open == sent /= None /\ ~ended

\* end_turn: a turn that ends while its reply is open holds its TurnDone until that reply settles.
EndTurn(closes) ==
  /\ ended' = TRUE
  /\ IF replying THEN held' = TRUE /\ bus' = bus \o closes
                  ELSE bus' = bus \o closes \o <<<<"turnDone">>>> /\ UNCHANGED held

\* A turn sent after the held end lets it out before anything else is read.
DriverRead ==
  /\ out /= <<>> /\ ~(held /\ Open)
  /\ LET ev == Head(out) IN
     /\ out' = Tail(out)
     /\ CASE ev[1] = "userMsg" ->
               \* message.updated, role user: the sent turn's anchor
               /\ anchored' = (anchored \/ (Open /\ ev[2] = sent))
               /\ unplaced' = (unplaced /\ ~(Open /\ ev[2] = sent))
               /\ UNCHANGED <<sent, answered, interrupted, ended, replying, held, pending, bus>>
          [] ev[1] = "text" ->
               /\ bus' = Append(bus, <<"text">>)
               /\ replying' = (replying \/ (Open /\ ev[2] = sent))
               /\ UNCHANGED <<sent, anchored, answered, interrupted, ended, unplaced, held, pending>>
          [] ev[1] = "finished" ->
               \* an assistant message newer than the anchor finished: only a reply to the sent turn;
               \* the sent turn's reply settling lets a held end out
               /\ answered' = (answered \/ (Open /\ ev[2] = sent))
               /\ IF ev[2] = sent
                    THEN /\ replying' = FALSE /\ held' = FALSE
                         /\ bus' = IF held THEN Append(bus, <<"turnDone">>) ELSE bus
                    ELSE UNCHANGED <<replying, held, bus>>
               /\ UNCHANGED <<sent, anchored, interrupted, ended, unplaced, pending>>
          [] ev[1] = "ask" ->
               /\ pending' = pending \cup {ev[2]} /\ bus' = Append(bus, <<"ask", ev[2]>>)
               /\ UNCHANGED turnVars
          [] ev[1] = "replied" ->
               \* track_ask: closes it unless this driver already took it
               /\ IF ev[2] \in pending
                    THEN pending' = pending \ {ev[2]} /\ bus' = Append(bus, <<"resolved", ev[2]>>)
                    ELSE UNCHANGED <<pending, bus>>
               /\ UNCHANGED turnVars
          [] ev[1] = "idle" ->
               \* decline_all: an idle OpenCode holds no ask, so any the driver still holds is closed
               LET closes == SetToSeq({<<"resolved", r>> : r \in pending}) IN
               /\ pending' = {}
               /\ IF Open /\ (answered \/ interrupted)
                    THEN EndTurn(closes) /\ UNCHANGED unplaced
                  ELSE IF Open /\ ~anchored
                    THEN unplaced' = TRUE /\ bus' = bus \o closes /\ UNCHANGED <<ended, held>>
                  ELSE bus' = bus \o closes /\ UNCHANGED <<ended, unplaced, held>>
               /\ UNCHANGED <<sent, anchored, answered, interrupted, replying>>
          [] ev[1] = "abortErr" ->
               \* session.error, MessageAbortedError: this turn's only if this driver asked
               /\ IF Open /\ interrupted
                    THEN EndTurn(<<>>) /\ UNCHANGED unplaced
                  ELSE IF Open /\ ~anchored
                    THEN unplaced' = TRUE /\ UNCHANGED <<ended, held, bus>>
                  ELSE UNCHANGED <<ended, unplaced, held, bus>>
               /\ UNCHANGED <<sent, anchored, answered, interrupted, replying, pending>>
  /\ UNCHANGED <<alive, busy, current, pipe, queue, cOpen, asked, drops, downs, live, inTurn,
                 parked, stored, viewVars, accepted, failed, delivered, interrupts, droppedStop, stopFor>>

\* expire: an idle or error no user message claimed ends the sent turn after UNPLACED_WAIT.
Expire ==
  /\ unplaced /\ Open /\ ~anchored
  /\ unplaced' = FALSE /\ ended' = TRUE
  /\ bus' = Append(bus, <<"turnDone">>)
  /\ UNCHANGED <<ocVars, sent, anchored, answered, interrupted, replying, held, pending, live,
                 inTurn, parked, stored, viewVars, accepted, failed, delivered, interrupts,
                 droppedStop, stopFor>>

\* SETTLE_WAIT runs out, or a send's next event comes: the held end goes out. OpenCode settles
\* a reply it cut short well inside SETTLE_WAIT, so the wait runs out only on a reply that will
\* not settle.
Settle ==
  /\ held /\ ~InSeq(<<"finished", sent>>, out)
  /\ held' = FALSE /\ replying' = FALSE
  /\ bus' = Append(bus, <<"turnDone">>)
  /\ UNCHANGED <<ocVars, sent, anchored, answered, interrupted, ended, unplaced, pending, live,
                 inTurn, parked, stored, viewVars, accepted, failed, delivered, interrupts,
                 droppedStop, stopFor>>

(* ---- spawn_relay + ActorState::handle_event (author_for's next_turn), the desktop ---- *)

\* On Exited the relay drops the driver and ends; the desktop clears its working state and its
\* cards and hands back its own unechoed messages, and the relay reports an agent's
\* (held::report_undelivered), so a message whose echo never got stored is reported failed.
RelayExited ==
  /\ bus /= <<>> /\ Head(bus) = <<"exited">>
  /\ bus' = <<>>
  /\ live' = FALSE /\ vLive' = FALSE
  /\ inTurn' = FALSE /\ parked' = FALSE /\ streaming' = FALSE /\ vOpen' = {}
  /\ failed' = failed \cup {m \in Msgs : InSeq(<<"echo", m>>, bus)}
  /\ UNCHANGED <<ocVars, turnVars, pending, stored, clicked, sends, accepted, delivered,
                 interrupts, droppedStop, stopFor>>

Relay ==
  /\ bus /= <<>> /\ Head(bus) /= <<"exited">>
  /\ LET ev == Head(bus) IN
     /\ bus' = Tail(bus)
     /\ inTurn' = CASE ev[1] = "echo" -> TRUE
                   [] ev[1] = "text" /\ parked -> TRUE
                   [] ev[1] = "turnDone" -> FALSE
                   [] OTHER -> inTurn
     /\ parked' = CASE ev[1] = "echo" /\ inTurn -> TRUE
                   [] ev[1] = "text" /\ ~inTurn -> FALSE
                   [] OTHER -> parked
     /\ stored' = IF ev[1] \in {"echo", "ask"} THEN stored \cup {ev[2]} ELSE stored
     /\ streaming' = CASE ev[1] \in {"echo", "ask", "text"} -> TRUE
                      [] ev[1] = "turnDone" -> FALSE
                      [] OTHER -> streaming
     \* OpenCode's permission names (bash, edit) never match a tool's (Bash, Edit), so a tool
     \* result never decides a card here
     /\ vOpen' = CASE ev[1] = "ask" -> vOpen \cup {ev[2]}
                  [] ev[1] = "resolved" -> vOpen \ {ev[2]}
                  [] OTHER -> vOpen
  /\ UNCHANGED <<ocVars, turnVars, pending, live, vLive, clicked, sends, accepted, failed,
                 delivered, interrupts, droppedStop, stopFor>>

ClientReload ==
  /\ vLive /\ live
  /\ vOpen' = pending \cap stored
  /\ UNCHANGED <<ocVars, turnVars, pending, bus, live, inTurn, parked, stored, vLive,
                 streaming, clicked, sends, accepted, failed, delivered, interrupts, droppedStop, stopFor>>

(* ---- the rules ---- *)

Quiet ==
  /\ out = <<>> /\ bus = <<>> /\ pipe = <<>> /\ ~(unplaced /\ Open /\ ~anchored) /\ ~held
  /\ alive => ~busy
  /\ alive \/ ~live

Done == Quiet /\ UNCHANGED vars

Next ==
  \/ \E m \in Composer : ClientSend(m)
  \/ \E m \in Agent : AgentSend(m)
  \/ \E r \in Asks : ClientAnswer(r) \/ OcAsk(r) \/ OcElsewhereReply(r)
  \/ Interrupt
  \/ OcRead \/ OcFinish \/ OcDown \/ ServerExited \/ SseDrop
  \/ DriverRead \/ Expire \/ Settle \/ Relay \/ RelayExited \/ ClientReload \/ Done

Spec == Init /\ [][Next]_vars

\* No chips and no held messages: a message is kept once its echo is stored, or reported failed
\* when the server's death drops the driver first.
Kept(m) ==
  \/ m \notin accepted
  \/ delivered[m] > 0
  \/ InSeq(<<"echo", m>>, bus)
  \/ m \in failed
  \/ m \in stored

NoLostUserMessage == \A m \in Composer : Kept(m)
NoLostAgentMessage == \A m \in Agent : Kept(m)

NoDoubleSend == \A m \in Msgs : delivered[m] <= 1

TurnProgress == Quiet => ~inTurn /\ ~streaming

AskResolution == Quiet => vOpen \subseteq cOpen

AskVisible == (out = <<>> /\ bus = <<>> /\ pipe = <<>>) => cOpen \subseteq vOpen

InterruptSafe ==
  (Quiet /\ interrupts > 0 /\ alive) => ~inTurn /\ ~streaming /\ vOpen = {}

\* The abort reaches OpenCode while a prompt sent before it is still to run.
InterruptHonored == ~droppedStop

TypeOK ==
  /\ alive \in BOOLEAN /\ busy \in BOOLEAN /\ replying \in BOOLEAN /\ held \in BOOLEAN
  /\ cOpen \subseteq Asks /\ asked \subseteq Asks /\ pending \subseteq Asks
  /\ vOpen \subseteq Asks /\ stored \subseteq Msgs \cup Asks
  /\ accepted \subseteq Msgs /\ failed \subseteq Msgs
=============================================================================
