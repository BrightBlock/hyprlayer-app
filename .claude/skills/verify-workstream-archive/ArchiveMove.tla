---------------------------- MODULE ArchiveMove ----------------------------
(* Archiving one workstream on Windows: stop everything the app has open inside
   the worktree, then rename it into archived-contexts/. Windows refuses to
   rename a directory while any handle inside it is open, so a holder that is
   only asked to let go, not confirmed gone, can fail the move. Each app holder
   goes held -> releasing (asked) -> free (its OS handles are closed). Outside
   the app, one transient holder (Defender, the search indexer) may open the tree
   for a short while. See SKILL.md for the coverage map. *)
EXTENDS Naturals, FiniteSets

CONSTANTS Design, MaxRetries, MaxExternal

(* Every app-side holder of the worktree. A new one belongs here and in SKILL.md's table. *)
AppHolders == {"watch", "terminal", "script", "harness"}
Designs == {"v0_9_0", "fixed"}

VARIABLES hold, ext, extUses, pc, fails, blockedBy, backoff

vars == <<hold, ext, extUses, pc, fails, blockedBy, backoff>>

Retries == IF Design = "fixed" THEN MaxRetries ELSE 0

Init ==
  /\ hold = [h \in AppHolders |-> "held"]
  /\ ext = FALSE /\ extUses = 0
  /\ pc = "idle" /\ fails = 0 /\ blockedBy = {} /\ backoff = FALSE

(* stop_for_archive and the watch disarm: every holder is asked to let go. *)
Start ==
  /\ pc = "idle"
  /\ hold' = [h \in AppHolders |-> "releasing"]
  /\ pc' = "stopping"
  /\ UNCHANGED <<ext, extUses, fails, blockedBy, backoff>>

(* The OS finishes closing a holder's handles: a terminated process tree is gone,
   the watcher thread has closed its directory handles. *)
Released(h) ==
  /\ hold[h] = "releasing"
  /\ hold' = [hold EXCEPT ![h] = "free"]
  /\ UNCHANGED <<ext, extUses, pc, fails, blockedBy, backoff>>

(* fixed: archive waits until each holder confirms it let go.
   v0_9_0: kills and drops without waiting, so it may go straight on. *)
Stopped ==
  /\ pc = "stopping"
  /\ Design = "fixed" => \A h \in AppHolders : hold[h] = "free"
  /\ pc' = "moving"
  /\ UNCHANGED <<hold, ext, extUses, fails, blockedBy, backoff>>

ExternalOpens ==
  /\ pc \in {"stopping", "moving"}
  /\ ~ext /\ extUses < MaxExternal
  /\ ext' = TRUE /\ extUses' = extUses + 1
  /\ UNCHANGED <<hold, pc, fails, blockedBy, backoff>>

ExternalCloses ==
  /\ ext
  /\ ext' = FALSE /\ backoff' = FALSE
  /\ UNCHANGED <<hold, extUses, pc, fails, blockedBy>>

Holding == {h \in AppHolders : hold[h] # "free"} \cup (IF ext THEN {"external"} ELSE {})

(* `git worktree move`. A failed try is retried after a backoff long enough for a
   transient holder to close (the assumption the retry window rests on); out of
   retries, the archive rolls back and re-arms the watch it disarmed. *)
Move ==
  /\ pc = "moving" /\ ~backoff
  /\ IF Holding = {}
       THEN /\ pc' = "archived"
            /\ UNCHANGED <<hold, fails, blockedBy, backoff>>
       ELSE /\ fails' = fails + 1
            /\ blockedBy' = blockedBy \cup Holding
            /\ IF fails < Retries
                 THEN /\ backoff' = ext
                      /\ UNCHANGED <<hold, pc>>
                 ELSE /\ pc' = "rolledback"
                      /\ hold' = [hold EXCEPT !["watch"] = "held"]
                      /\ UNCHANGED backoff
  /\ UNCHANGED <<ext, extUses>>

Done ==
  /\ pc \in {"archived", "rolledback"}
  /\ UNCHANGED vars

Next ==
  \/ Start \/ Stopped \/ Move \/ ExternalOpens \/ ExternalCloses \/ Done
  \/ \E h \in AppHolders : Released(h)

Spec == Init /\ [][Next]_vars

----------------------------------------------------------------------------
TypeOK ==
  /\ Design \in Designs
  /\ hold \in [AppHolders -> {"held", "releasing", "free"}]
  /\ ext \in BOOLEAN /\ extUses \in 0..MaxExternal
  /\ pc \in {"idle", "stopping", "moving", "archived", "rolledback"}
  /\ fails \in Nat /\ blockedBy \subseteq AppHolders \cup {"external"}
  /\ backoff \in BOOLEAN

(* A move is never refused because of something the app itself still holds open. *)
NoAppHolderAtMove == blockedBy \cap AppHolders = {}

(* With the app's holders gone and outside holders transient, the archive succeeds. *)
NeverRolledBack == pc # "rolledback"

(* A rolled-back archive leaves the workstream watched again. *)
RollbackRearms == pc = "rolledback" => hold["watch"] = "held"
=============================================================================
