---------------------------- MODULE StreamCredit ----------------------------
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS Window, Announced, Sizes, MaxFrames

Batch == IF Announced \div 4 > 1 THEN Announced \div 4 ELSE 1

Charge(n) == n
Counted(n) == n

VARIABLES
  phase,
  entry,
  task,
  credit,
  sent,
  s2c,
  c2s,
  inbox,
  rx,
  queue,
  owed,
  cancelRead

vars == <<phase, entry, task, credit, sent, s2c, c2s, inbox, rx, queue, owed, cancelRead>>

Resp == [k |-> "resp", n |-> 0]
Data(n) == [k |-> "data", n |-> n]
CreditMsg(n) == [k |-> "credit", n |-> n]
CancelMsg == [k |-> "cancel", n |-> 0]

SumOf(s) == LET F[i \in 0..Len(s)] == IF i = 0 THEN 0 ELSE F[i - 1] + s[i].n IN F[Len(s)]
Total(s, kind) == SumOf(SelectSeq(s, LAMBDA m: m.k = kind))
QueueBytes == LET F[i \in 0..Len(queue)] == IF i = 0 THEN 0 ELSE F[i - 1] + queue[i] IN F[Len(queue)]
DataInFlight == \E i \in 1..Len(s2c): s2c[i].k = "data"

Init ==
  /\ phase = "prepare"
  /\ entry = "none"
  /\ task = "unborn"
  /\ credit = 0
  /\ sent = 0
  /\ s2c = <<>>
  /\ c2s = <<>>
  /\ inbox = "none"
  /\ rx = "none"
  /\ queue = <<>>
  /\ owed = 0
  /\ cancelRead = FALSE

SrvRespond ==
  /\ phase = "prepare"
  /\ entry' = "reserved"
  /\ s2c' = Append(s2c, Resp)
  /\ phase' = "spawning"
  /\ UNCHANGED <<task, credit, sent, c2s, inbox, rx, queue, owed, cancelRead>>

SrvSpawn ==
  /\ phase = "spawning"
  /\ phase' = "finished"
  /\ IF entry = "reservedCancelled"
       THEN /\ entry' = "none"
            /\ UNCHANGED <<task, credit>>
       ELSE /\ entry' = "open"
            /\ task' = "running"
            /\ credit' = Window
  /\ UNCHANGED <<sent, s2c, c2s, inbox, rx, queue, owed, cancelRead>>

SrvReady ==
  /\ task = "running"
  /\ credit >= 1
  /\ sent < MaxFrames
  /\ task' = "pushing"
  /\ UNCHANGED <<phase, entry, credit, sent, s2c, c2s, inbox, rx, queue, owed, cancelRead>>

SrvPush(n) ==
  /\ task \in {"pushing", "aborting"}
  /\ credit' = credit - Charge(n)
  /\ sent' = sent + 1
  /\ s2c' = Append(s2c, Data(n))
  /\ task' = IF task = "aborting" THEN "done" ELSE "running"
  /\ UNCHANGED <<phase, entry, c2s, inbox, rx, queue, owed, cancelRead>>

SrvSourceEnds ==
  /\ task = "running"
  /\ task' = "done"
  /\ entry' = IF entry = "open" THEN "none" ELSE entry
  /\ UNCHANGED <<phase, credit, sent, s2c, c2s, inbox, rx, queue, owed, cancelRead>>

SrvReadCredit ==
  /\ c2s /= <<>>
  /\ Head(c2s).k = "credit"
  /\ c2s' = Tail(c2s)
  /\ credit' = IF entry = "open" THEN credit + Head(c2s).n ELSE credit
  /\ UNCHANGED <<phase, entry, task, sent, s2c, inbox, rx, queue, owed, cancelRead>>

SrvReadCancel ==
  /\ c2s /= <<>>
  /\ Head(c2s).k = "cancel"
  /\ c2s' = Tail(c2s)
  /\ cancelRead' = TRUE
  /\ CASE entry = "reserved" ->
            /\ entry' = "reservedCancelled"
            /\ UNCHANGED task
       [] entry = "open" ->
            /\ entry' = "none"
            /\ task' = CASE task = "running" -> "done"
                         [] task = "pushing" -> "aborting"
                         [] OTHER -> task
       [] OTHER -> UNCHANGED <<entry, task>>
  /\ UNCHANGED <<phase, credit, sent, s2c, inbox, rx, queue, owed>>

CliReadResp ==
  /\ s2c /= <<>>
  /\ Head(s2c).k = "resp"
  /\ s2c' = Tail(s2c)
  /\ inbox' = "open"
  /\ rx' = "alive"
  /\ UNCHANGED <<phase, entry, task, credit, sent, c2s, queue, owed, cancelRead>>

CliReadData ==
  /\ s2c /= <<>>
  /\ Head(s2c).k = "data"
  /\ s2c' = Tail(s2c)
  /\ IF inbox = "open" /\ rx = "alive"
       THEN /\ queue' = Append(queue, Head(s2c).n)
            /\ UNCHANGED inbox
       ELSE /\ inbox' = IF inbox = "open" THEN "removed" ELSE inbox
            /\ UNCHANGED queue
  /\ UNCHANGED <<phase, entry, task, credit, sent, c2s, rx, owed, cancelRead>>

CliRecv ==
  /\ rx = "alive"
  /\ queue /= <<>>
  /\ queue' = Tail(queue)
  /\ LET due == owed + Counted(Head(queue)) IN
       IF due >= Batch
         THEN /\ c2s' = Append(c2s, CreditMsg(due))
              /\ owed' = 0
         ELSE /\ owed' = due
              /\ UNCHANGED c2s
  /\ UNCHANGED <<phase, entry, task, credit, sent, s2c, inbox, rx, cancelRead>>

CliDrop ==
  /\ rx = "alive"
  /\ rx' = "dropped"
  /\ queue' = <<>>
  /\ owed' = 0
  /\ c2s' = Append(c2s, CancelMsg)
  /\ UNCHANGED <<phase, entry, task, credit, sent, s2c, inbox, cancelRead>>

Done ==
  /\ phase = "finished"
  /\ task \in {"unborn", "done"}
  /\ rx = "dropped"
  /\ s2c = <<>> /\ c2s = <<>>
  /\ UNCHANGED vars

Next ==
  \/ SrvRespond \/ SrvSpawn \/ SrvReady \/ (\E n \in Sizes: SrvPush(n))
  \/ SrvSourceEnds \/ SrvReadCredit \/ SrvReadCancel
  \/ CliReadResp \/ CliReadData \/ CliRecv \/ CliDrop
  \/ Done

Spec == Init /\ [][Next]_vars

TypeOK ==
  /\ phase \in {"prepare", "spawning", "finished"}
  /\ entry \in {"none", "reserved", "reservedCancelled", "open"}
  /\ task \in {"unborn", "running", "pushing", "aborting", "done"}
  /\ credit \in Int
  /\ sent \in 0..MaxFrames
  /\ inbox \in {"none", "open", "removed"}
  /\ rx \in {"none", "alive", "dropped"}
  /\ owed \in Nat
  /\ cancelRead \in BOOLEAN

Quiet ==
  /\ ~DataInFlight
  /\ queue = <<>>
  /\ ~\E i \in 1..Len(c2s): c2s[i].k = "credit"

Live == entry = "open" /\ task = "running" /\ rx = "alive" /\ sent < MaxFrames

NoStall == (Live /\ Quiet) => credit >= 1

CreditBalance ==
  (entry = "open" /\ rx = "alive")
    => credit + Total(s2c, "data") + QueueBytes + owed + Total(c2s, "credit") = Window

CancelLeaksNoCredit ==
  cancelRead => (entry /= "open" /\ task \in {"unborn", "aborting", "done"})

ServerDone == task = "done" \/ (phase = "finished" /\ task = "unborn")

CancelReleasesInbox ==
  (rx = "dropped" /\ ServerDone /\ ~DataInFlight)
    => inbox /= "open"

=============================================================================
