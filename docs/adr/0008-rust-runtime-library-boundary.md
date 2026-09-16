# ADR 0008: The Rust runtime library boundary — models → core facade → app shell

**Status:** accepted · 2026-08-30 · amended 2026-08-31 (phase 7b removed the last
`compat_*` accessor and the `desktop-internal` feature; the transitional-accessor
paragraphs below are rewritten as history) · amended 2026-08-31 (phase 7c declared the
supported surface — `docs/architecture/rust-runtime-library.md` is now the consumer
reference, and it, not this ADR's outline table, describes what actually exists) ·
amended 2026-08-31 (Linux ARM64 dropped as a support target — no consumer, and its
cross/Docker gate blocked the Linux release path) · amended 2026-08-31 (the boundary
rules are recorded here as review rules; `check-rust-boundaries.mjs` and
`rust-boundaries.json` are deleted) · amended 2026-09-03 (the "no remote daemon" third
of the daemon/`Host`/remote-terminal bullet under "What we're not doing" is superseded
by ADR-0009 — the one runtime moves into a `hyprlayer-server` process; the other two
thirds stand)

## Decision

The Rust workspace is refactored, over several independently-shippable phases, into a
reusable, headless, Rust-native runtime library while preserving the desktop
application, SQLite data, `settings.json`, stored session replay, and established
Tauri wire contracts. The approved graph:

```text
hyprlayer-models
        ↓
hyprlayer-core
  HyprlayerRuntime facade
        ↓
hyprlayer-app
  Tauri transport + desktop integration
```

`hyprlayer-models` holds dependency-light serialized contracts: `serde` as its only
normal dependency, with an optional exact-version `specta`. It has no Tokio,
filesystem, process, Git, database, PTY, notification, HTTP, or Tauri dependency.
`hyprlayer-core` is the application engine and exposes the first supported Rust API:
`HyprlayerRuntime`, its service handles, operational request/result types, model
re-exports, and `CoreError`. `hyprlayer-app` retains app-path resolution, IPC
registration, webview channels, window/menu/plugin integration, and the desktop-only
integrations enumerated below.

This is a decision about where the seam is drawn today, not a rewrite of the whole
tree at once: the seven phases each compile and ship independently, preserve current
user data, and carry their own automated and manual verification.

### Type ownership

- Selected cross-consumer durable domain and event contracts belong in
  `hyprlayer-models`. This is a deliberately small dependency-light contract crate,
  not a requirement that every serialized value move there.
- Device-local/mixed `AppSettings` is the explicit exception: the value and
  `SettingsStore` stay together in core because the file mixes secrets, provider
  configuration, and desktop preferences that are not a portable public model
  contract.
- Operational requests/results and service handles belong in
  `hyprlayer-core::runtime::<service>`.
- Tauri-only command envelopes, desktop view DTOs, and `tauri_specta::Event`
  wrappers remain in `hyprlayer-app`, with explicit `From` conversions.
- Concrete provider protocols, stores, Git/process implementations, and registries
  remain internal core implementation details.
- **The runtime facade lives inside `hyprlayer-core` for the duration of this
  refactor, by decision rather than default.** A facade crate introduced now would
  churn every import twice — the app consumed core internals through transitional
  `#[doc(hidden)]` compat accessors until phase 7b removed the last one — and binding
  generation needs the concrete types either way. The cost is that the facade boundary
  is not a boundary the compiler can see: `hyprlayer-app` can reach any of core's
  `pub mod`s, and only the rules under [Boundary rules](#boundary-rules) say it
  shouldn't. The sanctioned follow-up, now that the compat
  accessors are gone, is extracting a `hyprlayer-runtime` facade crate — re-exporting
  models and `CoreError`, with `hyprlayer-app` depending on it instead of on core
  directly — at which point most of those rules become ordinary crate boundaries and
  stop needing to be written down at all. It has not been done:
  the shell still imports thirty core modules for the desktop carve-outs, so the
  extraction would move that reach rather than remove it. Phase 7c's narrower answer
  is to declare the supported surface at the crate root and document the rest as
  implementation detail with no stability promise.

### Runtime events

`hyprlayer-models::RuntimeEvent` contains exactly:

```rust
pub enum RuntimeEvent {
    ConfigChanged,
    BinariesChanged,
    ThoughtsIndexUpdated {
        update: ThoughtsIndexUpdateKind,
        ids: Vec<String>,
    },
    ProjectsChanged,
    PrStatusChanged {
        workstream_id: String,
    },
    WorktreeChanged {
        workstream_id: String,
    },
    WorkstreamBranchChanged {
        workstream_id: String,
        branch: String,
        previous: String,
    },
    SessionLifecycle {
        session_id: String,
        workstream_id: String,
        state: SessionState,
    },
    ScriptRunChanged {
        workstream_id: String,
        kind: ScriptKind,
        phase: RunPhase,
        exit_code: Option<i32>,
        port: Option<u16>,
    },
    ProjectCreateProgress {
        op_id: String,
        step: CreateStep,
        status: StepStatus,
        detail: Option<String>,
    },
}

pub enum ThoughtsIndexUpdateKind {
    Updated,
    Removed,
    Rescanned,
}
```

Tauri maps each variant to today's existing event:

| Runtime event | Existing Tauri event |
|---|---|
| `ConfigChanged` | `events::ConfigChanged` |
| `BinariesChanged` | `events::BinariesChanged` |
| `ThoughtsIndexUpdated` | `events::ThoughtsIndexUpdated`, converting the typed kind to the existing string |
| `ProjectsChanged` | No-op in the Tauri event bridge; commands already invalidate project queries through their result path |
| `PrStatusChanged` | `events::PrStatusChanged` |
| `WorktreeChanged` | `events::WorktreeChanged` |
| `WorkstreamBranchChanged` | `events::WorkstreamBranchChanged` |
| `SessionLifecycle` | `events::SessionLifecycle` |
| `ScriptRunChanged` | `events::ScriptRunChanged` |
| `ProjectCreateProgress` | `events::ProjectCreateProgress` |

`OmarchyThemeChanged` and `QuitRequested` remain Tauri-only; they never become
`RuntimeEvent` variants.

### The supported facade surface

```rust
impl HyprlayerRuntime {
    pub async fn open(options: RuntimeOptions) -> Result<Arc<Self>>;
    pub async fn start(&self) -> Result<()>;
    pub fn set_foreground(&self, foreground: bool);
    pub async fn shutdown(&self) -> ShutdownReport;
    pub fn shutdown_now(&self);
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent>;
    pub fn worktree_root(&self) -> &Path;

    pub fn settings(&self) -> SettingsService<'_>;
    pub fn providers(&self) -> ProviderService<'_>;
    pub fn sessions(&self) -> SessionService<'_>;
    pub fn projects(&self) -> ProjectService<'_>;
    pub fn library(&self) -> LibraryService<'_>;
    pub fn agents(&self) -> AgentService<'_>;
    pub fn prompts(&self) -> PromptService<'_>;
    pub fn workstreams(&self) -> WorkstreamService<'_>;
    pub fn attachments(&self) -> AttachmentService<'_>;
    pub fn terminals(&self) -> TerminalService<'_>;
    pub fn scripts(&self) -> ScriptService<'_>;
}
```

All service methods are `pub async fn` returning `Result<T>` unless noted otherwise.
Named request/result types are public operational DTOs beside their service; the
durable values they carry reuse `hyprlayer-models`. Concrete stores, provider
clients, and registries are never accepted by the public API. In outline, by
service:

| Service | Covers |
|---|---|
| `SettingsService` | settings get/set/patch, binary status/override/redetect, config migration |
| `ProviderService` | provider status/catalog/auth/logout, model discovery, Ollama sync, title generation |
| `SessionService` | list/start/resume/restart/rewind, log/log-after, attach/send/answer/approve/interrupt/kill, session config |
| `ProjectService` | bootstrap, inspect, import (repo/folder)/clone/remove/reorder, profiles, backend health, thoughts sync, onboarding, create/publish, GitHub owners |
| `LibraryService` | query/get/save/patch-frontmatter/create, reindex, thoughts git status |
| `AgentService` | list/read/save/install/update/remove, bundle status/install/repair |
| `PromptService` | list, import preview/apply, export |
| `WorkstreamService` | list/create/rename/update/archive/restore/purge/reprovision, git status, PR status, documents |
| `AttachmentService` | import, save |
| `TerminalService` | open/attach/write/resize/close, open-agent, capture session |
| `ScriptService` | settings, run/stop/output/runtime, run-lifecycle |

The exact request/result struct for each method is reviewed per-service as each
phase lands it; this table is the reviewed initial surface, not a promise that it
never grows — nor a description of what landed. Several phases diverged from it
deliberately (`SettingsService` has no patch DTO, `SessionService` no
`start`/`send`/`answer` under those names, several services take plain arguments
where the outline implies a Request struct).
`docs/architecture/rust-runtime-library.md` enumerates the surface as it exists
and records each divergence; read it, not this table, to write against the facade.

`SessionAttachment` and `TerminalAttachment` own their bounded
receivers and cancellation-on-drop behavior. Adding a method after the facade is
declared (phase 7) requires an API-doc update and a headless integration test.

### Command-module disposition

Every current Tauri command module either retires into a runtime service or keeps
a narrowed, transport-only responsibility. This is the map later phases execute
against:

| Current module | Runtime owner | Final Tauri responsibility |
|---|---|---|
| `commands/mod.rs` | settings/config, provider auth environment, project guards | `db_task`/`blocking` remain transport adapters until their callers move |
| `commands/system.rs` | settings/binaries/config, provider model reconciliation | installers/self-maintenance, editor discovery, sound, FDA, system settings, quit/exit |
| `pr_watch.rs` | `runtime::pr_watcher` | feed foreground state and map events |
| `settings.rs` | `core::settings` | none |
| `commands/sessions.rs` | `runtime::sessions` | command DTO conversion and channel pump |
| `opencode.rs` | `core::opencode::runtime` | none |
| `commands/harnesses.rs` | `runtime::providers` | desktop installer actions only |
| `commands/models.rs` | `runtime::providers` | macOS Keychain adapter only |
| `commands/ollama.rs` | `runtime::providers` | command mapping |
| `commands/titles.rs` | `runtime::providers` | command mapping |
| `commands/projects.rs` | `runtime::projects` | bootstrap/project view envelopes and command mapping |
| `commands/onboarding.rs` | `runtime::projects` | desktop navigation only |
| `commands/library.rs` | `runtime::library` | reveal/open actions and IPC DTO conversion |
| `commands/agents.rs` | `runtime::agents` | command mapping |
| `commands/prompts.rs` | `runtime::prompts` | dialog-selected paths and command mapping |
| `commands/workstreams.rs` | `runtime::workstreams` | wire view conversion |
| `commands/attachments.rs` | `runtime::attachments` | dialog/source-path acquisition |
| `commands/terminal.rs` | `runtime::terminals` | base64 channel forwarding remains unchanged |
| `commands/scripts.rs` | `runtime::scripts` | terminal channel forwarding |
| `src-tauri/src/editors.rs` | remains a desktop carve-out | discovery/icons/launching |

`commands/mod.rs::claude_auth_env` moves to `runtime::providers`;
`normalize_branch_prefixes` moves with core settings/config; `ensure_registered_project`
and `effective_for` move with the project/config services. `db_task` and `blocking`
are permitted only as temporary Tauri scheduling adapters and disappear from a
command once that command's owning service has moved.

## Explicit Tauri carve-outs

The following remain in `hyprlayer-app`, by design, for the life of this
architecture — not a temporary gap the runtime will eventually absorb:

- Tauri app-config/app-data/resource path resolution and construction of
  `RuntimePaths`.
- macOS Keychain credential retrieval through a narrow runtime credential adapter.
- OpenCode and Hyprlayer CLI desktop installer/self-maintenance flows.
- Editor discovery, editor icons, and editor launching.
- Sound playback, Full Disk Access status/UI, and opening system settings.
- Windows, menus, plugins, dialogs, openers, notifications, updater UI, and
  webview behavior.
- Asset-protocol scope registration.
- IPC command registration, Tauri event wrappers, request-scoped channels, and
  conversion to `AppError`.
- Foreground/focus observation; Tauri feeds foreground state to the runtime's PR
  watcher.
- Omarchy theme polling and the `OmarchyThemeChanged` event; core's `omarchy.rs`
  helpers take their paths as explicit parameters.

All other HTTP calls move to core with their owning service. The desktop `reqwest`
dependency remains only until its final command-layer caller moves, then is
removed.

## Boundary rules

These are review rules, not a build gate. They were briefly a 343-line Node script
(`scripts/check-rust-boundaries.mjs` plus a `rust-boundaries.json` of 18 rules,
2026-08-30 to 2026-08-31) that regex-scanned the tree behind a hand-rolled Rust
lexer. It was deleted: it ran from no gate, ~120 of its lines existed only to stop
its own regexes lying about strings and comments, three of its manifest rules
restated an error cargo raises first (a `hyprlayer-core` → `hyprlayer-app`
dependency fails `cargo check` with `error: cyclic package dependency`, exit 101 —
though `cargo metadata --no-deps` does tolerate it), and 8 of its 11 source rules
carried a prose escape clause naming legitimate code that tripped them. A rule
whose text is mostly exceptions is a rule for a reader, not for a scanner.

**Manifests.** `hyprlayer-models` depends on `serde` plus an optional exact-version
`specta`, and nothing else. Neither `hyprlayer-models` nor `hyprlayer-core` depends
on Tauri — Tauri stops at `hyprlayer-app`. `hyprlayer-app` keeps no HTTP client of
its own; network calls belong to the core service that owns them, reached through
the `ProviderHttp` port. `tauri-plugin-updater` is the named exception, per the
carve-outs above. The `models → core → app` direction needs no rule: cargo rejects
the cycle.

**Core.** No `tauri::` or `tauri_specta::` imports in `hyprlayer-core` or
`hyprlayer-models` — move the Tauri type into `src-tauri` and convert at the
boundary. Core modules outside `runtime/` do not import `crate::runtime`; they take
the policy value as a parameter, so a primitive stays testable without building a
runtime. That is the Rust analog of ADR-0003's one-way import direction.

**The shell.** `hyprlayer-app` maps DTOs, forwards channels, and performs desktop
actions. It does not hold a `Db` handle or a `db_task` scheduler — every database
transaction belongs to the service that owns its use case, and the shell's `store::`
imports are types only. It does not write or decode stored session events; that is
`runtime::sessions` through `store::session_event_codec`, though forwarding a
`Channel<SessionEvent>` is exactly its job. It does not sequence worktree
create/remove/archive/restore/purge — that is `runtime::workstreams`' locked
transaction, while read-and-refresh helpers (`worktree::{status, folder_status,
detect_base_branch, fetch_remote, pull_base, archive_dir_for}`, `worktree::seed`,
`worktree::thoughts_links`) sequence nothing and are fine. It does not spawn PTYs
(`runtime::terminals` and `runtime::scripts` do; the shell forwards their base64
`TerminalEvent` channel). It does not run the Setup/Archive gate
(`ScriptService::run_lifecycle`, called by `runtime::workstreams` from inside its own
lock). It does not construct or drive providers, or dial out to them — that is
`runtime::providers` and `runtime::sessions`; `ClaudeProbe` and `CodexProbe` are the
Settings → Binaries version probes and stay in the shell. It holds no document,
agent, bundle, or project provisioning policy.

**Known divergences, carried deliberately.** The 2026-08-31 audit of the
command-module disposition table found 12 of 19 rows matching and 7 partial, all of
them deferred phase 4/5 work rather than phase 7 regressions. `commands/harnesses.rs`
still reads and writes OpenCode's auth and config files directly through
`opencode::config`. `commands/mod.rs` still holds `ensure_registered_project` and
`effective_for`. `commands/system.rs` normalizes branch prefixes and reconciles
Claude model keys. `commands/sessions.rs` validates OpenCode model support in
`set_session_model`. `commands/titles.rs` composes model resolution and title
sanitization. `commands/workstreams.rs` fires the Setup script itself after
`create()` returns — moving it inside the locked `create()` would hold the project
lock for the whole setup timeout and serialise concurrent creates, a concurrency
change no phase specified — and holds an mtime concurrency check plus PR
attach/merge sequencing. These are the seven; anything else that looks like one is
new.

## Hermetic construction

`HyprlayerRuntime::open` is hermetic: it touches no home-directory or global
provider state outside explicitly supplied paths and configured
project/thoughts/worktree roots. Core operations do not call `dirs::home_dir`,
read `HOME`/`XDG`/`PATH` directly, or call `std::env::current_dir`; apart from the
three exceptions named below, every such read is routed through an explicit
`RuntimeEnvironment`/`RuntimePaths` value supplied at `open`. Hermeticity
constrains core's *own* reads; it says nothing
about what child processes (PTY terminals, provider CLIs) inherit — they receive
the captured desktop environment verbatim, so user shells keep `SSH_AUTH_SOCK`,
locale, and custom variables.

One tracked exception: `config/types.rs` does the CLI config's own `~`-expansion,
threaded through every `thoughts_root()` caller. It is tracked, not fixed.

Two ambient reads are outside the rule's shape and need naming, because a scan for
`dirs::`/`current_dir` never sees them: `process/binaries.rs`'s `join_deduped`
fallback reads `PATH` through `env::var`, and `runtime/terminals.rs`'s
`default_shell` reads `SHELL`. Both are deliberate; neither is a precedent for a
third.

## Supported build targets

Actual support targets are:

- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `x86_64-unknown-linux-gnu`

Standalone, non-publishing scripts verify the library builds and runs headless on
those targets.

`aarch64-unknown-linux-gnu` was a fourth target when this ADR was written — as a
library only, never an application bundle — and was dropped on 2026-08-31 before
its check had ever run. Nothing embeds `hyprlayer-core` on ARM Linux, so the
`cross check` proved a property with no consumer, and it was not free: it ran from
`release-linux-remote.sh` under `set -e` ahead of the x64 bundle build, so an
unprovisioned builder failed the entire Linux release, and provisioning meant a
pinned `cross` install plus a ~1 GB Docker image. `Cross.toml`, the
`--linux-arm64` arm of `scripts/check-rust-libraries.sh`, and the release step went
with it; the rest of that script went too, a day later, once the surviving host
check turned out to be one `cargo check` line. Reinstating the target means
rebuilding all of that, not adding a triple back to this list.

## What we're not doing

- No foreign-language ABI, C API, UniFFI, Swift, Kotlin, or JNI layer.
- No separate core repository.
- No crates.io publishing.
- No broad `Host` or remote-terminal abstraction. This bullet read "No remote daemon,
  broad `Host`, or remote-terminal abstraction" until 2026-09-03, when ADR-0009
  superseded its first third: the one runtime moves into a `hyprlayer-server` process
  reached over a local socket or SSH. The other two stand — the shell is always a
  client, so there is no local/remote branch to abstract, and terminals are not
  abstracted, they are simply on the server.
- No provider or store microcrates.
- No database replacement.
- No `ProjectKey` migration or project-identity redesign.
- No frontend redesign or transcript-reducer port.
- No crate-per-module decomposition.
- No settings-file split or secret synchronization.
- No Linux ARM64 support, as an application bundle or as a library target.
- No source-compatibility promise for the current private/unpublished core API.

## Rationale

Before this decision, the workspace held two crates only: `hyprlayer-core` (Tauri-free,
but exposing 27 implementation modules rather than a supported application facade)
and `src-tauri`. The real runtime aggregate was `AppState`, an ad hoc struct combining
core components with settings, provider managers, registries, watchers, and desktop
state. Application transactions — project provisioning and rollback, document
optimistic concurrency and reindexing, provider selection and two-phase sequencing,
workstream create/archive/restore/purge — lived in the Tauri command layer rather
than in core, alongside settings, registries, the OpenCode runtime, provider
catalog/model/auth work, title generation, bundle installation, prompts, lifecycle
scripts, terminals, watchers, and PR polling.

Provider-neutral contracts sat under provider namespaces (`AgentDriver` imported
`ExitOutcome` and `SessionEvent` from `claude`), provider identity leaked into Rust
as `claude_session_id`, and session storage had multiple uncoordinated consumers —
`spawn_forwarder`, workflow recovery, `attach_session`, and `get_session_log` did not
all pass through one codec, and the frontend decoded stored payloads separately. None
of this was reachable outside a Tauri process, and none of it could be exercised,
tested, or reused without a webview. This ADR is the answer: a dependency-light
contracts crate, a core crate that owns a supported `HyprlayerRuntime` facade, and a
Tauri shell that is actually thin, arrived at over phases so the app keeps shipping
throughout.

## Consequences

- `settings.json`, the SQLite `claude_session_id` column, and the serialized
  `claudeSessionId` wire key are preserved unchanged throughout the refactor, even
  though the in-process Rust name becomes `provider_session_id` — an intentional
  private source break, not a wire break.
- Every SQLite session-event consumer is required to go through one
  `core::store::session_event_codec` once it exists (phase 2 onward); raw stored
  question events stay legacy-string encoded for rollback, while runtime/Tauri/
  frontend consumers receive canonical typed questions.
- **The transitional compat accessors are gone.** Phase 3 parked `hyprlayer-app` on
  ten `#[doc(hidden)]` accessors (`compat_config`, `compat_binaries`,
  `compat_settings`, `compat_db`, `compat_worktrees`, `compat_index`,
  `compat_sessions`, `compat_terminals`, `compat_scripts`, `compat_watchers`) behind
  a non-default `desktop-internal` feature, each to be retired by the phase that
  moved its owning service. Phase 7a retired the settings/binaries/config three and
  the three that had no consumers left; phase 7b retired `compat_db`,
  `compat_scripts`, and `compat_watchers` along with the feature itself, from both
  manifests. `AppState` is now a single field, the runtime, and the shell reaches
  core only through the facade and the carve-outs above. Nothing is excluded from
  the supported surface on transitional grounds any more — what is excluded is
  excluded on the type-ownership grounds stated above.
- This plan makes no source-compatibility promise for the current private,
  unpublished core API — call sites inside this workspace are free to change; only
  the wire/on-disk contracts named above are frozen.
- The compatibility fixtures under `tests/fixtures/compat/v0_7/` (settings, raw
  session-event rows, a pinned bindings-contract excerpt, and one representative
  `AppError` per vertical slice) exist so that every later phase can prove it
  changed nothing it didn't mean to, before it's trusted to change what it does
  mean to. Bindings drift is caught by `release.sh`'s gate, which regenerates
  `src/lib/bindings.ts` through `cargo test` and fails on a dirty file; the
  standalone `scripts/check-bindings.sh` that did the same regenerate-and-compare
  was deleted as a second copy of that check wired into nothing.
- **Runtime migrations run once, and that is a deliberate narrowing.** The ledger at
  `<app_data_dir>/runtime-migrations.json` records each migration on completion and
  never reruns it. Some of what it replaced were *every-launch* guards — the OpenCode
  default-model scrub ran on every startup and would clean a retired provider's
  default however often it came back. A one-shot migration will not: if such a value
  reappears (a restored backup, a config synced from another machine, a hand edit),
  nothing cleans it again. This is accepted because a migration is a statement about
  a profile's history, not a policy that holds it in shape. A rule that must keep
  holding is not a migration and does not belong in the ledger — it belongs in the
  service that owns the value, where it can run whenever that value is read.
