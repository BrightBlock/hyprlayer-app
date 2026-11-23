---------------------------- MODULE SessionSync ----------------------------
(* One desktop following one session's event log: the actor storing events under
   consecutive seqs and fanning them out (handle_event, LiveAttachment::deliver with
   its one free slot for Lagged), forward_session's re-attach after a lag
   (attach_after + send_replay), and sessionStore's dispatch/route (apply, hold,
   flush, skip), pullNow over list_turns pages, mergeEntries and trimHeld.
   SKILL.md maps every action to the code. *)
EXTENDS Integers, Sequences

CONSTANTS N,          \* events the actor stores
          Cap,        \* an attachment's queue, less the slot kept for Lagged
          PageLimit,  \* rows per list_turns page
          MaxLags

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
  skipped     \* the view applied a seq with an earlier one missing

vars == <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
          held, pull, lags, skipped>>

Init ==
  /\ next = 0 /\ author = [s \in Seqs |-> "own"]
  /\ att = <<>> /\ lastQueued = -1 /\ detached = FALSE
  /\ wire = <<>> /\ applied = -1 /\ appliedSet = {}
  /\ held = <<>> /\ pull = "none" /\ lags = 0 /\ skipped = FALSE

\* handle_event: store at seq `next`, then deliver to the attachment.
Store ==
  /\ next < N
  /\ \E who \in {"own", "other"} : author' = [author EXCEPT ![next] = who]
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
  /\ UNCHANGED <<wire, applied, appliedSet, held, pull, skipped>>

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
  /\ UNCHANGED <<next, author, applied, appliedSet, held, pull, lags, skipped>>

\* dispatch -> route: skip what is already folded in, hold another device's item, and on
\* this desktop's own item apply the held queue first (flush).
Receive ==
  /\ wire /= <<>>
  /\ LET s == Head(wire) IN
     /\ wire' = Tail(wire)
     /\ CASE s <= applied ->
               UNCHANGED <<applied, appliedSet, held, skipped>>
          [] held /= <<>> /\ author[s] = "own" ->
               LET run == held \o <<s>> IN
               /\ skipped' = (skipped \/ \E i \in 1..Len(run) :
                                 run[i] > (IF i = 1 THEN applied ELSE run[i - 1]) + 1)
               /\ applied' = s /\ appliedSet' = appliedSet \cup {run[i] : i \in 1..Len(run)}
               /\ held' = <<>>
          [] held /= <<>> \/ author[s] = "other" ->
               held' = Append(held, s) /\ UNCHANGED <<applied, appliedSet, skipped>>
          [] OTHER ->
               /\ skipped' = (skipped \/ s > applied + 1)
               /\ applied' = s /\ appliedSet' = appliedSet \cup {s}
               /\ held' = held
  /\ UNCHANGED <<next, author, att, lastQueued, detached, pull, lags>>

\* pullNow: list_turns after the view's cursor, one page; the read is one actor turn.
PullRead ==
  /\ pull = "none" /\ (held /= <<>> \/ applied < next - 1)
  /\ LET hi == IF applied + PageLimit < next - 1 THEN applied + PageLimit ELSE next - 1 IN
     pull' = <<Range(applied + 1, hi), next - 1 - hi>>
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, applied, appliedSet,
                 held, lags, skipped>>

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
  /\ UNCHANGED <<next, author, att, lastQueued, detached, wire, lags>>

Quiet == next = N /\ att = <<>> /\ wire = <<>> /\ pull = "none"

Done == Quiet /\ UNCHANGED vars

Next == Store \/ Forward \/ Receive \/ PullRead \/ PullApply \/ Done

Spec == Init /\ [][Next]_vars

\* The view never folds a seq in with an earlier one missing.
NoSkippedSeq == ~skipped

\* Once the log stops and every frame and pull has landed, the view holds every stored seq,
\* or it still holds another device's items (the "behind" strip, until you pull).
ClientsConverge ==
  Quiet => (appliedSet = Seqs \/ held /= <<>>)

TypeOK ==
  /\ next \in 0..N /\ applied \in -1..(N - 1) /\ appliedSet \subseteq Seqs
  /\ detached \in BOOLEAN /\ skipped \in BOOLEAN
=============================================================================
