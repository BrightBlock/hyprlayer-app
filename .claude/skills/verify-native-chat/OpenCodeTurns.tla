---------------------------- MODULE OpenCodeTurns ----------------------------
(* One composer session on the shared OpenCode 2 server (@opencode/cli 2.4+):
   SessionService::send / respond_permission / interrupt, OpenCodeDriver (send_user_turn's
   echo or held echo, the POSTs it queues in order, interrupt's ask drain, stop and inbox
   cancels) and its filter task over the server's event stream (Core::take: inbox delivered
   and cancelled, a run's start and end, asks; the read-back after a gap in the stream), OpenCode
   itself (its inbox, runs, asks, death), the relay and the forwarder actor, and one desktop's
   sessionStore, which steers a message sent mid-turn. The relay, actor and desktop steps are
   SessionTurns' (claude); SKILL.md maps every action to the code. *)
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS Composer, Agent, Asks,
          MaxInterrupts,
          MaxDrops,      \* events the driver never sees (a stream reconnect, a lagged receiver)
          MaxDowns,      \* the OpenCode server dying
          MaxWakes,      \* runs OpenCode starts on its own: a subagent's result waking the session
          MaxFails       \* runs that fail

Msgs == Composer \cup Agent

VARIABLES
  \* OpenCode: up, its inbox of undelivered messages, whether a run is going, whether the inbox
  \* waits for a new prompt (a stopped run leaves it parked), the messages the running run read,
  \* open asks, the event stream, and each message's deliveries
  alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, out, drops, downs, wakes, fails,
  delivered,
  \* what a read-back sees: messages delivered since their latest send, and the last run's end
  ocDelivered, lastEnd,
  \* the driver: its turn (open, a message of it delivered, a run going, a stop asked, a run it
  \* did not start), its undelivered messages and those
  \* whose echo waits, the asks it holds and has closed, its events out, its POSTs in order, and
  \* the queued events a read-back already covers
  open, deliv, executing, stopping, foreign, undel, heldEcho, pending, closed, bus, pipe,
  older,
  \* a stopped turn's TurnDone held until the stop's withdrawals of held messages come back
  doneOwed, stopCancels,
  \* messages already steered again once after a run ended without reading them, and those whose
  \* prompt OpenCode has answered
  woken, posted,
  \* the messages a stop is withdrawing: those undelivered when it was pressed
  stopTargets,
  \* the service and forwarder actor: in_turn, next_turn, the stored rows, the relay's held agent
  \* messages, the ids handed back since the last TurnDone, and the resends due
  live, inTurn, nextTurn, stored, accepted, failed, held, dropped, resendDue,
  \* the desktop
  vLive, streaming, vOpen, clicked, sends, chip, stopped,
  \* the rules' bookkeeping: stops pressed, and the messages a stop was for
  interrupts, droppedStop, stopFor, beforeStop

ocVars == <<alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, out, drops, downs, wakes, fails,
            delivered, ocDelivered, lastEnd>>
ocRest == <<alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, drops, downs, wakes, fails,
            delivered, ocDelivered, lastEnd>>
drvVars == <<open, deliv, executing, stopping, foreign, undel, heldEcho, pending, closed,
             bus, pipe, older, doneOwed, stopCancels, woken, posted, stopTargets>>
svcVars == <<live, inTurn, nextTurn, stored, accepted, failed, held, dropped, resendDue>>
viewVars == <<vLive, streaming, vOpen, clicked, sends, chip, stopped>>
ruleVars == <<interrupts, droppedStop, stopFor, beforeStop>>
vars == <<ocVars, drvVars, svcVars, viewVars, ruleVars>>

InSeq(x, s) == \E i \in 1..Len(s) : s[i] = x
Range(s) == {s[i] : i \in 1..Len(s)}
Without(s, x) == SelectSeq(s, LAMBDA y : y /= x)

RECURSIVE SetToSeq(_)
SetToSeq(S) == IF S = {} THEN <<>> ELSE LET x == CHOOSE y \in S : TRUE IN <<x>> \o SetToSeq(S \ {x})

Closes(S) == SetToSeq({<<"resolved", r>> : r \in S})

Init ==
  /\ alive = TRUE /\ inbox = <<>> /\ running = FALSE /\ parkedInbox = FALSE /\ runMsgs = {}
  /\ cOpen = {} /\ asked = {} /\ out = <<>> /\ drops = 0 /\ downs = 0 /\ wakes = 0 /\ fails = 0
  /\ delivered = [m \in Msgs |-> 0] /\ ocDelivered = {} /\ lastEnd = "succeeded"
  /\ open = FALSE /\ deliv = FALSE /\ executing = FALSE /\ stopping = FALSE
  /\ foreign = FALSE /\ undel = {} /\ heldEcho = {} /\ pending = {} /\ closed = {}
  /\ bus = <<>> /\ pipe = <<>> /\ older = 0 /\ doneOwed = FALSE /\ stopCancels = {} /\ woken = {} /\ posted = {} /\ stopTargets = {}
  /\ live = TRUE /\ inTurn = FALSE /\ nextTurn = FALSE /\ stored = {} /\ accepted = {}
  /\ failed = {} /\ held = {} /\ dropped = {} /\ resendDue = {}
  /\ vLive = TRUE /\ streaming = FALSE /\ vOpen = {} /\ clicked = {}
  /\ sends = [m \in Composer |-> 0] /\ chip = [m \in Composer |-> "none"] /\ stopped = FALSE
  /\ interrupts = 0 /\ droppedStop = FALSE /\ stopFor = {} /\ beforeStop = {}

(* ---- send_user_turn: the echo, or the echo held for the delivery; the prompt POST ---- *)

\* Always Ok. Mid-turn the echo of a message whose sender hears back waits for OpenCode to read
\* it; otherwise it goes out first and opens the turn.
SendServer(m) ==
  /\ live
  \* a stop pressed with no turn open is not this new turn's
  /\ IF open
       THEN /\ heldEcho' = heldEcho \cup {m} /\ UNCHANGED <<bus, open, deliv, stopping>>
       ELSE /\ bus' = Append(bus, <<"echo", m>>) /\ open' = TRUE /\ deliv' = FALSE
            /\ stopping' = FALSE
            /\ UNCHANGED heldEcho
  /\ undel' = undel \cup {m}
  /\ pipe' = Append(pipe, <<"prompt", m>>)
  /\ accepted' = accepted \cup {m}
  /\ ocDelivered' = ocDelivered \ {m}
  /\ woken' = woken \ {m} /\ posted' = posted \ {m} /\ stopTargets' = stopTargets \ {m}

\* The composer steers OpenCode: mid-turn its message goes out at once (`steerMessage`) and stays
\* `unechoed` until its echo; a handed-back one is sent again once the view is idle.
ClientSend(m) ==
  /\ m \in Composer /\ vLive /\ sends[m] < 2
  /\ (chip[m] = "none" /\ sends[m] = 0) \/ (chip[m] = "queued" /\ ~streaming)
  /\ SendServer(m)
  /\ sends' = [sends EXCEPT ![m] = @ + 1]
  /\ streaming' = TRUE
  /\ chip' = [chip EXCEPT ![m] = "unechoed"] /\ stopped' = FALSE
  /\ UNCHANGED <<alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, out, drops, downs, wakes, fails,
                 delivered, lastEnd, executing, foreign, pending, closed, older, doneOwed, stopCancels,
                 live, inTurn, nextTurn, stored, failed, held, dropped, resendDue,
                 vLive, vOpen, clicked, ruleVars>>

AgentSend(m) ==
  /\ m \in Agent /\ m \notin accepted
  /\ SendServer(m)
  /\ held' = held \cup {m}
  /\ UNCHANGED <<alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, out, drops, downs, wakes, fails,
                 delivered, lastEnd, executing, foreign, pending, closed, older, doneOwed, stopCancels,
                 live, inTurn, nextTurn, stored, failed, dropped, resendDue, viewVars, ruleVars>>

(* ---- respond_permission and interrupt ---- *)

\* take_ask: an ask the driver holds closes at once; the reply is POSTed either way.
ClientAnswer(r) ==
  /\ r \in vOpen \ clicked /\ live
  /\ clicked' = clicked \cup {r}
  /\ IF r \in pending
       THEN pending' = pending \ {r} /\ closed' = closed \cup {r}
            /\ bus' = Append(bus, <<"resolved", r>>)
       ELSE UNCHANGED <<pending, closed, bus>>
  /\ pipe' = Append(pipe, <<"reply", r>>)
  /\ UNCHANGED <<ocVars, open, deliv, executing, stopping, foreign, undel, heldEcho,
                 older, doneOwed, stopCancels, woken, posted, stopTargets, svcVars, vLive, streaming, vOpen, sends, chip, stopped, ruleVars>>

\* interrupt: every held ask closes as declined; a cancel for each undelivered message, then the
\* stop, go out in order behind what is queued. The stop stays asked until a run's end, or the
\* stop finding none, settles it, with or without a turn open.
Interrupt ==
  /\ interrupts < MaxInterrupts /\ live
  /\ interrupts' = interrupts + 1
  /\ bus' = bus \o Closes(pending)
  /\ closed' = closed \cup pending /\ pending' = {}
  /\ stopping' = (stopping \/ open \/ executing \/ undel /= {})
  \* withdrawals first: a run a subagent's result wakes must find none of them parked
  /\ pipe' = pipe \o SetToSeq({<<"cancel", m>> : m \in undel}) \o <<<<"interrupt">>>>
  /\ beforeStop' = accepted
  /\ stopTargets' = stopTargets \cup undel
  /\ stopCancels' = stopCancels \cup heldEcho
  /\ stopped' \in {stopped, TRUE}
  /\ UNCHANGED <<ocVars, open, deliv, executing, foreign, undel, heldEcho, older, doneOwed, woken, posted,
                 svcVars, vLive, streaming, vOpen, clicked, sends, chip, droppedStop, stopFor>>

(* ---- OpenCode ---- *)

\* OpenCode handles the driver's POSTs one at a time, in order (`spawn_posts` sends each once
\* the one before is answered). A dead server fails every one.
OcPost ==
  /\ pipe /= <<>>
  /\ LET p == Head(pipe) IN
     /\ pipe' = Tail(pipe)
     /\ IF ~alive
          THEN \* post_failed: a held echo is handed back; an echoed message's turn fails
               /\ IF p[1] = "prompt" /\ live
                    THEN /\ undel' = undel \ {p[2]} /\ heldEcho' = heldEcho \ {p[2]}
                         /\ failed' = failed \cup {p[2]}
                         /\ LET last == open /\ ~deliv /\ (undel \ heldEcho) = {p[2]} IN
                            /\ bus' = IF p[2] \in heldEcho THEN Append(bus, <<"declined", p[2]>>)
                                      ELSE bus \o <<<<"text">>>> \o (IF last THEN <<<<"turnDone">>>> ELSE <<>>)
                            /\ open' = (open /\ ~(last /\ p[2] \notin heldEcho))
                            /\ stopping' = (stopping /\ open')
                    ELSE UNCHANGED <<undel, heldEcho, failed, bus, open, stopping, doneOwed, posted>>
               /\ UNCHANGED <<inbox, running, parkedInbox, runMsgs, cOpen, out, ocDelivered, stopFor, doneOwed, posted>>
          ELSE /\ UNCHANGED <<failed>>
               /\ CASE p[1] = "prompt" ->
                         \* admitted, it starts a run at once if none is going (the race probe:
                         \* a stop right after the prompt's answer always found the run)
                         /\ inbox' = Append(inbox, p[2]) /\ parkedInbox' = FALSE
                         /\ running' = TRUE /\ runMsgs' = IF running THEN runMsgs ELSE {}
                         /\ out' = out \o <<<<"enqueued", p[2]>>>> \o (IF running THEN <<>> ELSE <<<<"started">>>>)
                         /\ posted' = posted \cup {p[2]}
                         /\ UNCHANGED <<cOpen, undel, heldEcho, bus, open, stopping, ocDelivered,
                                        stopFor, doneOwed>>
                    [] p[1] = "interrupt" /\ running ->
                         \* the run stops and drops its asks with no reply; the inbox waits parked,
                         \* and the stop is for what it still holds
                         /\ running' = FALSE /\ runMsgs' = {} /\ cOpen' = {}
                         /\ parkedInbox' = (inbox /= <<>>)
                         /\ out' = Append(out, <<"ended", "interrupted">>)
                         /\ stopFor' = Range(inbox) \cap beforeStop
                         /\ UNCHANGED <<inbox, undel, heldEcho, bus, open, stopping, ocDelivered, doneOwed, posted>>
                    [] p[1] = "interrupt" ->
                         \* interrupted: false. stop_missed: with no run going and none seen, the
                         \* turn ends now, or once the stop's withdrawals come back
                         /\ LET missed == stopping /\ ~executing
                                 owe == missed /\ (stopCancels \cap undel) /= {}
                             IN
                            /\ bus' = IF missed /\ ~owe THEN Append(bus, <<"turnDone">>) ELSE bus
                            /\ doneOwed' = (doneOwed \/ owe)
                            /\ open' = (open /\ ~missed) /\ stopping' = (stopping /\ ~missed)
                         /\ parkedInbox' = (parkedInbox \/ inbox /= <<>>)
                         /\ stopFor' = Range(inbox) \cap beforeStop
                         /\ UNCHANGED <<inbox, running, runMsgs, cOpen, out, undel, heldEcho, ocDelivered, posted>>
                    [] p[1] = "cancel" /\ InSeq(p[2], inbox) ->
                         /\ inbox' = Without(inbox, p[2])
                         /\ out' = Append(out, <<"cancelled", p[2]>>)
                         \* withdrawn, it never runs for this stop; a resend is a new message
                         /\ stopFor' = stopFor \ {p[2]}
                         /\ UNCHANGED <<running, parkedInbox, runMsgs, cOpen, undel, heldEcho, bus, open,
                                        stopping, ocDelivered, doneOwed, posted>>
                    [] p[1] = "wake" /\ InSeq(p[2], inbox) ->
                         \* steering a parked message starts a run for it
                         /\ parkedInbox' = FALSE /\ running' = TRUE
                         /\ runMsgs' = IF running THEN runMsgs ELSE {}
                         /\ out' = IF running THEN out ELSE Append(out, <<"started">>)
                         /\ UNCHANGED <<inbox, cOpen, undel, heldEcho, bus, open, stopping,
                                        ocDelivered, stopFor, doneOwed, posted>>
                    [] p[1] = "reply" /\ p[2] \in cOpen ->
                         /\ cOpen' = cOpen \ {p[2]}
                         /\ out' = Append(out, <<"replied", p[2]>>)
                         /\ UNCHANGED <<inbox, running, parkedInbox, runMsgs, undel, heldEcho, bus, open,
                                        stopping, ocDelivered, stopFor, doneOwed, posted>>
                    [] OTHER ->
                         UNCHANGED <<inbox, running, parkedInbox, runMsgs, cOpen, out, undel, heldEcho,
                                     bus, open, stopping, ocDelivered, stopFor, doneOwed, posted>>
  /\ UNCHANGED <<alive, asked, drops, downs, wakes, fails, delivered, lastEnd, deliv, executing, foreign, pending,
                 closed, older, stopCancels, woken, stopTargets, live, inTurn, nextTurn, stored, accepted, held, dropped, resendDue,
                 viewVars, interrupts, droppedStop, beforeStop>>

\* A subagent's result wakes the session with a run no prompt started.
OcWake ==
  /\ alive /\ ~running /\ wakes < MaxWakes
  /\ running' = TRUE /\ runMsgs' = {} /\ wakes' = wakes + 1
  /\ out' = Append(out, <<"started">>)
  /\ UNCHANGED <<alive, inbox, parkedInbox, cOpen, asked, drops, downs, fails, delivered, ocDelivered, lastEnd,
                 drvVars, svcVars, viewVars, ruleVars>>

\* At a step boundary the run reads the next inbox message, steered in, and answers it.
OcDeliver ==
  /\ alive /\ running /\ inbox /= <<>> /\ cOpen = {}
  /\ LET m == Head(inbox) IN
     /\ inbox' = Tail(inbox)
     /\ delivered' = [delivered EXCEPT ![m] = @ + 1]
     /\ runMsgs' = runMsgs \cup {m} /\ ocDelivered' = ocDelivered \cup {m}
     /\ out' = out \o <<<<"delivered", m>>, <<"text">>>>
  /\ UNCHANGED <<alive, running, parkedInbox, cOpen, asked, drops, downs, wakes, fails, lastEnd, drvVars,
                 svcVars, viewVars, ruleVars>>

OcAsk(r) ==
  /\ alive /\ running /\ r \notin asked
  /\ asked' = asked \cup {r} /\ cOpen' = cOpen \cup {r}
  /\ out' = Append(out, <<"ask", r>>)
  /\ UNCHANGED <<alive, inbox, running, parkedInbox, runMsgs, drops, downs, wakes, fails, delivered,
                 ocDelivered, lastEnd, drvVars, svcVars, viewVars, ruleVars>>

\* Another OpenCode client (its TUI or web UI) answers the ask: permission.replied.
OcElsewhereReply(r) ==
  /\ alive /\ r \in cOpen
  /\ cOpen' = cOpen \ {r}
  /\ out' = Append(out, <<"replied", r>>)
  /\ UNCHANGED <<alive, inbox, running, parkedInbox, runMsgs, asked, drops, downs, wakes, fails, delivered,
                 ocDelivered, lastEnd, drvVars, svcVars, viewVars, ruleVars>>

\* The run ends: it succeeds once the inbox is drained, or fails at any point, dropping its asks;
\* a failed run leaves the rest of the inbox parked.
OcFinish(outcome) ==
  /\ alive /\ running
  /\ outcome = "succeeded" => (inbox = <<>> /\ cOpen = {})
  /\ outcome = "failed" => fails < MaxFails
  /\ fails' = fails + (IF outcome = "failed" THEN 1 ELSE 0)
  /\ running' = FALSE /\ runMsgs' = {} /\ cOpen' = {} /\ lastEnd' = outcome
  /\ parkedInbox' = (inbox /= <<>>)
  /\ out' = Append(out, <<"ended", outcome>>)
  \* a stop OpenCode took while this run's message waited to run did not stop it
  /\ droppedStop' = (droppedStop \/ (outcome = "succeeded" /\ runMsgs \cap stopFor /= {}))
  /\ stopFor' = stopFor \ runMsgs
  /\ UNCHANGED <<alive, inbox, asked, drops, downs, wakes, delivered, ocDelivered, drvVars,
                 svcVars, viewVars, interrupts, beforeStop>>

\* The server dies: its runs and its event stream with it. Its durable inbox stays for a resume,
\* which the model leaves out.
OcDown ==
  /\ alive /\ downs < MaxDowns
  /\ alive' = FALSE /\ downs' = downs + 1
  /\ running' = FALSE /\ runMsgs' = {} /\ cOpen' = {} /\ out' = <<>>
  /\ UNCHANGED <<inbox, parkedInbox, asked, drops, wakes, fails, delivered, ocDelivered, lastEnd,
                 drvVars, svcVars, viewVars, ruleVars>>

\* watch_exit sees the child gone and every driver on the server sends Exited (crashed).
ServerExited ==
  /\ ~alive /\ live /\ ~InSeq(<<"exited">>, bus)
  /\ bus' = Append(bus, <<"exited">>)
  /\ UNCHANGED <<ocVars, open, deliv, executing, stopping, foreign, undel, heldEcho,
                 pending, closed, pipe, older, doneOwed, stopCancels, woken, posted, stopTargets, svcVars, viewVars, ruleVars>>

\* An event the driver never sees: a reconnect gap, or its broadcast receiver lagging. The new
\* stream starts with server.connected; a lagged receiver's next read is the Lagged error.
SseDrop ==
  /\ out /= <<>> /\ drops < MaxDrops /\ Head(out) /= <<"connected">>
  /\ out' = <<<<"connected">>>> \o Tail(out) /\ drops' = drops + 1
  /\ UNCHANGED <<alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, downs, wakes, fails, delivered,
                 ocDelivered, lastEnd, drvVars, svcVars, viewVars, ruleVars>>

(* ---- the filter task: Core::take, and the read-back ---- *)

\* execution_end: the run drops the asks the driver holds; the turn ends if this run read one of
\* its messages or a stop was asked, and a run no prompt started closes on its own. A stopped
\* turn's end waits for the stop's withdrawals of its held messages, so each hand-back comes
\* first (the relay resends an agent's at the TurnDone). Unless a stop is withdrawing them (those
\* undelivered when it was pressed), the messages OpenCode holds are steered again once, which starts a run for them: a run that
\* never read them, or one that failed before it did, leaves them parked. One a second run also
\* left unread is withdrawn and handed back, as a stop would, so a failing run is not woken forever.
EndRun(pre, undelNow) ==
  LET ours == (open /\ deliv) \/ stopping
      done == ours \/ (~open /\ foreign)
      parked == (undelNow \cap posted) \ stopTargets
      again == parked \ woken
      giveUp == parked \cap woken
      cancels == stopCancels \cup (giveUp \cap heldEcho)
      \* with no turn left open, a hand-back still needs a TurnDone after it for the relay
      owe == (done \/ ~open) /\ (cancels \cap undelNow) /= {}
  IN
  /\ executing' = FALSE /\ foreign' = FALSE
  /\ closed' = closed \cup pending /\ pending' = {}
  /\ bus' = pre \o Closes(pending) \o (IF done /\ ~owe THEN <<<<"turnDone">>>> ELSE <<>>)
  /\ doneOwed' = (doneOwed \/ owe)
  /\ open' = (open /\ ~ours) /\ deliv' = (deliv /\ ~ours) /\ stopping' = (stopping /\ ~ours)
  /\ pipe' = pipe \o SetToSeq({<<"wake", m>> : m \in again}) \o SetToSeq({<<"cancel", m>> : m \in giveUp})
  /\ woken' = woken \cup again
  /\ stopCancels' = cancels

\* The owed end goes out once the last withdrawal came back.
Settled(pre, cancels) ==
  IF doneOwed /\ cancels \cap undel' = {} THEN Append(pre, <<"turnDone">>) ELSE pre

\* inbox.delivered: the held echo goes out; a delivery opens the turn if none is open.
Deliver(m) ==
  /\ undel' = undel \ {m} /\ heldEcho' = heldEcho \ {m} /\ stopCancels' = stopCancels \ {m}
  /\ LET pre == IF m \in heldEcho THEN Append(bus, <<"echo", m>>) ELSE bus IN
     /\ bus' = Settled(pre, stopCancels \ {m})
     /\ doneOwed' = (doneOwed /\ (stopCancels \ {m}) \cap undel' /= {})
  /\ open' = (open \/ ~doneOwed) /\ deliv' = (deliv \/ ~doneOwed)

\* inbox.cancelled: a held one is handed back; the last echoed one of a turn nothing of went to a
\* run ends it, a run going or not.
Cancel(m) ==
  /\ undel' = undel \ {m} /\ heldEcho' = heldEcho \ {m} /\ stopCancels' = stopCancels \ {m}
  /\ LET last == open /\ ~deliv /\ m \notin heldEcho /\ (undel \ heldEcho) = {m}
         rest == (stopCancels \ {m}) \cap (undel \ {m}) /= {}
         pre == IF m \in heldEcho THEN Append(bus, <<"declined", m>>)
                ELSE IF last /\ ~rest THEN Append(bus, <<"turnDone">>) ELSE bus
     IN
     /\ bus' = Settled(pre, stopCancels \ {m})
     /\ doneOwed' = ((doneOwed \/ last) /\ rest)
     /\ open' = (open /\ ~last) /\ stopping' = (stopping /\ ~last)
     \* a run still going goes on outside a turn, and its end closes it
     /\ foreign' = (foreign \/ (last /\ executing))

DriverRead ==
  /\ out /= <<>> /\ Head(out) /= <<"connected">>
  /\ LET ev == Head(out)
         isOlder == older > 0
     IN
     /\ out' = Tail(out)
     /\ older' = IF isOlder THEN older - 1 ELSE 0
     /\ CASE ev[1] = "enqueued" ->
               UNCHANGED <<open, deliv, executing, stopping, foreign, undel, heldEcho,
                           pending, closed, bus, pipe, doneOwed, stopCancels, woken>>
          [] ev[1] = "started" /\ isOlder ->
               \* the read-back said whether a run is going
               UNCHANGED <<open, deliv, executing, stopping, foreign, undel, heldEcho,
                           pending, closed, bus, pipe, doneOwed, stopCancels, woken>>
          [] ev[1] = "started" ->
               /\ executing' = TRUE /\ foreign' = (foreign \/ ~open)
               \* a stop asked as this run started, while an earlier one was ending
               /\ pipe' = IF stopping THEN Append(pipe, <<"interrupt">>) ELSE pipe
               /\ UNCHANGED <<open, deliv, stopping, undel, heldEcho, pending, closed, bus, doneOwed,
                              stopCancels, woken>>
          [] ev[1] = "delivered" ->
               \* the read-back settled every message that left the inbox before it; the code's
               \* fresh id per send makes an older one match nothing
               /\ IF ev[2] \in undel /\ ~isOlder THEN Deliver(ev[2])
                  ELSE UNCHANGED <<undel, heldEcho, bus, open, deliv, doneOwed, stopCancels>>
               /\ UNCHANGED <<executing, stopping, foreign, pending, closed, pipe, woken>>
          [] ev[1] = "text" ->
               \* text the read-back replayed is not shown again
               /\ bus' = IF isOlder THEN bus ELSE Append(bus, <<"text">>)
               /\ UNCHANGED <<open, deliv, executing, stopping, foreign, undel, heldEcho,
                              pending, closed, pipe, doneOwed, stopCancels, woken>>
          [] ev[1] = "cancelled" ->
               /\ IF ev[2] \in undel /\ ~isOlder THEN Cancel(ev[2])
                  ELSE UNCHANGED <<undel, heldEcho, bus, open, stopping, doneOwed, stopCancels, foreign>>
               /\ UNCHANGED <<deliv, executing, pending, closed, pipe, woken>>
          [] ev[1] = "ended" ->
               \* the read-back said whether a run is going
               /\ IF isOlder
                    THEN UNCHANGED <<open, deliv, executing, stopping, foreign, pending, closed, bus, pipe,
                                     doneOwed, stopCancels, woken>>
                    ELSE EndRun(bus, undel)
               /\ UNCHANGED <<undel, heldEcho>>
          [] ev[1] = "ask" ->
               \* once closed, an ask id never opens again; one queued before a read-back is
               \* covered by it, which passed on every ask OpenCode still held
               /\ IF ev[2] \in pending \cup closed \/ isOlder
                    THEN UNCHANGED <<pending, bus>>
                    ELSE pending' = pending \cup {ev[2]} /\ bus' = Append(bus, <<"ask", ev[2]>>)
               /\ UNCHANGED <<open, deliv, executing, stopping, foreign, undel, heldEcho,
                              closed, pipe, doneOwed, stopCancels, woken>>
          [] ev[1] = "replied" ->
               /\ IF ev[2] \in pending
                    THEN /\ pending' = pending \ {ev[2]} /\ closed' = closed \cup {ev[2]}
                         /\ bus' = Append(bus, <<"resolved", ev[2]>>)
                    ELSE UNCHANGED <<pending, closed, bus>>
               /\ UNCHANGED <<open, deliv, executing, stopping, foreign, undel, heldEcho, pipe,
                              doneOwed, stopCancels, woken>>
  /\ UNCHANGED <<ocRest, posted, stopTargets, svcVars, viewVars, ruleVars>>

\* The filter task reads server.connected (or Lagged) and reads back what OpenCode holds now: its
\* inbox (a message gone from it was delivered if a user message stands for it, else cancelled:
\* the hand-backs and echoes go out first), whether a run is going (if not, the run ended as the
\* newest idle message says), and its open asks (one it does not hold is passed on, one it holds
\* that OpenCode dropped closes). One step: every read follows the reconnect, so the events queued
\* behind it are older than what it read.
Resync ==
  /\ out /= <<>> /\ Head(out) = <<"connected">>
  /\ older' = Len(out) - 1
  /\ out' = Tail(out)
  \* a message whose prompt OpenCode has not answered yet is not in its inbox either
  /\ LET gone == (undel \cap posted) \ Range(inbox)
         came == gone \cap ocDelivered
         left == gone \ ocDelivered
         undel1 == undel \ gone
         held1 == heldEcho \ gone
         cancels1 == stopCancels \ gone
         owed1 == doneOwed /\ cancels1 \cap undel1 /= {}
         open1 == open \/ (came /= {} /\ ~doneOwed)
         deliv1 == deliv \/ (came /= {} /\ ~doneOwed)
         \* an echoed message the gap hid the cancel of ends a turn nothing of reached a run
         lastLeft == open1 /\ ~deliv1 /\ (left \ heldEcho) /= {} /\ (undel1 \ held1) = {}
         owedLeft == lastLeft /\ cancels1 \cap undel1 /= {}
         open2 == open1 /\ ~lastLeft
         stopping2 == stopping /\ ~lastLeft
         fresh == cOpen \ (pending \cup closed)
         stale == pending \ cOpen
         pre == bus \o SetToSeq({<<"echo", m>> : m \in came \cap heldEcho})
                    \o SetToSeq({<<"declined", m>> : m \in left \cap heldEcho})
                    \o (IF (lastLeft /\ ~owedLeft) \/ (doneOwed /\ ~owed1) THEN <<<<"turnDone">>>> ELSE <<>>)
                    \o Closes(stale)
                    \o SetToSeq({<<"ask", r>> : r \in fresh})
         pend1 == (pending \ stale) \cup fresh
         ours == (open2 /\ deliv1) \/ stopping2
         done == ours \/ (~open2 /\ foreign)
         again == ((undel1 \cap posted) \ stopTargets) \ woken
         giveUp == ((undel1 \cap posted) \ stopTargets) \cap woken
         cancels2 == cancels1 \cup (giveUp \cap held1)
         owe == (done \/ ~open2) /\ cancels2 \cap undel1 /= {}
         endsNow == ~running /\ (executing \/ open2 \/ stopping2)
     IN
     /\ undel' = undel1 /\ heldEcho' = held1
     /\ IF endsNow
          THEN /\ executing' = FALSE /\ foreign' = FALSE
               /\ closed' = closed \cup pending \cup pend1 /\ pending' = {}
               /\ bus' = pre \o Closes(pend1) \o (IF done /\ ~owe THEN <<<<"turnDone">>>> ELSE <<>>)
               /\ doneOwed' = (owed1 \/ owedLeft \/ owe)
               /\ open' = (open2 /\ ~ours) /\ deliv' = (deliv1 /\ ~ours)
               /\ stopping' = (stopping2 /\ ~ours)
               /\ pipe' = pipe \o SetToSeq({<<"wake", m>> : m \in again}) \o SetToSeq({<<"cancel", m>> : m \in giveUp})
               /\ woken' = woken \cup again /\ stopCancels' = cancels2
          ELSE /\ executing' = (executing \/ running)
               /\ closed' = closed \cup stale /\ pending' = pend1 /\ bus' = pre
               /\ doneOwed' = (owed1 \/ owedLeft \/ (~running /\ ~open2 /\ cancels2 \cap undel1 /= {}))
               /\ open' = open2 /\ deliv' = deliv1 /\ stopping' = stopping2
               /\ foreign' = (foreign \/ (lastLeft /\ (executing \/ running)) \/ (running /\ ~open2))
               \* no run going: what OpenCode holds of this driver's is parked, whatever ended in the gap
               /\ IF running
                    THEN UNCHANGED <<pipe, woken>> /\ stopCancels' = cancels1
                    ELSE /\ pipe' = pipe \o SetToSeq({<<"wake", m>> : m \in again})
                                       \o SetToSeq({<<"cancel", m>> : m \in giveUp})
                         /\ woken' = woken \cup again /\ stopCancels' = cancels2
  /\ UNCHANGED <<ocRest, posted, stopTargets, svcVars, viewVars, ruleVars>>

(* ---- spawn_relay + ActorState::handle_event (author_for's next_turn), the desktop ---- *)

\* On Exited the relay drops the driver and ends; the desktop clears its working state and its
\* cards and hands back its own unechoed messages, and the relay reports an agent's
\* (held::report_undelivered).
RelayExited ==
  /\ bus /= <<>> /\ Head(bus) = <<"exited">> /\ resendDue = {}
  /\ bus' = <<>>
  /\ live' = FALSE /\ vLive' = FALSE
  /\ inTurn' = FALSE /\ nextTurn' = FALSE /\ streaming' = FALSE /\ vOpen' = {}
  /\ chip' = [m \in Composer |-> IF chip[m] = "unechoed"
                                   THEN IF stopped THEN "returned" ELSE "queued"
                                   ELSE chip[m]]
  /\ failed' = failed \cup held /\ held' = {} /\ dropped' = {}
  /\ UNCHANGED <<ocVars, open, deliv, executing, stopping, foreign, undel, heldEcho,
                 pending, closed, pipe, older, doneOwed, stopCancels, woken, posted, stopTargets, stored, accepted, resendDue, clicked, sends, stopped,
                 ruleVars>>

Relay ==
  /\ bus /= <<>> /\ Head(bus) /= <<"exited">> /\ resendDue = {}
  /\ LET ev == Head(bus)
         back == IF stopped THEN "returned" ELSE "queued"
     IN
     /\ bus' = Tail(bus)
     /\ inTurn' = CASE ev[1] = "echo" -> TRUE
                   [] ev[1] = "text" /\ nextTurn -> TRUE
                   [] ev[1] = "turnDone" -> FALSE
                   [] OTHER -> inTurn
     /\ nextTurn' = CASE ev[1] = "echo" /\ inTurn -> TRUE
                     [] ev[1] = "text" /\ ~inTurn -> FALSE
                     [] OTHER -> nextTurn
     /\ stored' = IF ev[1] \in {"echo", "ask"} THEN stored \cup {ev[2]} ELSE stored
     /\ streaming' = CASE ev[1] \in {"echo", "ask", "text"} -> TRUE
                      [] ev[1] = "turnDone" -> FALSE
                      [] OTHER -> streaming
     \* OpenCode's permission actions (read, shell) never match a tool's (Read, Bash), so a tool
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
  /\ UNCHANGED <<ocVars, open, deliv, executing, stopping, foreign, undel, heldEcho,
                 pending, closed, pipe, older, doneOwed, stopCancels, woken, posted, stopTargets, live, accepted, failed, vLive, clicked, sends, stopped,
                 ruleVars>>

\* held::send_again: OpenCode's send never fails.
RelayResend ==
  \E m \in resendDue :
    /\ resendDue' = resendDue \ {m}
    /\ SendServer(m)
    /\ UNCHANGED <<alive, inbox, running, parkedInbox, runMsgs, cOpen, asked, out, drops, downs, wakes, fails,
                   delivered, lastEnd, executing, foreign, pending, closed, older, doneOwed, stopCancels,
                   live, inTurn, nextTurn, stored, failed, held, dropped, viewVars, ruleVars>>

ClientReload ==
  /\ vLive /\ live
  /\ vOpen' = pending \cap stored
  /\ UNCHANGED <<ocVars, drvVars, svcVars, vLive, streaming, clicked, sends, chip, stopped, ruleVars>>

(* ---- the rules ---- *)

Quiet ==
  /\ out = <<>> /\ bus = <<>> /\ pipe = <<>> /\ resendDue = {} /\ (live => ~doneOwed)
  /\ alive => (~running /\ (inbox = <<>> \/ parkedInbox))
  /\ alive \/ ~live

Done == Quiet /\ UNCHANGED vars

Next ==
  \/ \E m \in Composer : ClientSend(m)
  \/ \E m \in Agent : AgentSend(m)
  \/ \E r \in Asks : ClientAnswer(r) \/ OcAsk(r) \/ OcElsewhereReply(r)
  \/ Interrupt
  \/ OcPost \/ OcWake \/ OcDeliver
  \/ \E outcome \in {"succeeded", "failed"} : OcFinish(outcome)
  \/ OcDown \/ ServerExited \/ SseDrop
  \/ DriverRead \/ Resync
  \/ Relay \/ RelayExited \/ RelayResend \/ ClientReload \/ Done

Spec == Init /\ [][Next]_vars

\* A parked inbox runs again on a new prompt or a wake, which the driver sends on reading the end
\* of a run that never read its turn; a run going reads it too.
Unparks == out /= <<>> \/ \E i \in 1..Len(pipe) : pipe[i][1] \in {"prompt", "wake"}

OnItsWay(m) ==
  \/ InSeq(<<"prompt", m>>, pipe) \/ InSeq(<<"cancel", m>>, pipe)
  \/ alive /\ InSeq(m, inbox) /\ (running \/ ~parkedInbox \/ Unparks)
  \/ InSeq(<<"echo", m>>, bus) \/ InSeq(<<"declined", m>>, bus)
  \/ m \in resendDue

\* No relaunch: a message is kept once OpenCode reads it, or reported failed when the server's
\* death drops the driver first. A stored echo counts only while OpenCode does not hold the
\* message parked unread. A dead server's inbox stays for a resume to read, which the model
\* leaves out.
Kept(m) ==
  \/ m \notin accepted
  \/ delivered[m] > 0
  \/ OnItsWay(m)
  \/ m \in failed
  \/ m \in stored /\ ~(alive /\ InSeq(m, inbox) /\ parkedInbox /\ ~running /\ ~Unparks)
  \/ m \in Composer /\ chip[m] /= "none"
  \/ m \in held /\ ~Quiet
  \/ ~alive /\ InSeq(m, inbox)

NoLostUserMessage == \A m \in Composer : Kept(m)
NoLostAgentMessage == \A m \in Agent : Kept(m)

NoDoubleSend == \A m \in Msgs : delivered[m] <= 1

TurnProgress == Quiet => ~inTurn /\ ~streaming

AskResolution == Quiet => vOpen \subseteq cOpen

AskVisible == (out = <<>> /\ bus = <<>> /\ pipe = <<>>) => cOpen \subseteq vOpen

InterruptSafe ==
  (Quiet /\ interrupts > 0 /\ alive) => ~inTurn /\ ~streaming /\ vOpen = {}

\* A stop OpenCode takes while a message sent before it waits to run keeps that message's run from
\* finishing.
InterruptHonored == ~droppedStop

TypeOK ==
  /\ alive \in BOOLEAN /\ running \in BOOLEAN /\ parkedInbox \in BOOLEAN
  /\ Range(inbox) \subseteq Msgs /\ runMsgs \subseteq Msgs
  /\ cOpen \subseteq Asks /\ asked \subseteq Asks /\ pending \subseteq Asks /\ closed \subseteq Asks
  /\ vOpen \subseteq Asks /\ stored \subseteq Msgs \cup Asks
  /\ accepted \subseteq Msgs /\ failed \subseteq Msgs
  /\ undel \subseteq Msgs /\ heldEcho \subseteq undel
  /\ held \subseteq Agent /\ chip \in [Composer -> {"none", "unechoed", "queued", "returned"}]
  /\ ocDelivered \subseteq Msgs /\ lastEnd \in {"succeeded", "failed", "interrupted"}
  /\ open \in BOOLEAN /\ deliv \in BOOLEAN /\ executing \in BOOLEAN /\ stopping \in BOOLEAN
  /\ doneOwed \in BOOLEAN /\ stopCancels \subseteq Msgs /\ woken \subseteq Msgs /\ posted \subseteq Msgs /\ stopTargets \subseteq Msgs /\ fails \in Nat
=============================================================================
