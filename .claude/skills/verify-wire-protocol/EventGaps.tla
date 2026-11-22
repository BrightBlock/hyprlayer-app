------------------------------ MODULE EventGaps ------------------------------
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS MaxSeq, QueueFrames, MaxLag

Seqs == 0..(MaxSeq - 1)

VARIABLES
  fc,
  next,
  q,
  wire,
  cnext,
  missed,
  missedOf,
  seen,
  lostAtServer

vars == <<fc, next, q, wire, cnext, missed, missedOf, seen, lostAtServer>>

Frame(s) == [k |-> "frame", first |-> s, count |-> 1]
Gap(s, n) == [k |-> "missed", first |-> s, count |-> n]

Range(lo, hi) == {s \in Seqs: lo <= s /\ s < hi}
Bump(f, set) == [s \in Seqs |-> IF s \in set THEN f[s] + 1 ELSE f[s]]
Sum(f) == LET F[i \in 0..MaxSeq] == IF i = 0 THEN 0 ELSE F[i - 1] + f[i - 1] IN F[MaxSeq]
Max(a, b) == IF a > b THEN a ELSE b
SatSub(a, b) == IF a > b THEN a - b ELSE 0

GapOpen == q /= <<>> /\ q[Len(q)].k = "missed"
QueuedFrames == Len(SelectSeq(q, LAMBDA m: m.k = "frame"))

Init ==
  /\ fc \in BOOLEAN
  /\ next = 0
  /\ q = <<>>
  /\ wire = <<>>
  /\ cnext = 0
  /\ missed = 0
  /\ missedOf = [s \in Seqs |-> 0]
  /\ seen = [s \in Seqs |-> 0]
  /\ lostAtServer = {}

NoteMissed(n) ==
  /\ q' = IF GapOpen
            THEN [q EXCEPT ![Len(q)].count = @ + n]
            ELSE Append(q, Gap(next, n))
  /\ next' = next + n
  /\ lostAtServer' = lostAtServer \cup Range(next, next + n)

SrvQueue ==
  /\ next < MaxSeq
  /\ ~GapOpen
  /\ ~(QueuedFrames > 0 /\ QueuedFrames >= QueueFrames)
  /\ q' = Append(q, Frame(next))
  /\ next' = next + 1
  /\ UNCHANGED <<fc, wire, cnext, missed, missedOf, seen, lostAtServer>>

SrvDrop ==
  /\ next < MaxSeq
  /\ NoteMissed(1)
  /\ UNCHANGED <<fc, wire, cnext, missed, missedOf, seen>>

SrvLag(n) ==
  /\ next + n <= MaxSeq
  /\ NoteMissed(n)
  /\ UNCHANGED <<fc, wire, cnext, missed, missedOf, seen>>

SrvWrite ==
  /\ q /= <<>>
  /\ q' = Tail(q)
  /\ wire' = IF Head(q).k = "frame" \/ fc THEN Append(wire, Head(q)) ELSE wire
  /\ UNCHANGED <<fc, next, cnext, missed, missedOf, seen, lostAtServer>>

Arrive(how) ==
  /\ wire' = Tail(wire)
  /\ LET seq == Head(wire).first
         lost == SatSub(seq, cnext)
         gap == Range(cnext, seq)
     IN /\ cnext' = Max(cnext, seq + 1)
        /\ CASE how = "kept" ->
                  /\ missed' = missed + lost
                  /\ missedOf' = Bump(missedOf, gap)
                  /\ seen' = Bump(seen, {seq})
             [] how = "full" ->
                  /\ missed' = missed + lost + 1
                  /\ missedOf' = Bump(missedOf, gap \cup {seq})
                  /\ UNCHANGED seen
             [] how = "undecodable" ->
                  /\ seen' = Bump(seen, {seq})
                  /\ UNCHANGED <<missed, missedOf>>
  /\ UNCHANGED <<fc, next, q, lostAtServer>>

EventNext == wire /= <<>> /\ Head(wire).k = "frame"

CliEventKept == EventNext /\ Arrive("kept")
CliEventFull == EventNext /\ Arrive("full")
CliEventSkipped == EventNext /\ Arrive("undecodable")

CliMissed ==
  /\ wire /= <<>>
  /\ Head(wire).k = "missed"
  /\ wire' = Tail(wire)
  /\ LET end == Head(wire).first + Head(wire).count
         lost == SatSub(end, cnext)
     IN /\ cnext' = Max(cnext, end)
        /\ missed' = missed + lost
        /\ missedOf' = Bump(missedOf, Range(cnext, end))
  /\ UNCHANGED <<fc, next, q, seen, lostAtServer>>

Done ==
  /\ next = MaxSeq
  /\ q = <<>> /\ wire = <<>>
  /\ UNCHANGED vars

Next ==
  \/ SrvQueue \/ SrvDrop \/ (\E n \in 1..MaxLag: SrvLag(n)) \/ SrvWrite
  \/ CliEventKept \/ CliEventFull \/ CliEventSkipped
  \/ CliMissed
  \/ Done

Spec == Init /\ [][Next]_vars

TypeOK ==
  /\ fc \in BOOLEAN
  /\ next \in 0..MaxSeq
  /\ cnext \in 0..MaxSeq
  /\ missed \in Nat
  /\ missedOf \in [Seqs -> Nat]
  /\ seen \in [Seqs -> Nat]
  /\ lostAtServer \subseteq Seqs

Settled == q = <<>> /\ wire = <<>>

CountedOnce ==
  /\ missed = Sum(missedOf)
  /\ \A s \in Seqs: missedOf[s] + seen[s] <= 1
  /\ \A s \in lostAtServer: seen[s] = 0
  /\ \A s \in Range(0, cnext): missedOf[s] + seen[s] = 1

GapsCountedOnce == fc => (CountedOnce /\ (Settled => cnext = next))

GapsCountedOnceWithoutFlowControl == ~fc => CountedOnce

=============================================================================
