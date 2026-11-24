---------------------------- MODULE SessionSync ----------------------------
(* One desktop following one session's event log: the actor storing events under
   consecutive seqs and fanning them out (handle_event, LiveAttachment::deliver with
   its one free slot for Lagged), forward_session's re-attach after a lag
   (attach_after + send_replay), and sessionStore's dispatch/route (apply, hold,
   flush, skip), pullNow over list_turns pages, mergeEntries and trimHeld; and the
   actor's lifetime: the harness ending and resuming, the desktop closing and
   reopening the session (attach_from its cursor), and an idle actor retiring.
   Rows open and close turns: the actor's in_turn follows them, and the view's working
   state follows the rows it folds; a reopen seeds it from the actor's in_turn, which
   resume_session returns as turn_started_at.
   SKILL.md maps every action to the code. *)
EXTENDS Integers, Sequences

CONSTANTS N,          \* events the actor stores
          Cap,        \* an attachment's queue, less the slot kept for Lagged
          PageLimit,  \* rows per list_turns page
          MaxLags,
          MaxReopens

Seqs == 0..(N - 1)

VARIABLES
  next,       \* the actor's seq: rows 0..next-1 are stored
  author,     \* author[s]: "own" (this desktop) or "other" (another device)
  att,        \* the live attachment's queue: <<"item", s>> or <<"lagged", lastPersisted>>
  lastQueued, \* LiveAttachment.last_persisted_seq
  detached,
  wire,       \* frames on their way to the desktop, in order
  applied,    \* entry.appliedSeq
  appliedSet, \* every seq the view folded in (to catch a skip or a repeat)
  held,       \* the held queue: other devices' items waiting on a pull
  pull,       \* "none", or the page a list_turns read returned: <<seqs..., remaining>>
  lags,
  skipped,    \* the view applied a seq with an earlier one missing
  harness,    \* a live harness's relay holds the actor's ingress
  actor,      \* the session's actor runs
  viewing,    \* the desktop has the session open (an attachment, or one on its way)
  reopens,
  lost,       \* the relay sent an event to an actor that had retired
  kind,       \* kind[s]: "open" (a user message), "close" (turnDone or an exit) or "item"
  inTurn,     \* the actor's in_turn
  vWorking    \* the view shows the turn working (streaming)

vars == <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
          held, pull, lags, skipped, harness, actor, viewing, reopens, lost,
          kind, inTurn, vWorking>>
lifeVars == <<harness, actor, viewing, reopens, lost>>
turnVars == <<kind, inTurn, vWorking>>

Init ==
  /\ next = 0 /\ author = [s \in Seqs |-> "own"]
  /\ att = <<>> /\ lastQueued = -1 /\ detached = FALSE
  /\ wire = <<>> /\ applied = -1 /\ appliedSet = {}
  /\ held = <<>> /\ pull = "none" /\ lags = 0 /\ skipped = FALSE
  /\ harness = TRUE /\ actor = TRUE /\ viewing = TRUE /\ reopens = 0 /\ lost = FALSE
  /\ kind = [s \in Seqs |-> "item"] /\ inTurn = FALSE /\ vWorking = FALSE

\* The view's working state after folding `rows` in order: the last open or close decides.
RECURSIVE WorkingAfter(_, _)
WorkingAfter(w, rows) ==
  IF rows = <<>> THEN w
  ELSE WorkingAfter(CASE kind[Head(rows)] = "open" -> TRUE
                      [] kind[Head(rows)] = "close" -> FALSE
                      [] OTHER -> w, Tail(rows))

\* handle_event: store at seq `next`, then deliver to the attachment.
Store ==
  /\ next < N /\ harness /\ actor
  /\ \E who \in {"own", "other"} : author' = [author EXCEPT ![next] = who]
  /\ \E k \in {"open", "close", "item"} :
       /\ kind' = [kind EXCEPT ![next] = k]
       /\ inTurn' = CASE k = "open" -> TRUE [] k = "close" -> FALSE [] OTHER -> inTurn
  /\ next' = next + 1
  /\ IF detached
       THEN UNCHANGED <<att, lastQueued, detached, lags>>
     ELSE IF Len(att) < Cap
       THEN att' = Append(att, <<"item", next>>) /\ lastQueued' = next
            /\ UNCHANGED <<detached, lags>>
     ELSE IF lags < MaxLags
       THEN \* mark_lagged: the free slot takes Lagged{last_persisted_seq}
            att' = Append(att, <<"lagged", lastQueued>>) /\ detached' = TRUE
            /\ lags' = lags + 1 /\ UNCHANGED lastQueued
       ELSE FALSE
  /\ UNCHANGED <<wire, applied, appliedSet, held, pull, skipped, lifeVars, vWorking>>

RECURSIVE Range(_, _)
Range(lo, hi) == IF lo > hi THEN <<>> ELSE <<lo>> \o Range(lo + 1, hi)

\* forward_session: an item goes on the wire; Lagged re-attaches in one actor turn.
Forward ==
  /\ att /= <<>>
  /\ LET f == Head(att) IN
     IF f[1] = "item"
       THEN /\ wire' = Append(wire, f[2]) /\ att' = Tail(att)
            /\ UNCHANGED <<lastQueued, detached>>
       ELSE \* attach_after(epoch, last_persisted_seq): the log after it, then a fresh attachment
            LET replay == Range(f[2] + 1, next - 1) IN
            /\ wire' = wire \o replay
            /\ att' = <<>> /\ detached' = FALSE
            /\ lastQueued' = IF replay = <<>> THEN f[2] ELSE next - 1
  /\ UNCHANGED <<next, author, applied, appliedSet, held, pull, lags, skipped, lifeVars, turnVars>>

\* dispatch -> route: skip what is already folded in, hold another device's item, and on
\* this desktop's own item apply the held queue first (flush).
Receive ==
  /\ wire /= <<>>
  /\ LET s == Head(wire) IN
     /\ wire' = Tail(wire)
     /\ CASE s <= applied ->
               UNCHANGED <<applied, appliedSet, held, skipped, vWorking>>
          [] held /= <<>> /\ author[s] = "own" ->
               LET run == held \o <<s>> IN
               /\ skipped' = (skipped \/ \E i \in 1..Len(run) :
                                 run[i] > (IF i = 1 THEN applied ELSE run[i - 1]) + 1)
               /\ applied' = s /\ appliedSet' = appliedSet \cup {run[i] : i \in 1..Len(run)}
               /\ held' = <<>> /\ vWorking' = WorkingAfter(vWorking, run)
          [] held /= <<>> \/ author[s] = "other" ->
               held' = Append(held, s) /\ UNCHANGED <<applied, appliedSet, skipped, vWorking>>
          [] OTHER ->
               /\ skipped' = (skipped \/ s > applied + 1)
               /\ applied' = s /\ appliedSet' = appliedSet \cup {s}
               /\ held' = held /\ vWorking' = WorkingAfter(vWorking, <<s>>)
  /\ UNCHANGED <<next, author, att, lastQueued, detached, pull, lags, lifeVars, kind, inTurn>>

\* pullNow: list_turns after the view's cursor, one page; the read is one actor turn.
PullRead ==
  /\ viewing /\ pull = "none" /\ (held /= <<>> \/ applied < next - 1)
  /\ LET hi == IF applied + PageLimit < next - 1 THEN applied + PageLimit ELSE next - 1 IN
     pull' = <<Range(applied + 1, hi), next - 1 - hi>>
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
                 held, lags, skipped, lifeVars, turnVars>>

\* mergeEntries keeps rows past appliedSeq; trimHeld drops held items it covered; a page
\* with rows remaining pulls again.
PullApply ==
  /\ pull /= "none"
  /\ LET rows == pull[1]
         fresh == SelectSeq(rows, LAMBDA s : s > applied)
         top == IF fresh = <<>> THEN applied ELSE fresh[Len(fresh)]
     IN
     /\ skipped' = (skipped \/ (fresh /= <<>> /\ fresh[1] > applied + 1))
     /\ applied' = top
     /\ appliedSet' = appliedSet \cup {fresh[i] : i \in 1..Len(fresh)}
     /\ held' = SelectSeq(held, LAMBDA s : s > top)
     /\ pull' = "none"
     /\ vWorking' = WorkingAfter(vWorking, fresh)
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, lags, lifeVars, kind, inTurn>>

\* The harness exits: its relay ends and drops the actor's ingress.
\* Its exit row closes any open turn, so the harness ends outside one.
End ==
  /\ harness /\ ~inTurn /\ harness' = FALSE
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
                 held, pull, lags, skipped, actor, viewing, reopens, lost, turnVars>>

\* launch_driver: the relay takes `ingress()`, which spawns an actor if none runs.
Resume ==
  /\ ~harness /\ next < N
  /\ harness' = TRUE /\ actor' = TRUE
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
                 held, pull, lags, skipped, viewing, reopens, lost, turnVars>>

\* The tab closes: its stream and any page in flight are dropped; the entry keeps what it folded.
Close ==
  /\ viewing /\ viewing' = FALSE
  /\ att' = <<>> /\ detached' = TRUE /\ wire' = <<>> /\ pull' = "none"
  /\ UNCHANGED <<next, author, lastQueued, applied, appliedSet, held, lags, skipped,
                 harness, actor, reopens, lost, turnVars>>

\* An idle actor retires (Control::Retire): no attachment, and no relay holding its ingress.
Retire ==
  /\ actor /\ ~viewing /\ ~harness
  /\ actor' = FALSE
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
                 held, pull, lags, skipped, harness, viewing, reopens, lost, turnVars>>

\* A relay still feeding a retired actor: its send fails and the event is never stored.
Lose ==
  /\ harness /\ ~actor /\ next < N /\ ~lost
  /\ lost' = TRUE
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
                 held, pull, lags, skipped, harness, actor, viewing, reopens, turnVars>>

\* The tab opens again: attach_from(cursor) on the actor, spawned again if it retired, replays the
\* log after the view's cursor and registers a fresh attachment in the same actor turn. Its reply
\* says whether a turn is open there (turn_started_at), and the view takes that before the replay.
Reopen ==
  /\ ~viewing /\ reopens < MaxReopens
  /\ viewing' = TRUE /\ actor' = TRUE /\ reopens' = reopens + 1
  /\ wire' = Range(applied + 1, next - 1)
  /\ att' = <<>> /\ detached' = FALSE /\ lastQueued' = next - 1
  /\ vWorking' = inTurn
  /\ UNCHANGED <<next, author, applied, appliedSet, held, pull, lags, skipped, harness, lost,
                 kind, inTurn>>

Quiet == next = N /\ att = <<>> /\ wire = <<>> /\ pull = "none"

Done == Quiet /\ UNCHANGED vars

Next == Store \/ Forward \/ Receive \/ PullRead \/ PullApply
        \/ End \/ Resume \/ Close \/ Retire \/ Lose \/ Reopen \/ Done

Spec == Init /\ [][Next]_vars

\* The view never folds a seq in with an earlier one missing.
NoSkippedSeq == ~skipped

\* Once the log stops and every frame and pull has landed, an open view holds every stored seq,
\* or it still holds another device's items (the "behind" strip, until you pull).
ClientsConverge ==
  (Quiet /\ viewing) => (appliedSet = Seqs \/ held /= <<>>)

\* Once everything has landed, an open view with nothing held shows a turn working exactly
\* when the actor has one open, including after a reopen whose replay carried no row.
ReopenShowsTurn ==
  (Quiet /\ viewing /\ held = <<>>) => vWorking = inTurn

\* No event a live harness emits is lost to a retired actor.
NoLostEvent == ~lost

TypeOK ==
  /\ next \in 0..N /\ applied \in -1..(N - 1) /\ appliedSet \subseteq Seqs
  /\ detached \in BOOLEAN /\ skipped \in BOOLEAN
  /\ harness \in BOOLEAN /\ actor \in BOOLEAN /\ viewing \in BOOLEAN /\ lost \in BOOLEAN
  /\ inTurn \in BOOLEAN /\ vWorking \in BOOLEAN
=============================================================================
