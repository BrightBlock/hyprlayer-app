# ADR 0007: One capability layer maps a stage's guardrails onto each harness

**Status:** accepted · 2026-08-26 · amended 2026-08-26 (codex's structured question
tool is enabled and reachable; Commit, Plan, and Research run two-phase on codex,
and Research gains a posture of its own)

## Decision

A stage (Commit, Validate, Ship, …) carries an *intent* about what its agent may
do — commit but not push, read but not write, publish without invoking skills.
The three harnesses enforce that intent with different, non-overlapping
primitives, so the mapping lives in one place: `crates/hyprlayer-core/src/capability.rs`.

`capability::launch_policy(harness, stage, requested_mode) -> LaunchPolicy` is the
single point that lowers `(harness, stage)` onto the primitive a driver
understands. `launch_session` (`src-tauri/src/commands/sessions.rs`) is a thin
consumer: it destructures the variant for the harness it is spawning and passes
those fields straight to the driver.

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
  permission-mode change kills and relaunches (`sessions.rs`), so there is no
  second path to keep in sync.

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
  and `run_followup` (`src-tauri/src/commands/sessions.rs`) is the single
  orchestrator, shared with Ship. Three stages qualify: **Commit** (review and ask,
  then commit and maybe push), **Plan**, and **Research** (collaborate and ask, then
  write the document). Commit two-phases only when its built-in body is in play — a
  user's custom body owns its turn — while Plan and Research have no custom-body
  path at all, so they always two-phase.

  The codex arm deliberately diverges from Ship's: it fires only after a turn whose
  `turn_status` parses to exactly `TurnStatus::Completed`, because its follow-up is
  destructive where Ship's only writes a PR description. Codex reports an
  interrupted turn as `"interrupted"` (verified on 0.149.1), but the check demands
  an exact `Completed` rather than excluding known-bad statuses, so an absent or
  unrecognised status fails safe too. Each set-mode-then-send pair runs under the
  session op-lock `send_user_message` already takes, so a message typed mid-flight
  cannot consume the one-shot collaboration mode.

  Inherited from the Ship precedent, and unchanged: no timeout, no re-arming after
  a kill or `resume_session`, the phase-two user message is persisted but not pushed
  to the live channel (it appears on reload), and a user interjecting while the first
  turn is in flight can make the waiter fire on the wrong `TurnDone`.

- **Version floor: codex-cli 0.149.** `-c` on `app-server`, `experimentalApi`, and
  `collaborationMode` were all verified against 0.149.1. There is no runtime version
  gate: a codex too old to accept `-c` fails at spawn with the existing exit
  diagnostics. `codex/probe.rs`'s argv is deliberately *not* given the flag — it
  needs no tool and is not the floor check — so Settings → Binaries can report a
  healthy codex whose sessions would fail at spawn. Named here so nobody "fixes"
  that inconsistency.

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
  - **Per-command deny.** Commit's `disallowed_tools` of `Bash(git push:*)` and
    `Bash(gh:*)` has no codex equivalent — codex cannot allow `git commit` while
    denying `git push`. The nearest faithful mapping is coarser: `codex_posture`
    pins the Commit stage to `untrusted` approval regardless of the session mode,
    so *every* command — the push included — prompts before it runs, even under
    `AcceptEdits`/`BypassPermissions` that would otherwise auto-run it. This mirrors
    Claude, whose Commit deny-list is likewise mode-independent. The cost of the
    coarseness: `git add`/`git commit` also prompt on codex, where Claude
    pre-allows them (in `Default` mode codex already prompted for those, so the
    extra friction only appears if the user lowered the mode). A push is gated, not
    forbidden — approving it lets it through.

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
