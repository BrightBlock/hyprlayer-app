# ADR 0007: One capability layer maps a stage's guardrails onto each harness

**Status:** accepted · 2026-08-26 · amended 2026-08-26 (codex's structured question
tool is enabled and reachable; Commit, Plan, and Research run two-phase on codex,
and Research gains a posture of its own) · amended 2026-09-19 (codex floor raised to
0.155.1: paginated threads rewind with `thread/revert`, the planning tool is switched
on, and a skipped phase two leaves plan mode; Claude's Commit never denied `git push`)
· amended 2026-10-16 (goals: the follow-up send path gains a second, persisted arm; codex
native goals off)

## Decision

A stage (Commit, Validate, Ship, …) carries an *intent* about what its agent may
do — commit but not push, read but not write, publish without invoking skills.
The three harnesses enforce that intent with different, non-overlapping
primitives, so the mapping lives in one place: `crates/hyprlayer-core/src/capability.rs`.

`capability::launch_policy(harness, stage, requested_mode) -> LaunchPolicy` is the
single point that lowers `(harness, stage)` onto the primitive a driver
understands. `SessionService::spawn_driver`
(`crates/hyprlayer-core/src/runtime/sessions/service.rs`) is a thin consumer: it
destructures the variant for the harness it is spawning and passes those fields
straight to the driver.

| Harness | Primitive it enforces | Lowering |
|---|---|---|
| **claude** | per-command `allowed`/`disallowed` tool lists + a `--agent` | `pipeline::policy_for` (the lists) + `pipeline::agent_for` (Ship) |
| **codex** | a sandbox + approval `CodexPosture` (no per-command gate) | `capability::codex_posture(stage, mode)` |
| **opencode** | its own HTTP permission model | none injected — `permission_mode` is informational |

Two related, harness-aware seams sit alongside it:

- **Prompt directives that name a Claude-native tool** are chosen per harness in
  `pipeline.rs`. `commit_prompt_default(harness)` is the first: Claude's copy
  names the `AskUserQuestion` tool (which renders a structured question card),
  opencode gets wording that names no tool, and codex gets wording that asks for
  "a single structured question tool call" without naming one. Claude's copy still
  names `AskUserQuestion` because that tool genuinely does not exist on codex.

  Codex reaches the same card by a different route, and it took three conditions
  at once — any one missing and the tool is absent or refused:

  1. `initialize` carries `capabilities: { experimentalApi: true }`.
  2. The spawn argv carries `-c tools.experimental_request_user_input.enabled=true`
     (`APP_SERVER_ARGS`, `codex/driver.rs`).
  3. The turn starts with `collaborationMode: {"mode":"plan","settings":{"model":…}}`,
     whose `settings.model` codex requires. Without it the tool is present but
     refused — its own description says it is only available in Plan mode, which is
     why an enabled-but-unmoded tool stays inert for the ephemeral title spawner.

  `collaborationMode` is **sticky**: it is a thread-level setting, so the executing
  turn must send `mode: "default"` explicitly rather than omitting the field. That
  stickiness is the reason these stages are two turns rather than one.
- **`CodexPosture`** (`crates/hyprlayer-core/src/codex/protocol.rs`) is codex's
  guardrail primitive — `approval_policy` + `sandbox` + `network_access`. It is
  resolved once, at launch, into `CodexDriver::SpawnConfig`; a mid-session
  permission-mode change kills and relaunches (`RestartSession` in
  `crates/hyprlayer-server/src/streams.rs`), so there is no second path to keep in
  sync.

## Consequences

- Adding a stage means updating `policy_for` (the Claude lists) and, only when the
  stage's intent is not already captured by its permission mode, `codex_posture`.
  Three stages need the latter today: **Validate**, a read-only audit, forces
  `read-only` sandbox regardless of the session's mode (without it codex ran
  Validate under `workspace-write` and could mutate the tree); **Commit**, which
  pins `untrusted` approval so a push cannot run silently (see below); and
  **Research**, which softens approval to `on-request` *only* in the mode that
  would have produced `untrusted`. Research explores, and `untrusted` raises a card
  per read-only command; a measured codex Commit review turn raised 4 cards for far
  less work. The softening is guarded on `mode == PermissionMode::Default` so a
  mode the user chose deliberately is left alone, and it is still tighter than
  Plan, which already resolves to `never` via its `AcceptEdits` default.

- **Two-phase stages on codex.** `pipeline::codex_phase_two(harness, stage, bodies)`
  is the single predicate deciding whether a stage runs as collaborate-then-execute,
  and `followup::await_followup` (`runtime/sessions/followup.rs`) is the single waiter,
  shared with Ship; `send_app_turn` sends its turn. Three stages qualify: **Commit**
  (review and ask, then commit and maybe push), **Plan**, and **Research**
  (collaborate and ask, then write the document). Commit two-phases only when its
  built-in body is in play — a user's custom body owns its turn — while Plan and
  Research have no custom-body path at all, so they always two-phase.

  The codex arm deliberately diverges from Ship's: it fires only after a turn whose
  `turn_status` parses to exactly `TurnStatus::Completed`, because its follow-up is
  destructive where Ship's only writes a PR description. Codex reports an
  interrupted turn as `"interrupted"` (verified on 0.149.1 and 0.155.1), but the
  check demands an exact `Completed` rather than excluding known-bad statuses, so an
  absent or unrecognised status fails safe too. Each set-mode-then-send pair runs
  under the session op-lock `send_user_message` already takes, so a message typed
  mid-flight cannot consume the one-shot collaboration mode.

  A phase two that never runs still has to leave plan mode, because the mode
  outlives the turn: on 0.155.1, after an interrupted Commit phase one, the user's
  next turn ran in plan mode and codex refused to write. So a skipped or abandoned
  codex follow-up arms `mode: "default"` for the next user turn. A relaunch never
  re-arms a follow-up, so a resume that reports `collaborationMode: plan` arms the
  same; only 0.156+ restores and reports the saved mode.

- **Two arms, one send path.** App-originated turns go through one call,
  `SessionService::send_app_turn`
  (`crates/hyprlayer-core/src/runtime/sessions/service.rs`): the session op lock, a
  live check, a reservation under the triggering client's author with a server-minted
  nonce, un-archive, send. Two orchestrators use it.
  - **One-shot stage follow-ups** (`followup.rs`: Ship's `/describe_pr`, codex phase
    two) are unchanged. Only `launch_and_converse` arms them; the first `TurnDone`
    fires them; there is no timeout; and nothing re-arms them after a kill, a
    `resume_session` or a server restart. A user interjecting while the first turn is
    in flight can still make the waiter fire on the wrong `TurnDone`. Their user
    message is persisted, and delivered live to attachments that asked for echoes.
  - **Goal loops** (`runtime/sessions/goal_loop.rs`, driven by `GoalService`) are
    persisted in `goals` and `goal_iterations` and capped by iterations and active
    time. A goal's driver task is spawned and registered, never awaited inside
    `launch_and_converse`, and stops before the runtime detaches its harnesses at
    shutdown. It launches harnesses only through `launch_if_exited_held`, under the
    session op lock. An iteration ends on its own echoed message, a `TurnDone`, and
    two quiet seconds, so a native Stop-hook or `/goal` loop inside it (one
    `TurnDone`) is one iteration. Goals are re-armed when the server opens. The
    goal's orphaned harness is reaped first, and more than three resumes without a
    finished iteration pause the goal. Spend is capped too: dollars on Claude and
    OpenCode, uncached input plus output tokens on Codex, with `--max-budget-usd` as
    a backstop on Claude processes the goal spawns.
    Only the app drives a goal: Codex runs with `-c features.goals=false` in
    `APP_SERVER_ARGS`, and every Claude session is spawned with `ProposeGoal` denied.
    A goal's Validation check opens a fresh Validate session. On Claude, that session
    may run exactly the goal's approved commands without asking; it is report-only
    (no file edits, no thoughts sync), and its posture is otherwise Validate's.

- **Version floor: codex-cli 0.155.1.** Codex has created threads with
  `historyMode: "paginated"` since 0.151, and 0.155.1 rejects `thread/rollback` on
  them (`-32600 "paginated threads do not support thread/rollback"`), so a rewind
  follows the thread's mode:

  - **Paginated** threads rewind with `thread/revert {threadId, beforeTurnId}`,
    which drops that turn and every later one. The driver records each root turn
    id from `turn/started` and, on resume, seeds them from `thread.turns` and
    `thread/turns/list`; ordinal *n* reverts before the *n*th. When the seeded
    count disagrees with the session's logged user turns (a rewind that never
    reached the model, say), no id can be trusted and the rewind stays app-side.
  - **Legacy** threads, from older codex or with no `historyMode`, keep
    `thread/rollback`'s turn count. 0.156 removes the method, and its "unknown
    variant" error maps to the harness's "rewind is not supported", so the app log
    is truncated and the session relaunched without a model-level rewind.

  The planning tool is opt-in since 0.152, so `APP_SERVER_ARGS` also carries
  `-c tools.update_plan.enabled=true`; without it `turn/plan/updated` never fires
  and the plan view stays empty. `-c` on `app-server`, `experimentalApi`,
  `collaborationMode`, and `developerInstructions` all predate the floor, so
  nothing above it is version-gated, and `BinaryResolver::require` refuses a codex
  that reports a version below it. `codex/probe.rs`'s argv is deliberately *not*
  given the `-c` flags — it needs no tool and is not the floor check. Named here so
  nobody "fixes" that inconsistency.

- **The Prompt Library is harness-unaware.** `PromptDefaults::commit`
  (`src-tauri/src/commands/projects.rs`) shows Claude's wording to every harness, and
  saving any custom body — including that default text verbatim — sets
  `builtin_bodies.commit` and reverts codex Commit to a single prose turn carrying a
  tool name codex does not have. This predates the two-phase work and is recorded,
  not fixed.

- **Three degradation classes, three behaviors.** An unknown `-c` key is tolerated
  without `--strict-config`, so the tool is simply absent and the model asks in
  prose. A silently ignored `collaborationMode` means the turn runs normally, the
  model asks in prose, the answer is never read, and the followup still fires — for
  Commit that means phase two commits un-consulted, with the push still
  approval-gated by the `untrusted` posture, so nothing silent reaches the network;
  each phase-two prompt therefore carries an "if the previous turn produced nothing"
  clause so it still does useful work. A *rejected* `collaborationMode` errors the
  `turn/start`, and the driver's synthetic `TurnDone` is marked `"failed"`, so the
  followup does not fire and the stage ends as a visibly failed single turn rather
  than an unrequested commit. The tripwire for the middle case is
  `CodexDriver::observed_collab_mode()`, fed by the `thread/settings/updated`
  notification — whose payload nests under `threadSettings`, not `settings`.
- **Guardrails codex cannot reproduce, by construction — do not assume parity:**
  - **Per-command allow and deny.** Claude's Commit pre-allows `git add` and
    `git commit`, denies `Bash(gh:*)`, and leaves `git push` off both lists, so its
    `Default` mode asks before a push (`pipeline.rs`'s
    `commit_invokes_the_skill_then_offers_a_push_but_never_a_pr` asserts all three).
    Codex has neither list, so it can neither pre-allow `git commit` nor deny `gh`.
    The nearest mapping is coarser: `codex_posture` pins the Commit stage to
    `untrusted` approval regardless of the session mode, so *every* command — the
    push included — prompts before it runs, even under
    `AcceptEdits`/`BypassPermissions` that would otherwise auto-run it. That is
    stricter than Claude, where only the mode asks before a push, so a mode that
    stops asking (`BypassPermissions`) lets it through. The cost of the coarseness:
    `git add`/`git commit` also prompt on codex, where Claude pre-allows them (in
    `Default` mode codex already prompted for those, so the extra friction only
    appears if the user lowered the mode). A push is gated, not forbidden —
    approving it lets it through.

    Since Commit went two-phase the push is gated *twice*: agreed with the user in
    phase one through the question card, and approved in phase two through the
    `untrusted` posture. Only the second is a guardrail. The card is agreement — a
    model that ignored the answer would still hit the approval prompt, which is the
    property that actually keeps a push off the network. Codex 0.149 does expose
    per-turn `approvalPolicy`/`sandboxPolicy` overrides that could relax phase one,
    but they are sticky per-thread like `collaborationMode` and their interaction
    with plan-mode exec is unverified, so they are deliberately unused.
  - **The Ship publishing agent.** `SHIP_AGENT` (a restricted `--agent` that cannot
    invoke skills) is a Claude primitive. Codex and opencode receive its intent as
    prompt text only.
- Opencode drives its own permission model and takes nothing from the policy; the
  `Opencode` variant carries `permission_mode` for symmetry, and the launch arm
  leaves `launched_mode` unset as before.

## Rationale

Before this, the mapping was scattered across the three arms of `launch_session`:
Claude consumed the whole `StagePolicy`, codex silently used only its
`permission_mode` field and dropped the tool lists, and one prompt hard-coded a
Claude tool name that broke codex. The lists were written in Claude's vocabulary,
so "just pass them to codex" was never possible — codex has no per-command gate to
receive them. Centralising the lowering makes the real divergence legible in one
file and its tests, and makes the un-reproducible guardrails a documented fact
rather than a silent gap discovered per stage.
