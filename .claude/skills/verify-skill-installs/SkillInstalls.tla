---------------------------- MODULE SkillInstalls ----------------------------
(* One skill folder in one scope: its store dir, its two link paths, and its
   lock entry, under install, update (replace), remove and restore. Every
   filesystem step can fail (the operation's undo runs) or be cut off by a
   crash (nothing runs). The person or another tool may delete the store or a
   link, or put their own entry at a free link path. Operations hold the
   scope mutex, so they never overlap, and each starts by sweeping the store's
   leftovers (storeJunk' = 0); install, remove and restore also sweep beside the
   links (update does it in ReplaceSweep).
   See SKILL.md for the coverage map. *)
EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS Windows, MaxCrashes, MaxFaults, MaxDamage

Targets == {"claude", "agents"}
Order == <<"claude", "agents">>
Versions == {"A", "B"}
Copy(v) == "copy" \o v

VARIABLES
  store, link, lock,
  storeJunk, linkJunk,
  pc, ver, idx, made, states, oldStore, swapped, tmpLink,
  owned0,
  foreign,
  crashes, faults, damage,
  justDone

vars == <<store, link, lock, storeJunk, linkJunk, pc, ver, idx, made, states,
          oldStore, swapped, tmpLink, owned0, foreign, crashes, faults, damage,
          justDone>>
opVars == <<pc, ver, idx, made, states, oldStore, swapped, tmpLink, owned0>>
fsVars == <<store, link, lock, storeJunk, linkJunk>>

(* layout::link_state with the lock as it is now. Every copy the app makes carries
   COPY_MARKER, so one that isn't the lock's content is stale, never foreign. *)
State(t) ==
  CASE link[t] = "none" -> "absent"
    [] link[t] = "foreign" -> "foreign"
    [] link[t] = "text" -> "text"
    [] link[t] = "sym" -> IF store # "none" THEN "healthy" ELSE "dangling"
    [] OTHER -> IF lock # "none" /\ link[t] = Copy(lock) THEN "healthy" ELSE "stale"

Owned(t) == State(t) \notin {"absent", "foreign"}

(* layout::link_skill: a symlink; on Windows the symlink may be refused and a marked copy
   of the store made (tmp beside, mark, rename: one step here) *)
Linked == IF Windows THEN {"sym", Copy(store)} ELSE {"sym"}

Init ==
  /\ store = "none" /\ lock = "none"
  /\ link \in [Targets -> {"none", "foreign"}]
  /\ foreign = {t \in Targets : link[t] = "foreign"}
  /\ storeJunk = 0 /\ linkJunk = 0
  /\ pc = "idle" /\ ver = "A" /\ idx = 1 /\ made = {} /\ states = [t \in Targets |-> "absent"]
  /\ oldStore = "none" /\ swapped = {} /\ tmpLink = FALSE /\ owned0 = {}
  /\ crashes = 0 /\ faults = 0 /\ damage = 0
  /\ justDone = "none"

Finish(op) == /\ pc' = "idle" /\ justDone' = op

----------------------------------------------------------------------------
(* install::commit, the fresh-install branch *)

InstallStart(v) ==
  /\ pc = "idle"
  /\ ver' = v
  /\ justDone' = "none"
  /\ IF \E t \in Targets : link[t] # "none" \/ lock # "none"
     THEN pc' = "idle"
     ELSE pc' = "i_clear"
  /\ storeJunk' = 0 /\ linkJunk' = 0
  /\ made' = {} /\ swapped' = {} /\ tmpLink' = FALSE /\ idx' = 1
  /\ UNCHANGED <<store, link, lock, states, oldStore, owned0, foreign,
                 crashes, faults, damage>>

(* `if store_dir.exists() { remove_dir_all }`, then the tmp copy *)
InstallClear ==
  /\ pc = "i_clear"
  /\ store' = "none"
  /\ storeJunk' = storeJunk + 1
  /\ pc' = "i_store" /\ idx' = 1 /\ made' = {}
  /\ UNCHANGED <<link, lock, linkJunk, ver, states, oldStore, swapped, tmpLink,
                 owned0, foreign, crashes, faults, damage, justDone>>

(* rename(tmp, store_dir) *)
InstallStore ==
  /\ pc = "i_store"
  /\ store' = ver /\ storeJunk' = storeJunk - 1
  /\ pc' = "i_link"
  /\ UNCHANGED <<link, lock, linkJunk, ver, idx, made, states, oldStore, swapped,
                 tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

InstallLink ==
  /\ pc = "i_link"
  /\ LET t == Order[idx] IN
       /\ link[t] = "none"
       /\ \E l \in Linked : link' = [link EXCEPT ![t] = l]
       /\ made' = made \cup {t}
  /\ IF idx = 2 THEN pc' = "i_lock" /\ UNCHANGED idx ELSE idx' = idx + 1 /\ UNCHANGED pc
  /\ UNCHANGED <<store, lock, storeJunk, linkJunk, ver, states, oldStore, swapped,
                 tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

(* lock::write, tmp + rename *)
InstallLock ==
  /\ pc = "i_lock"
  /\ lock' = ver
  /\ Finish("install")
  /\ UNCHANGED <<store, link, storeJunk, linkJunk, ver, idx, made, states,
                 oldStore, swapped, tmpLink, owned0, foreign, crashes, faults, damage>>

(* Undo::run *)
InstallUndo ==
  /\ pc \in {"i_store", "i_link", "i_lock"}
  /\ faults < MaxFaults
  /\ faults' = faults + 1
  /\ link' = [t \in Targets |-> IF t \in made THEN "none" ELSE link[t]]
  /\ store' = IF pc = "i_store" THEN store ELSE "none"
  /\ storeJunk' = IF pc = "i_store" THEN storeJunk - 1 ELSE storeJunk
  /\ pc' = "idle"
  /\ UNCHANGED <<lock, linkJunk, ver, idx, made, states, oldStore, swapped, tmpLink,
                 owned0, foreign, crashes, damage, justDone>>

----------------------------------------------------------------------------
(* install::replace: an update, or an approved restore *)

ReplaceStart(v) ==
  /\ pc = "idle"
  /\ ver' = v
  /\ justDone' = "none"
  /\ IF lock = "none" THEN pc' = "idle" ELSE pc' = "r_sweep"
  /\ storeJunk' = 0
  /\ made' = {} /\ swapped' = {} /\ tmpLink' = FALSE /\ idx' = 1
  /\ UNCHANGED <<store, link, lock, linkJunk, states, oldStore, owned0, foreign,
                 crashes, faults, damage>>

(* sweep_leftovers beside the links, then the link states by the old lock *)
ReplaceSweep ==
  /\ pc = "r_sweep"
  /\ linkJunk' = 0
  /\ states' = [t \in Targets |-> State(t)]
  /\ pc' = "r_aside"
  /\ UNCHANGED <<store, link, lock, storeJunk, ver, idx, made, oldStore, swapped,
                 tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

(* rename(store_dir, .old-<token>), then the tmp copy *)
ReplaceAside ==
  /\ pc = "r_aside"
  /\ oldStore' = store
  /\ store' = "none"
  /\ storeJunk' = storeJunk + (IF store # "none" THEN 1 ELSE 0) + 1
  /\ pc' = "r_store"
  /\ UNCHANGED <<link, lock, linkJunk, ver, idx, made, states, swapped, tmpLink,
                 owned0, foreign, crashes, faults, damage, justDone>>

ReplaceStore ==
  /\ pc = "r_store"
  /\ store' = ver /\ storeJunk' = storeJunk - 1
  /\ pc' = "r_link" /\ idx' = 1 /\ made' = {} /\ swapped' = {}
  /\ UNCHANGED <<link, lock, linkJunk, ver, states, oldStore, tmpLink, owned0,
                 foreign, crashes, faults, damage, justDone>>

NextLink == IF idx = 2 THEN pc' = "r_lock" /\ UNCHANGED idx
            ELSE pc' = "r_link" /\ idx' = idx + 1

(* One link: an owned copy (healthy or stale) is refreshed (tmp, set aside, rename); an absent or
   text-file one is (re)made; anything else is left. *)
ReplaceLink ==
  /\ pc = "r_link"
  /\ LET t == Order[idx] IN
       CASE states[t] \in {"healthy", "stale"} /\ link[t] \notin {"sym", "none"} ->
              /\ linkJunk' = linkJunk + 1 /\ tmpLink' = TRUE
              /\ pc' = "r_refresh_aside"
              /\ UNCHANGED <<link, made, swapped, idx>>
         [] states[t] = "text" ->
              /\ link' = [link EXCEPT ![t] = "none"]
              /\ linkJunk' = linkJunk + 1 /\ swapped' = swapped \cup {<<t, "text">>}
              /\ pc' = "r_make"
              /\ UNCHANGED <<made, tmpLink, idx>>
         [] states[t] = "absent" /\ link[t] = "none" ->
              /\ pc' = "r_make"
              /\ UNCHANGED <<link, linkJunk, made, swapped, tmpLink, idx>>
         [] OTHER ->
              /\ NextLink
              /\ UNCHANGED <<link, linkJunk, made, swapped, tmpLink>>
  /\ UNCHANGED <<store, lock, storeJunk, ver, states, oldStore, owned0, foreign,
                 crashes, faults, damage, justDone>>

ReplaceMake ==
  /\ pc = "r_make"
  /\ LET t == Order[idx] IN
       /\ \E l \in Linked : link' = [link EXCEPT ![t] = l]
       /\ made' = IF states[t] = "absent" THEN made \cup {t} ELSE made
  /\ NextLink
  /\ UNCHANGED <<store, lock, storeJunk, linkJunk, ver, states, oldStore, swapped,
                 tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

(* refresh_copy: set_aside(link), then rename(tmp, link) *)
ReplaceRefreshAside ==
  /\ pc = "r_refresh_aside"
  /\ LET t == Order[idx] IN link' = [link EXCEPT ![t] = "none"]
  /\ linkJunk' = linkJunk + 1
  /\ swapped' = swapped \cup {<<Order[idx], link[Order[idx]]>>}
  /\ pc' = "r_refresh_in"
  /\ UNCHANGED <<store, lock, storeJunk, ver, idx, made, states, oldStore, tmpLink,
                 owned0, foreign, crashes, faults, damage, justDone>>

ReplaceRefreshIn ==
  /\ pc = "r_refresh_in"
  /\ LET t == Order[idx] IN link' = [link EXCEPT ![t] = Copy(store)]
  /\ linkJunk' = linkJunk - 1 /\ tmpLink' = FALSE
  /\ NextLink
  /\ UNCHANGED <<store, lock, storeJunk, ver, states, made, oldStore, swapped,
                 owned0, foreign, crashes, faults, damage, justDone>>

ReplaceLock ==
  /\ pc = "r_lock"
  /\ lock' = ver
  /\ pc' = "r_clean"
  /\ UNCHANGED <<store, link, storeJunk, linkJunk, ver, idx, made, states,
                 oldStore, swapped, tmpLink, owned0, foreign, crashes, faults,
                 damage, justDone>>

(* remove_any on the old store and every set-aside link *)
ReplaceClean ==
  /\ pc = "r_clean"
  /\ storeJunk' = storeJunk - (IF oldStore # "none" THEN 1 ELSE 0)
  /\ linkJunk' = linkJunk - Cardinality(swapped)
  /\ Finish("replace")
  /\ UNCHANGED <<store, link, lock, ver, idx, made, states, oldStore, swapped,
                 tmpLink, owned0, foreign, crashes, faults, damage>>

(* ReplaceUndo::run: new links out, set-asides back, tmps and new store out, old store back *)
ReplaceUndo ==
  /\ pc \in {"r_store", "r_link", "r_make", "r_refresh_aside", "r_refresh_in", "r_lock"}
  /\ faults < MaxFaults
  /\ faults' = faults + 1
  /\ LET restored(t) ==
           IF t \in made THEN "none"
           ELSE IF \E p \in swapped : p[1] = t
                THEN (CHOOSE p \in swapped : p[1] = t)[2]
                ELSE link[t]
     IN link' = [t \in Targets |-> restored(t)]
  /\ linkJunk' = linkJunk - Cardinality(swapped) - (IF tmpLink THEN 1 ELSE 0)
  /\ store' = oldStore
  /\ storeJunk' = storeJunk - (IF oldStore # "none" THEN 1 ELSE 0)
                  - (IF pc = "r_store" THEN 1 ELSE 0)
  /\ tmpLink' = FALSE
  /\ pc' = "idle"
  /\ UNCHANGED <<lock, ver, idx, made, states, oldStore, swapped, owned0, foreign,
                 crashes, damage, justDone>>

----------------------------------------------------------------------------
(* install::remove *)

RemoveStart ==
  /\ pc = "idle"
  /\ justDone' = "none"
  /\ owned0' = {t \in Targets : Owned(t)}
  /\ IF {t \in Targets : Owned(t)} = {} /\ lock = "none" /\ store = "none"
     THEN pc' = "idle"
     ELSE pc' = "m_links"
  /\ storeJunk' = 0 /\ linkJunk' = 0
  /\ made' = {} /\ swapped' = {} /\ tmpLink' = FALSE /\ idx' = 1
  /\ UNCHANGED <<store, link, lock, ver, states, oldStore, foreign, crashes, faults, damage>>

RemoveLinks ==
  /\ pc = "m_links"
  /\ IF owned0 = {}
     THEN pc' = "m_store" /\ UNCHANGED <<link, owned0>>
     ELSE \E t \in owned0 :
            /\ link' = [link EXCEPT ![t] = "none"]
            /\ owned0' = owned0 \ {t}
            /\ UNCHANGED pc
  /\ UNCHANGED <<store, lock, storeJunk, linkJunk, ver, idx, made, states, oldStore,
                 swapped, tmpLink, foreign, crashes, faults, damage, justDone>>

RemoveStore ==
  /\ pc = "m_store"
  /\ store' = "none"
  /\ pc' = "m_lock"
  /\ UNCHANGED <<link, lock, storeJunk, linkJunk, ver, idx, made, states, oldStore,
                 swapped, tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

RemoveLock ==
  /\ pc = "m_lock"
  /\ lock' = "none"
  /\ Finish("remove")
  /\ UNCHANGED <<store, link, storeJunk, linkJunk, ver, idx, made, states, oldStore,
                 swapped, tmpLink, owned0, foreign, crashes, faults, damage>>

(* A step of remove fails: it returns the error with no undo *)
RemoveFails ==
  /\ pc \in {"m_links", "m_store", "m_lock"}
  /\ faults < MaxFaults
  /\ faults' = faults + 1
  /\ pc' = "idle"
  /\ UNCHANGED <<fsVars, ver, idx, made, states, oldStore, swapped, tmpLink, owned0,
                 foreign, crashes, damage, justDone>>

----------------------------------------------------------------------------
(* SkillService::restore_entry: a store whose content_hash differs from the lock's
   (missing or stale) is refetched at the locked commit; its hash matches the lock,
   so adopt_into_store sets any stale store aside, renames the copy in and removes
   the old one. Then Layout::relink. *)

RestoreStart ==
  /\ pc = "idle"
  /\ lock # "none"
  /\ justDone' = "none"
  /\ pc' = CASE store = lock -> "s_relink"
            [] store = "none" -> "s_adopt"
            [] OTHER -> "s_aside"
  /\ oldStore' = "none"
  /\ storeJunk' = 0 /\ linkJunk' = 0
  /\ made' = {} /\ swapped' = {} /\ tmpLink' = FALSE /\ idx' = 1
  /\ UNCHANGED <<store, link, lock, ver, states, owned0, foreign, crashes, faults, damage>>

(* rename(store_dir, .old-<token>) *)
RestoreAside ==
  /\ pc = "s_aside"
  /\ oldStore' = store
  /\ store' = "none"
  /\ storeJunk' = storeJunk + 1
  /\ pc' = "s_adopt"
  /\ UNCHANGED <<link, lock, linkJunk, ver, idx, made, states, swapped, tmpLink,
                 owned0, foreign, crashes, faults, damage, justDone>>

(* rename(tmp, store_dir) *)
RestoreAdopt ==
  /\ pc = "s_adopt"
  /\ store' = lock
  /\ pc' = IF oldStore # "none" THEN "s_clean" ELSE "s_relink"
  /\ UNCHANGED <<link, lock, storeJunk, linkJunk, ver, idx, made, states, oldStore,
                 swapped, tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

(* remove_any(.old-<token>), its error ignored *)
RestoreClean ==
  /\ pc = "s_clean"
  /\ storeJunk' = storeJunk - 1
  /\ pc' = "s_relink"
  /\ UNCHANGED <<store, link, lock, linkJunk, ver, idx, made, states, oldStore,
                 swapped, tmpLink, owned0, foreign, crashes, faults, damage, justDone>>

(* healthy and foreign are left; absent is made; dangling, text and stale are removed then made *)
RestoreRelink ==
  /\ pc = "s_relink"
  /\ LET t == Order[idx] IN
       IF State(t) \in {"healthy", "foreign"}
       THEN UNCHANGED link
       ELSE \E l \in Linked : link' = [link EXCEPT ![t] = l]
  /\ IF idx = 2 THEN Finish("restore") /\ UNCHANGED idx
     ELSE idx' = idx + 1 /\ UNCHANGED <<pc, justDone>>
  /\ UNCHANGED <<store, lock, storeJunk, linkJunk, ver, made, states, oldStore,
                 swapped, tmpLink, owned0, foreign, crashes, faults, damage>>

(* A failed rename(tmp, store_dir) renames the set-aside store back *)
RestoreFails ==
  /\ pc \in {"s_aside", "s_adopt", "s_relink"}
  /\ faults < MaxFaults
  /\ faults' = faults + 1
  /\ IF pc = "s_adopt" /\ oldStore # "none"
     THEN store' = oldStore /\ storeJunk' = storeJunk - 1
     ELSE UNCHANGED <<store, storeJunk>>
  /\ pc' = "idle"
  /\ UNCHANGED <<link, lock, linkJunk, ver, idx, made, states, oldStore, swapped,
                 tmpLink, owned0, foreign, crashes, damage, justDone>>

----------------------------------------------------------------------------
(* The server is killed mid-operation: no undo runs *)
Crash ==
  /\ pc # "idle"
  /\ crashes < MaxCrashes
  /\ crashes' = crashes + 1
  /\ pc' = "idle" /\ tmpLink' = FALSE
  /\ UNCHANGED <<fsVars, ver, idx, made, states, oldStore, swapped, owned0, foreign,
                 faults, damage, justDone>>

(* Someone else deletes the store or a link, or puts their own entry at a free link path *)
Damage ==
  /\ pc = "idle"
  /\ damage < MaxDamage
  /\ damage' = damage + 1
  /\ justDone' = "none"
  /\ \/ store # "none" /\ store' = "none" /\ UNCHANGED <<link, foreign>>
     \/ \E t \in Targets :
          /\ link[t] # "none"
          /\ link' = [link EXCEPT ![t] = "none"]
          /\ foreign' = foreign \ {t}
          /\ UNCHANGED store
     \/ \E t \in Targets :
          /\ link[t] = "none"
          /\ link' = [link EXCEPT ![t] = "foreign"]
          /\ foreign' = foreign \cup {t}
          /\ UNCHANGED store
  /\ UNCHANGED <<lock, storeJunk, linkJunk, opVars, crashes, faults>>

Done == pc = "idle" /\ UNCHANGED vars

Next ==
  \/ \E v \in Versions : InstallStart(v) \/ ReplaceStart(v)
  \/ InstallClear \/ InstallStore \/ InstallLink \/ InstallLock \/ InstallUndo
  \/ ReplaceSweep \/ ReplaceAside \/ ReplaceStore \/ ReplaceLink \/ ReplaceMake
  \/ ReplaceRefreshAside \/ ReplaceRefreshIn \/ ReplaceLock \/ ReplaceClean \/ ReplaceUndo
  \/ RemoveStart \/ RemoveLinks \/ RemoveStore \/ RemoveLock \/ RemoveFails
  \/ RestoreStart \/ RestoreAside \/ RestoreAdopt \/ RestoreClean \/ RestoreRelink
  \/ RestoreFails
  \/ Crash \/ Damage \/ Done

Spec == Init /\ [][Next]_vars

----------------------------------------------------------------------------
(* The rules *)

TypeOK ==
  /\ store \in {"none"} \cup Versions
  /\ lock \in {"none"} \cup Versions
  /\ storeJunk \in Nat /\ linkJunk \in Nat

(* An entry someone else put at a link path is never deleted or replaced. *)
ForeignUntouched == \A t \in foreign : link[t] = "foreign"

(* Between operations, every entry the app made at a link path is one it still owns, so it can
   remove, update and restore it. *)
AppEntriesOwned ==
  pc = "idle" => \A t \in Targets \ foreign : link[t] # "none" => Owned(t)

(* A finished install, update or restore leaves the store matching the lock and
   every link resolving to it (or someone else's entry left alone). *)
Consistent ==
  /\ lock # "none" /\ store = lock
  /\ \A t \in Targets : link[t] \in {"sym", Copy(lock), "foreign"}

(* A finished remove leaves nothing of the skill but other people's entries. *)
Clean ==
  /\ lock = "none" /\ store = "none"
  /\ \A t \in Targets : link[t] \in {"none", "foreign"}

FinishedOpsConsistent ==
  /\ justDone \in {"install", "replace", "restore"} => Consistent
  /\ justDone = "remove" => Clean

(* A finished operation leaves no `.tmp-` or `.old-` dirs from earlier crashes. *)
LeftoversSwept == justDone # "none" => (storeJunk = 0 /\ linkJunk = 0)

=============================================================================
