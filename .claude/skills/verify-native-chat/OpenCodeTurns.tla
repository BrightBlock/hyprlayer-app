---------------------------- MODULE OpenCodeTurns ----------------------------
(* One composer session on the shared OpenCode HTTP server: SessionService::send /
   respond_permission / interrupt, OpenCodeDriver (send_user_turn's echo and
   prompt_async, the asks map, interrupt's drain and abort) and its filter task over the
   server's SSE stream (track_ask and the Translator's turn ending: sent turn, anchor,
   answered, interrupted, unplaced idle, the echoes it holds for messages sent mid-turn, and
   the deletes that hand back what a stopped turn left unread), OpenCode itself, the relay
   and the forwarder actor, and one desktop's sessionStore. The service, relay, actor and desktop steps are
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
  sent, anchored, answered, interrupted, ended, unplaced, replying, endHeld, pending, bus,
  \* the forwarder actor: in_turn, and a message that arrived mid-turn (next_turn)
  live, inTurn, parked, stored,
  vLive, streaming, vOpen, clicked, sends,
  accepted, failed, delivered, interrupts, droppedStop, stopFor,
  \* the driver: echoes held for messages sent while its turn ran, those a stopped turn left
  \* unread (being deleted), and the turn's events held back behind the deletes; OpenCode: user
  \* messages a stopped turn left in its history unread, which a later run reads
  oWaiting, unread, doneOwed, orphans,
  \* held_messages, the relay's dropped ids and resends, the desktop's chips and Stop
  held, dropped, resendDue, chip, stopped

vars == <<alive, busy, current, pipe, queue, cOpen, asked, out, drops, downs,
          sent, anchored, answered, interrupted, ended, unplaced, replying, endHeld, pending, bus,
          live, inTurn, parked, stored,
          vLive, streaming, vOpen, clicked, sends,
          accepted, failed, delivered, interrupts, droppedStop, stopFor,
          oWaiting, unread, doneOwed, orphans, held, dropped, resendDue, chip, stopped>>

ocVars == <<alive, busy, current, pipe, queue, cOpen, asked, out, drops, downs>>
turnVars == <<sent, anchored, answered, interrupted, ended, unplaced, replying, endHeld>>
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
  /\ ended = FALSE /\ unplaced = FALSE /\ replying = FALSE /\ endHeld = FALSE
  /\ pending = {} /\ bus = <<>>
  /\ live = TRUE /\ inTurn = FALSE /\ parked = FALSE /\ stored = {}
  /\ vLive = TRUE /\ streaming = FALSE /\ vOpen = {} /\ clicked = {}
  /\ sends = [m \in Composer |-> 0]
  /\ accepted = {} /\ failed = {} /\ delivered = [m \in Msgs |-> 0]
  /\ interrupts = 0 /\ droppedStop = FALSE /\ stopFor = {}
  /\ oWaiting = {} /\ unread = {} /\ doneOwed = FALSE /\ orphans = {}
  /\ held = {} /\ dropped = {} /\ resendDue = {} /\ chip = [m \in Composer |-> "none"]
  /\ stopped = FALSE

(* ---- send_user_turn: echo, begin_turn, prompt_async ---- *)

Open == sent /= None /\ ~ended

\* Always Ok. While the driver's turn runs the echo waits for the step that reads the message;
\* otherwise it goes out first. The sent turn is replaced, and the POST runs on its own.
SendServer(m) ==
  /\ live
  /\ IF Open THEN oWaiting' = oWaiting \cup {m} /\ UNCHANGED bus
             ELSE bus' = Append(bus, <<"echo", m>>) /\ UNCHANGED oWaiting
  /\ sent' = m /\ anchored' = FALSE /\ answered' = FALSE /\ interrupted' = FALSE
  /\ ended' = FALSE /\ unplaced' = FALSE /\ UNCHANGED <<replying, endHeld>>
  /\ pipe' = Append(pipe, <<"prompt", m>>)
  /\ accepted' = accepted \cup {m}

\* The composer steers only claude and codex; mid-turn its message waits in the local queue,
\* so a send goes out when the view shows idle (which can lag OpenCode). It stays `unechoed`
\* until its echo; a handed-back one is sent again once the view is idle.
ClientSend(m) ==
  /\ m \in Composer /\ vLive /\ ~streaming /\ sends[m] < 2
  /\ (chip[m] = "none" /\ sends[m] = 0) \/ chip[m] = "queued"
  /\ SendServer(m)
  /\ sends' = [sends EXCEPT ![m] = @ + 1]
  /\ streaming' = TRUE
  /\ chip' = [chip EXCEPT ![m] = "unechoed"] /\ stopped' = FALSE
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, pending,
                 live, inTurn, parked, stored, vLive, vOpen, clicked, failed, delivered,
                 interrupts, droppedStop, stopFor, unread, doneOwed, orphans, held, dropped,
                 resendDue>>

AgentSend(m) ==
  /\ m \in Agent /\ m \notin accepted
  /\ SendServer(m)
  /\ held' = held \cup {m}
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, pending,
                 live, inTurn, parked, stored, viewVars, failed, delivered, interrupts, droppedStop, stopFor,
                 unread, doneOwed, orphans, dropped, resendDue, chip, stopped>>

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
  /\ stopped' \in {stopped, TRUE}
  /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, sent,
                 anchored, answered, ended, unplaced, replying, endHeld, live, inTurn, parked, stored, vLive,
                 streaming, clicked, sends, accepted, failed, delivered, droppedStop>>

(* ---- OpenCode ---- *)

\* A run of m's prompt reads it and every message left unread in the history: each copy once.
Run(m) ==
  /\ delivered' = [x \in Msgs |-> delivered[x] + (IF x = m THEN 1 ELSE 0) + (IF x \in orphans THEN 1 ELSE 0)]
  /\ orphans' = {}

Read(m) == {m} \cup orphans

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
               /\ failed' = IF line[1] = "prompt" /\ live THEN failed \cup {line[2]} ELSE failed
               /\ UNCHANGED <<busy, current, queue, cOpen, out, delivered, orphans>>
          ELSE /\ UNCHANGED <<bus, failed>>
               /\ CASE line[1] = "prompt" /\ ~busy ->
                         /\ busy' = TRUE /\ current' = line[2] /\ Run(line[2])
                         /\ out' = out \o <<<<"userMsg", line[2]>>, <<"text", line[2], Read(line[2])>>>>
                         /\ UNCHANGED <<queue, cOpen>>
                    [] line[1] = "prompt" ->
                         /\ queue' = Append(queue, line[2])
                         /\ out' = Append(out, <<"userMsg", line[2]>>)
                         /\ UNCHANGED <<busy, current, cOpen, delivered, orphans>>
                    [] line[1] = "abort" /\ busy ->
                         \* the error and an idle first; then, if OpenCode had created the running
                         \* reply, its cut-short part settles and the reply ends with MessageAbortedError;
                         \* prompts held behind it that do not run stay in the history unread
                         \E keep, replied \in BOOLEAN :
                           LET aborted == IF replied
                                            THEN <<<<"abortErr">>, <<"idle">>, <<"text", current, {current}>>,
                                                   <<"finished", current>>>>
                                            ELSE <<<<"abortErr">>>>
                           IN
                           /\ cOpen' = {}
                           /\ IF keep /\ queue /= <<>>
                                THEN /\ current' = Head(queue) /\ queue' = Tail(queue)
                                     /\ Run(Head(queue))
                                     /\ out' = out \o aborted \o <<<<"text", Head(queue), Read(Head(queue))>>>>
                                     /\ UNCHANGED busy
                                ELSE /\ busy' = FALSE /\ current' = None /\ queue' = <<>>
                                     /\ orphans' = orphans \cup {queue[k] : k \in 1..Len(queue)}
                                     /\ out' = out \o aborted \o <<<<"idle">>>>
                                     /\ UNCHANGED delivered
                    [] line[1] = "abort" ->
                         UNCHANGED <<busy, current, queue, cOpen, out, delivered, orphans>>
                    [] line[1] = "reply" /\ line[2] \in cOpen ->
                         /\ cOpen' = cOpen \ {line[2]}
                         /\ out' = Append(out, <<"replied", line[2]>>)
                         /\ UNCHANGED <<busy, current, queue, delivered, orphans>>
                    [] line[1] = "reply" ->
                         UNCHANGED <<busy, current, queue, cOpen, out, delivered, orphans>>
  /\ UNCHANGED <<alive, asked, drops, downs, turnVars, pending, live, inTurn, parked, stored,
                 viewVars, accepted, interrupts, stopFor>>

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
       THEN /\ current' = Head(queue) /\ queue' = Tail(queue) /\ Run(Head(queue))
            /\ out' = out \o <<<<"finished", current>>, <<"text", Head(queue), Read(Head(queue))>>>>
            /\ UNCHANGED busy
       ELSE /\ busy' = FALSE /\ current' = None /\ queue' = <<>>
            /\ out' = out \o <<<<"finished", current>>, <<"idle">>>>
            /\ UNCHANGED <<delivered, orphans>>
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

\* Translator::end and end_turn: a stopped turn's waiting messages go to forget_unread, which the
\* filter task awaits before it sends the turn's events; a turn that ends while its reply is
\* open (`hold`) holds its TurnDone until that reply settles.
EndTurn(closes, hold) ==
  /\ ended' = TRUE
  /\ endHeld' = (endHeld \/ hold)
  /\ LET done == IF hold THEN <<>> ELSE <<<<"turnDone">>>> IN
     IF interrupted /\ oWaiting /= {}
       THEN /\ unread' = oWaiting /\ oWaiting' = {} /\ doneOwed' = TRUE
            /\ bus' = bus \o closes
       ELSE /\ bus' = bus \o closes \o done
            /\ UNCHANGED <<unread, oWaiting, doneOwed>>

\* A turn sent after the held end lets it out before anything else is read; nothing is read
\* while the deletes run.
DriverRead ==
  /\ out /= <<>> /\ ~(endHeld /\ Open) /\ ~doneOwed
  /\ LET ev == Head(out) IN
     /\ out' = Tail(out)
     /\ CASE ev[1] = "userMsg" ->
               \* message.updated, role user: the sent turn's anchor
               /\ anchored' = (anchored \/ (Open /\ ev[2] = sent))
               /\ unplaced' = (unplaced /\ ~(Open /\ ev[2] = sent))
               /\ UNCHANGED <<sent, answered, interrupted, ended, replying, endHeld, pending, bus,
                              oWaiting, unread, doneOwed>>
          [] ev[1] = "text" ->
               \* the step's parentID is at or past every message it read: their echoes go out
               /\ bus' = bus \o SetToSeq({<<"echo", w>> : w \in oWaiting \cap ev[3]}) \o <<<<"text">>>>
               /\ oWaiting' = oWaiting \ ev[3]
               /\ replying' = (replying \/ (Open /\ ev[2] = sent))
               /\ UNCHANGED <<sent, anchored, answered, interrupted, ended, unplaced, endHeld, pending,
                              unread, doneOwed>>
          [] ev[1] = "finished" ->
               \* an assistant message newer than the anchor finished: only a reply to the sent turn;
               \* the sent turn's reply settling lets a held end out
               /\ answered' = (answered \/ (Open /\ ev[2] = sent))
               /\ IF ev[2] = sent
                    THEN /\ replying' = FALSE /\ endHeld' = FALSE
                         /\ bus' = IF endHeld THEN Append(bus, <<"turnDone">>) ELSE bus
                    ELSE UNCHANGED <<replying, endHeld, bus>>
               /\ UNCHANGED <<sent, anchored, interrupted, ended, unplaced, pending, oWaiting,
                              unread, doneOwed>>
          [] ev[1] = "ask" ->
               /\ pending' = pending \cup {ev[2]} /\ bus' = Append(bus, <<"ask", ev[2]>>)
               /\ UNCHANGED <<turnVars, oWaiting, unread, doneOwed>>
          [] ev[1] = "replied" ->
               \* track_ask: closes it unless this driver already took it
               /\ IF ev[2] \in pending
                    THEN pending' = pending \ {ev[2]} /\ bus' = Append(bus, <<"resolved", ev[2]>>)
                    ELSE UNCHANGED <<pending, bus>>
               /\ UNCHANGED <<turnVars, oWaiting, unread, doneOwed>>
          [] ev[1] = "idle" ->
               \* decline_all: an idle OpenCode holds no ask, so any the driver still holds is closed
               LET closes == SetToSeq({<<"resolved", r>> : r \in pending}) IN
               /\ pending' = {}
               /\ IF Open /\ (answered \/ interrupted)
                    THEN EndTurn(closes, replying) /\ UNCHANGED unplaced
                  ELSE IF Open /\ ~anchored
                    THEN unplaced' = TRUE /\ bus' = bus \o closes
                         /\ UNCHANGED <<ended, endHeld, oWaiting, unread, doneOwed>>
                  ELSE bus' = bus \o closes /\ UNCHANGED <<ended, unplaced, endHeld, oWaiting, unread, doneOwed>>
               /\ UNCHANGED <<sent, anchored, answered, interrupted, replying>>
          [] ev[1] = "abortErr" ->
               \* session.error, MessageAbortedError: this turn's only if this driver asked
               /\ IF Open /\ interrupted
                    THEN EndTurn(<<>>, replying) /\ UNCHANGED unplaced
                  ELSE IF Open /\ ~anchored
                    THEN unplaced' = TRUE /\ UNCHANGED <<ended, endHeld, bus, oWaiting, unread, doneOwed>>
                  ELSE UNCHANGED <<ended, unplaced, endHeld, bus, oWaiting, unread, doneOwed>>
               /\ UNCHANGED <<sent, anchored, answered, interrupted, replying, pending>>
  /\ UNCHANGED <<alive, busy, current, pipe, queue, cOpen, asked, drops, downs, live, inTurn,
                 parked, stored, viewVars, accepted, failed, delivered, interrupts, droppedStop, stopFor,
                 orphans, held, dropped, resendDue, chip, stopped>>

\* expire: an idle or error no user message claimed ends the sent turn after UNPLACED_WAIT.
Expire ==
  /\ unplaced /\ Open /\ ~anchored /\ ~doneOwed
  /\ unplaced' = FALSE /\ EndTurn(<<>>, FALSE)
  /\ UNCHANGED <<ocVars, sent, anchored, answered, interrupted, replying, pending, live,
                 inTurn, parked, stored, viewVars, accepted, failed, delivered, interrupts,
                 droppedStop, stopFor, orphans, held, dropped, resendDue, chip, stopped>>

\* SETTLE_WAIT runs out, or a send's next event comes: the held end goes out. OpenCode settles
\* a reply it cut short well inside SETTLE_WAIT, so the wait runs out only on a reply that will
\* not settle.
Settle ==
  /\ endHeld /\ ~doneOwed /\ ~InSeq(<<"finished", sent>>, out)
  /\ endHeld' = FALSE /\ replying' = FALSE
  /\ bus' = Append(bus, <<"turnDone">>)
  /\ UNCHANGED <<ocVars, sent, anchored, answered, interrupted, ended, unplaced, pending, live,
                 inTurn, parked, stored, viewVars, accepted, failed, delivered, interrupts,
                 droppedStop, stopFor>>

\* forget_unread: DELETE each message a stopped turn left unread (OpenCode answers 409 while it
\* runs, and a dead server nothing), hand back each one deleted, then the turn's TurnDone unless
\* its reply holds it. One OpenCode would not delete waits for the step that reads it.
DropUnread ==
  /\ doneOwed
  /\ LET gone == IF alive /\ ~busy THEN unread \cap orphans ELSE {} IN
     /\ orphans' = orphans \ gone
     /\ oWaiting' = oWaiting \cup (unread \ gone)
     /\ bus' = bus \o SetToSeq({<<"declined", m>> : m \in gone})
                   \o (IF endHeld THEN <<>> ELSE <<<<"turnDone">>>>)
  /\ unread' = {} /\ doneOwed' = FALSE
  /\ UNCHANGED <<alive, busy, current, pipe, queue, cOpen, asked, out, drops, downs, turnVars,
                 pending, live, inTurn, parked, stored, viewVars, accepted, failed, delivered,
                 interrupts, droppedStop, stopFor, held, dropped, resendDue, chip, stopped>>

(* ---- spawn_relay + ActorState::handle_event (author_for's next_turn), the desktop ---- *)

\* On Exited the relay drops the driver and ends; the desktop clears its working state and its
\* cards and hands back its own unechoed messages, and the relay reports an agent's
\* (held::report_undelivered).
RelayExited ==
  /\ bus /= <<>> /\ Head(bus) = <<"exited">> /\ resendDue = {}
  /\ bus' = <<>>
  /\ live' = FALSE /\ vLive' = FALSE
  /\ inTurn' = FALSE /\ parked' = FALSE /\ streaming' = FALSE /\ vOpen' = {}
  /\ chip' = [m \in Composer |-> IF chip[m] = "unechoed"
                                   THEN IF stopped THEN "returned" ELSE "queued"
                                   ELSE chip[m]]
  /\ failed' = failed \cup held /\ held' = {} /\ dropped' = {}
  /\ UNCHANGED <<ocVars, turnVars, pending, stored, clicked, sends, accepted, delivered,
                 interrupts, droppedStop, stopFor, oWaiting, unread, doneOwed, orphans,
                 resendDue, stopped>>

Relay ==
  /\ bus /= <<>> /\ Head(bus) /= <<"exited">> /\ resendDue = {}
  /\ LET ev == Head(bus)
         back == IF stopped THEN "returned" ELSE "queued"
     IN
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
     /\ chip' = CASE ev[1] = "echo" /\ ev[2] \in Composer -> [chip EXCEPT ![ev[2]] = "none"]
                 [] ev[1] = "declined" /\ ev[2] \in Composer /\ chip[ev[2]] = "unechoed" ->
                      [chip EXCEPT ![ev[2]] = back]
                 [] OTHER -> chip
     /\ held' = IF ev[1] = "echo" THEN held \ {ev[2]} ELSE held
     /\ dropped' = CASE ev[1] = "declined" -> dropped \cup {ev[2]}
                    [] ev[1] = "turnDone" -> {}
                    [] OTHER -> dropped
     /\ resendDue' = IF ev[1] = "turnDone" THEN dropped \cap held ELSE {}
  /\ UNCHANGED <<ocVars, turnVars, pending, live, vLive, clicked, sends, accepted, failed,
                 delivered, interrupts, droppedStop, stopFor, oWaiting, unread, doneOwed, orphans,
                 stopped>>

\* held::send_again: OpenCode's send never fails.
RelayResend ==
  \E m \in resendDue :
    /\ resendDue' = resendDue \ {m}
    /\ SendServer(m)
    /\ UNCHANGED <<alive, busy, current, queue, cOpen, asked, out, drops, downs, pending,
                   live, inTurn, parked, stored, viewVars, failed, delivered, interrupts,
                   droppedStop, stopFor, unread, doneOwed, orphans, held, dropped, chip, stopped>>

ClientReload ==
  /\ vLive /\ live
  /\ vOpen' = pending \cap stored
  /\ UNCHANGED <<ocVars, turnVars, pending, bus, live, inTurn, parked, stored, vLive,
                 streaming, clicked, sends, accepted, failed, delivered, interrupts, droppedStop, stopFor>>

(* ---- the rules ---- *)

Quiet ==
  /\ out = <<>> /\ bus = <<>> /\ pipe = <<>> /\ ~(unplaced /\ Open /\ ~anchored) /\ ~endHeld
  /\ ~doneOwed /\ resendDue = {}
  /\ alive => ~busy
  /\ alive \/ ~live

Done == Quiet /\ UNCHANGED vars

\* The steps that leave the driver's held echoes, OpenCode's unread history, the relay's
\* resends and the desktop's chips as they are.
Frame(A) == A /\ UNCHANGED <<oWaiting, unread, doneOwed, orphans, held, dropped, resendDue,
                              chip, stopped>>

Next ==
  \/ \E m \in Composer : ClientSend(m)
  \/ \E m \in Agent : AgentSend(m)
  \/ \E r \in Asks : Frame(ClientAnswer(r) \/ OcAsk(r) \/ OcElsewhereReply(r))
  \/ Interrupt /\ UNCHANGED <<oWaiting, unread, doneOwed, orphans, held, dropped, resendDue, chip>>
  \/ OcRead /\ UNCHANGED <<oWaiting, unread, doneOwed, held, dropped, resendDue, chip, stopped>>
  \/ OcFinish /\ UNCHANGED <<oWaiting, unread, doneOwed, held, dropped, resendDue, chip, stopped>>
  \/ Frame(OcDown \/ ServerExited \/ SseDrop \/ Settle \/ ClientReload)
  \/ DriverRead \/ Expire \/ DropUnread \/ Relay \/ RelayExited \/ RelayResend \/ Done

Spec == Init /\ [][Next]_vars

OnItsWay(m) ==
  \/ InSeq(<<"prompt", m>>, pipe)
  \/ alive /\ InSeq(m, queue)
  \/ InSeq(<<"echo", m>>, bus) \/ InSeq(<<"declined", m>>, bus)
  \/ m \in unread \cup resendDue

\* No relaunch: a message is kept once OpenCode reads it, or reported failed when the server's
\* death drops the driver first. A stored echo counts only while the message is not left unread
\* in OpenCode's history (before the held echo, every echo was stored at send). A dead server's
\* history stays for a resume to read, which the model leaves out.
Kept(m) ==
  \/ m \notin accepted
  \/ delivered[m] > 0
  \/ OnItsWay(m)
  \/ m \in failed
  \/ m \in stored \ orphans
  \/ m \in Composer /\ chip[m] /= "none"
  \/ m \in held /\ ~Quiet
  \/ ~alive /\ m \in orphans

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
  /\ alive \in BOOLEAN /\ busy \in BOOLEAN /\ replying \in BOOLEAN /\ endHeld \in BOOLEAN
  /\ cOpen \subseteq Asks /\ asked \subseteq Asks /\ pending \subseteq Asks
  /\ vOpen \subseteq Asks /\ stored \subseteq Msgs \cup Asks
  /\ accepted \subseteq Msgs /\ failed \subseteq Msgs
  /\ oWaiting \subseteq Msgs /\ unread \subseteq Msgs /\ orphans \subseteq Msgs
  /\ held \subseteq Agent /\ chip \in [Composer -> {"none", "unechoed", "queued", "returned"}]
=============================================================================
