----------------------------- MODULE PtyResume -----------------------------
(* One PTY agent's conversation across server restarts: its pty_agents row, the
   startup pass (resume::resume_after_restart and resume_pty), a spawner's
   message_agent to it while it is exited (resume_pty with a message), a desktop
   launch resuming the same conversation (TerminalService::supersede, then the
   launch), the tab closing (TerminalService::close), archiving the workstream,
   and the agent's own turns and exits. Each resume takes the terminal's resume
   lock (pty_resume_locks), checks under it, then launches while still holding
   it. SKILL.md maps every action to the code. *)
EXTENDS Integers

CONSTANTS MaxResumes, MaxRestarts, MaxActs

VARIABLES
  restarts,
  procs,          \* live processes on the conversation
  row,            \* "stored" or "none": the pty_agents row
  working,        \* row.working
  resumes,        \* row.resumes
  nudgeOwed,      \* a resumed process whose nudge or message hasn't started a turn
  archived,
  pass,           \* the startup pass: "idle", "start", "due", "locked", "go", "done"
  msg,            \* a spawner's message_agent: "idle", "want", "locked", "go"
  desk,           \* a desktop's resuming launch: "idle", "want", "locked", "launch"
  deskTakes,      \* the superseded agent still owed its nudge
  lock,           \* who holds the terminal's resume lock
  acts,           \* messages, desktop launches and closes so far (bounds the model)
  received,       \* restart resumes since the last finished turn or a person's relaunch
  resumedIdle,    \* a restart resume launched an agent that was not mid-turn
  resumedArchived \* a resume launched in an archived workstream

vars == <<restarts, procs, row, working, resumes, nudgeOwed, archived, pass, msg,
          desk, deskTakes, lock, acts, received, resumedIdle, resumedArchived>>

Init ==
  /\ restarts = 0 /\ procs = 1 /\ row = "stored" /\ working = FALSE /\ resumes = 0
  /\ nudgeOwed = FALSE /\ archived = FALSE /\ pass = "idle" /\ msg = "idle"
  /\ desk = "idle" /\ deskTakes = FALSE /\ lock = "none" /\ acts = 0
  /\ received = 0 /\ resumedIdle = FALSE /\ resumedArchived = FALSE

Agent == <<working, resumes, nudgeOwed>>
Actors == <<pass, msg, desk, deskTakes, lock>>
Flags == <<received, resumedIdle, resumedArchived>>

\* PtyAgents::apply/save: a prompt (or the owed nudge landing) starts a turn.
StartTurn ==
  /\ procs = 1 /\ (~working \/ nudgeOwed)
  /\ working' = TRUE /\ nudgeOwed' = FALSE
  /\ UNCHANGED <<restarts, procs, row, resumes, archived, acts>> /\ UNCHANGED Actors
  /\ UNCHANGED Flags

\* A Stop: next_saved writes working = false with turn_done, which zeroes resumes.
FinishTurn ==
  /\ procs = 1 /\ working /\ ~nudgeOwed
  /\ working' = FALSE /\ resumes' = 0 /\ received' = 0
  /\ UNCHANGED <<restarts, procs, row, nudgeOwed, archived, acts>> /\ UNCHANGED Actors
  /\ UNCHANGED <<resumedIdle, resumedArchived>>

\* It exits while the server runs (SessionEnd, a crash, a --resume claude can't load):
\* release deletes the row.
ExitsOnItsOwn ==
  /\ procs = 1
  /\ procs' = 0 /\ row' = "none"
  /\ UNCHANGED <<restarts, archived, acts>> /\ UNCHANGED Agent /\ UNCHANGED Actors
  /\ UNCHANGED Flags

\* The server stops: server_stopping keeps the row; every process and caller dies with it.
Restart ==
  /\ restarts < MaxRestarts
  /\ restarts' = restarts + 1
  /\ procs' = 0 /\ pass' = "start" /\ msg' = "idle" /\ desk' = "idle" /\ lock' = "none"
  /\ UNCHANGED <<row, archived, acts, deskTakes>> /\ UNCHANGED Agent /\ UNCHANGED Flags

\* resume_after_restart's restart_skip, before the lock. An archived row is forgotten;
\* an exhausted one is reported and settled (not_resumed).
PassCheck ==
  /\ pass = "start"
  /\ IF row = "stored" /\ ~archived /\ working /\ resumes < MaxResumes
       THEN pass' = "due" /\ UNCHANGED <<row, working>>
       ELSE /\ pass' = "done"
            /\ row' = IF row = "stored" /\ archived THEN "none" ELSE row
            /\ working' = IF resumes >= MaxResumes THEN FALSE ELSE working
  /\ UNCHANGED <<restarts, procs, resumes, nudgeOwed, archived, msg, desk, deskTakes,
                 lock, acts>> /\ UNCHANGED Flags

PassLock ==
  /\ pass = "due" /\ lock = "none"
  /\ lock' = "pass" /\ pass' = "locked"
  /\ UNCHANGED <<restarts, procs, row, archived, msg, desk, deskTakes, acts>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

\* resume_pty under the lock: live, or a closed tab's row gone, or restart_skip again.
PassRecheck ==
  /\ pass = "locked"
  /\ IF procs = 0 /\ row = "stored" /\ ~archived /\ working /\ resumes < MaxResumes
       THEN pass' = "go" /\ UNCHANGED lock
       ELSE pass' = "done" /\ lock' = "none"
  /\ UNCHANGED <<restarts, procs, row, archived, msg, desk, deskTakes, acts>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

\* record_pty_resume, then launch_agent, which holds the workstream against archive
\* (hold_unarchived): an archived workstream refuses the launch.
PassLaunch ==
  /\ pass = "go"
  /\ pass' = "done" /\ lock' = "none"
  /\ IF ~archived
       THEN /\ procs' = procs + 1 /\ resumes' = resumes + 1 /\ nudgeOwed' = TRUE
            /\ received' = received + 1
            /\ resumedIdle' = (resumedIdle \/ ~working)
            /\ resumedArchived' = (resumedArchived \/ archived)
       ELSE UNCHANGED <<procs, resumes, nudgeOwed, received, resumedIdle, resumedArchived>>
  /\ UNCHANGED <<restarts, row, working, archived, msg, desk, deskTakes, acts>>

\* send_pty_message to an exited agent it spawned: stored() finds the row.
MsgWant ==
  /\ msg = "idle" /\ acts < MaxActs /\ row = "stored" /\ procs = 0
  /\ msg' = "want" /\ acts' = acts + 1
  /\ UNCHANGED <<restarts, procs, row, archived, pass, desk, deskTakes, lock>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

MsgLock ==
  /\ msg = "want" /\ lock = "none"
  /\ lock' = "msg" /\ msg' = "locked"
  /\ UNCHANGED <<restarts, procs, row, archived, pass, desk, deskTakes, acts>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

\* Live: the message goes in as to any live agent (Resumed::Live). Gone or archived: refused.
MsgRecheck ==
  /\ msg = "locked"
  /\ IF procs = 0 /\ row = "stored" /\ ~archived
       THEN msg' = "go" /\ UNCHANGED lock
       ELSE msg' = "idle" /\ lock' = "none"
  /\ UNCHANGED <<restarts, procs, row, archived, pass, desk, deskTakes, acts>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

MsgLaunch ==
  /\ msg = "go"
  /\ msg' = "idle" /\ lock' = "none"
  /\ IF ~archived
       THEN /\ procs' = procs + 1 /\ resumes' = resumes + 1 /\ nudgeOwed' = TRUE
            /\ resumedArchived' = (resumedArchived \/ archived)
       ELSE UNCHANGED <<procs, resumes, nudgeOwed, resumedArchived>>
  /\ UNCHANGED <<restarts, row, working, archived, pass, desk, deskTakes, acts,
                 received, resumedIdle>>

\* A desktop's launch with resume and this conversation's id (a reload whose attach failed).
DeskWant ==
  /\ desk = "idle" /\ acts < MaxActs /\ row = "stored"
  /\ desk' = "want" /\ acts' = acts + 1
  /\ UNCHANGED <<restarts, procs, row, archived, pass, msg, deskTakes, lock>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

DeskLock ==
  /\ desk = "want" /\ lock = "none"
  /\ lock' = "desk" /\ desk' = "locked"
  /\ UNCHANGED <<restarts, procs, row, archived, pass, msg, deskTakes, acts>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

\* supersede: under the lock, end the holder and delete its row; a nudge it still owed moves on.
DeskEnd ==
  /\ desk = "locked"
  /\ deskTakes' = (row = "stored" /\ working /\ (procs = 0 \/ resumes > 0))
  /\ procs' = 0 /\ row' = "none" /\ lock' = "none" /\ desk' = "launch"
  /\ UNCHANGED <<restarts, archived, pass, msg, acts>> /\ UNCHANGED Agent
  /\ UNCHANGED Flags

\* The launch itself, after the lock: a new terminal and row on the same conversation.
DeskLaunch ==
  /\ desk = "launch"
  /\ desk' = "idle"
  /\ IF ~archived
       THEN /\ procs' = procs + 1 /\ row' = "stored" /\ working' = deskTakes
            /\ resumes' = 0 /\ nudgeOwed' = deskTakes /\ received' = 0
       ELSE UNCHANGED <<procs, row, working, resumes, nudgeOwed, received>>
  /\ UNCHANGED <<restarts, archived, pass, msg, deskTakes, lock, acts>>
  /\ UNCHANGED <<resumedIdle, resumedArchived>>

\* close (a closed tab, kill_agent): under the lock, end it and delete its row.
Close ==
  /\ acts < MaxActs /\ lock = "none" /\ (procs > 0 \/ row = "stored")
  /\ procs' = 0 /\ row' = "none" /\ acts' = acts + 1
  /\ UNCHANGED <<restarts, archived, pass, msg, desk, deskTakes, lock>>
  /\ UNCHANGED Agent /\ UNCHANGED Flags

Archive ==
  /\ ~archived
  /\ archived' = TRUE
  /\ UNCHANGED <<restarts, procs, row, acts>> /\ UNCHANGED Agent /\ UNCHANGED Actors
  /\ UNCHANGED Flags

Done ==
  /\ pass \in {"idle", "done"} /\ msg = "idle" /\ desk = "idle"
  /\ UNCHANGED vars

Next ==
  \/ StartTurn \/ FinishTurn \/ ExitsOnItsOwn \/ Restart
  \/ PassCheck \/ PassLock \/ PassRecheck \/ PassLaunch
  \/ MsgWant \/ MsgLock \/ MsgRecheck \/ MsgLaunch
  \/ DeskWant \/ DeskLock \/ DeskEnd \/ DeskLaunch
  \/ Close \/ Archive \/ Done

Spec == Init /\ [][Next]_vars

\* Never two processes on one conversation: no second resume of a live agent, and a
\* desktop's relaunch ends the stored one before it starts its own.
NoDoubleResume == procs <= 1

\* Restart resumes stop at MAX_PTY_RESUMES (3) until the agent finishes a turn or a person relaunches it.
ResumeBounded == received <= MaxResumes

\* No resume starts an agent in an archived workstream.
NoResumeWhenArchived == ~resumedArchived

\* A restart resumes only an agent that was mid-turn (or still owed its nudge).
NoResumeOfIdle == ~resumedIdle

TypeOK ==
  /\ procs \in 0..2 /\ row \in {"stored", "none"}
  /\ working \in BOOLEAN /\ nudgeOwed \in BOOLEAN /\ archived \in BOOLEAN
  /\ resumes \in Nat /\ received \in Nat
  /\ pass \in {"idle", "start", "due", "locked", "go", "done"}
  /\ msg \in {"idle", "want", "locked", "go"}
  /\ desk \in {"idle", "want", "locked", "launch"}
  /\ lock \in {"none", "pass", "msg", "desk"}
=============================================================================
