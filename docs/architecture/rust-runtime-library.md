# The `hyprlayer-core` runtime library

The supported Rust API for embedding Hyprlayer without a webview. It is the
facade ADR-0008 describes: `hyprlayer-models` (dependency-light contracts) →
`hyprlayer-core` (all application logic, exposed through `HyprlayerRuntime`) →
`hyprlayer-app` (Tauri transport and desktop carve-outs).

`crates/hyprlayer-core/tests/headless_runtime.rs` is a complete embedder that
runs on every `cargo test --workspace`: open, start, import, observe an event,
shut down twice concurrently, and assert nothing was written outside the supplied
paths. Read it alongside this document — it is the worked example, and unlike a
prose snippet it cannot rot silently.

This document describes the surface **as it exists**, which in several places is
not the surface ADR-0008's outline table or the implementation plan sketched. The
tree is right and this document follows the tree.

## Constructing a runtime

Everything the runtime reads is supplied: no `dirs::` fallback, no ambient
`HOME`/`XDG`/cwd read. Two deliberate exceptions, both named in ADR-0008's
*Hermetic construction* section rather than hidden here: `process/binaries.rs`
reads `PATH` as a fallback when resolving a binary, and `runtime/terminals.rs`
reads `SHELL` to pick a default shell.

```rust
use std::sync::Arc;
use hyprlayer_core::{
    HyprlayerRuntime, NoCredentialLookup, RuntimeEnvironment, RuntimeOptions, RuntimePaths,
};

let options = RuntimeOptions::new(environment, paths)
    .with_credential_lookup(Arc::new(NoCredentialLookup));

let runtime = HyprlayerRuntime::open(options).await?;
runtime.start().await?;
```

### `RuntimeEnvironment`

| Field | Meaning |
|---|---|
| `home_dir: PathBuf` | The home the binary resolver's well-known directories hang off. Never read from the OS. |
| `process_cwd: PathBuf` | The working directory child processes inherit when a call doesn't name one. |
| `process_env: BTreeMap<OsString, OsString>` | The environment PTY terminals and provider CLIs receive. `RuntimeEnvironment::var(key)` is the accessor core itself uses. |

`process_env` is consulted by core for two things in particular: `PATH`, which
seeds the augmented binary search path, and `SHELL`, which `start()`'s
login-shell PATH probe spawns as `$SHELL -lic`. Omitting `SHELL` does not skip
the probe — it falls back to `/bin/zsh`. A headless embedder that does not want
a login shell run should name one that cannot spawn, which is what both the
example and the test do.

### `RuntimePaths`

All thirteen fields are required; there are no defaults.

`hyprlayer_config_file`, `app_config_dir`, `app_data_dir`,
`default_worktree_root`, `default_repos_root`, `opencode_config_file`,
`opencode_auth_file`, `opencode_data_dir`, `claude_config_dir`,
`claude_managed_settings_files`, `codex_home`, `agent_roots`, `download_dir`.

`app_data_dir` is the profile: `hyprlayer.db`, `.runtime.lock`, and
`runtime-migrations.json` live there. `app_config_dir` holds `settings.json`.

### `RuntimeOptions`

```rust
RuntimeOptions::new(environment: RuntimeEnvironment, paths: RuntimePaths) -> Self
    .with_credential_lookup(Arc<dyn CredentialLookup + Send + Sync>) -> Self
```

`new` installs the production-safe `NoCredentialLookup`, which reports every
credential as not-found without touching platform credential storage. The
desktop shell replaces it with its macOS Keychain adapter; a headless embedder
leaves it alone. It is the only port with production injection — every other
port is fixed at `open`, and the test-only `RuntimeBuilder` replaces them.

To implement `CredentialLookup` you also need `CredentialLookupRequest`,
`CredentialLookupResult`, `CredentialValue`, and `CredentialLookupError` — all
re-exported at the crate root. `CredentialValue` redacts itself in both `Debug`
and `Display`; `expose()` is the only way out.

### What hermeticity does and does not promise

**It promises:** `HyprlayerRuntime::open` and every core operation read and
write only under the supplied `RuntimePaths`, the supplied
`RuntimeEnvironment`, and the project/thoughts/worktree roots read back out of
those supplied files. Core does not call `dirs::home_dir`, does not read
`HOME`/`XDG`/`PATH` from the process, and does not call
`std::env::current_dir`.

**It does not promise anything about child processes.** Hermeticity constrains
core's *own* reads, never what a child inherits. PTY terminals, provider CLIs,
and `git` receive the desktop environment verbatim on purpose, so a user's
shell keeps `SSH_AUTH_SOCK`, locale, and custom variables. Two specific
consequences worth knowing before you write a test that asserts against ambient
state:

- `HyprCli::command` (`crates/hyprlayer-core/src/hyprcli.rs`) sets only `PATH`
  on the child. A hyprlayer-CLI child therefore inherits the *calling process's*
  environment, not `RuntimeEnvironment::process_env`. Threading `process_env`
  through every spawn is deliberately not done — it would reach `Exec`, the PTY
  layer, and every driver.
- `start()` will shell the CLI exactly once, through
  `migrate_config_if_needed`, and only when the supplied
  `hyprlayer_config_file` parses as needing migration. A schema-current config
  (`{"version": 4}` is enough) keeps that path unreached.

So an assertion of the form "nothing outside the supplied paths changed" is an
assertion about core's own writes. `tests/headless_runtime.rs` scopes it that
way and says so at the call site.

### The profile lock

`open` takes an exclusive `fs4` lock on `<app_data_dir>/.runtime.lock` and holds
the file for the runtime's lifetime. A second `open` against the same profile —
from this process or another — returns `CoreError::RuntimeInUse { path }`
(`code() == "runtime_in_use"`) **without mutating anything**. The lock is
released when the `Arc<HyprlayerRuntime>` is dropped, not when `shutdown()`
returns; a runtime you have shut down but still hold still owns its profile.

## Lifecycle

```rust
impl HyprlayerRuntime {
    pub async fn open(options: RuntimeOptions) -> Result<Arc<Self>>;
    pub async fn start(self: &Arc<Self>) -> Result<()>;
    pub fn set_foreground(&self, foreground: bool);
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent>;
    pub fn worktree_root(&self) -> &Path;
    pub async fn shutdown(&self) -> ShutdownReport;
    pub fn shutdown_now(&self);
    pub async fn migrate_config_if_needed(&self) -> bool;
    pub async fn rebuild_thoughts_index(&self);
}
```

`open` acquires the profile lock, applies pending runtime migrations, opens the
database, and clears orphan PIDs left by a prior run. It starts no background
work.

`start` is one-shot: a second call returns `CoreError::RuntimeAlreadyStarted`,
and a call after shutdown returns `CoreError::RuntimeStopped`. It spawns three
tracked tasks — a startup task (login-shell PATH enrichment, config migration
if needed, thoughts index rescan), the config-file poll loop, and the PR
watcher. Service methods check the same state: calling one on a stopped runtime
returns `CoreError::RuntimeStopped`.

`shutdown` is idempotent and safe to call concurrently. Exactly one caller runs
the teardown; everyone else waits for it and receives a clone of the same
`ShutdownReport { elapsed, forced_process_count, task_failures }`. Teardown
stops each tracked task with a 2s budget inside a 10s overall deadline, aborting
what is left, then kills terminals, sessions, and the OpenCode server.

`shutdown_now` is the synchronous fallback for a non-async exit hook. It runs
its kill step at most once and does not block; a later async `shutdown` still
runs in full.

`set_foreground` feeds the PR watcher's cadence. A headless embedder that never
calls it stays in the foreground-cadence default.

## Events

`subscribe()` returns a `tokio::sync::broadcast::Receiver<RuntimeEvent>` with a
256-slot buffer. A subscriber only sees events sent after it subscribed, so
subscribe before the call whose event you want to observe; a slow subscriber
lags and receives `RecvError::Lagged`.

`RuntimeEvent` (defined in `hyprlayer-models`, re-exported at
`hyprlayer_core::RuntimeEvent`) has exactly ten variants:

| Variant | Payload |
|---|---|
| `ConfigChanged` | — |
| `BinariesChanged` | — |
| `ThoughtsIndexUpdated` | `update: ThoughtsIndexUpdateKind`, `ids: Vec<String>` |
| `ProjectsChanged` | — |
| `PrStatusChanged` | `workstream_id` |
| `WorktreeChanged` | `workstream_id` |
| `WorkstreamBranchChanged` | `workstream_id`, `branch`, `previous` |
| `SessionLifecycle` | `session_id`, `workstream_id`, `state: SessionState` |
| `ScriptRunChanged` | `workstream_id`, `kind: ScriptKind`, `phase: RunPhase`, `exit_code: Option<i32>`, `port: Option<u16>` |
| `ProjectCreateProgress` | `op_id`, `step: CreateStep`, `status: StepStatus`, `detail: Option<String>` |

`ThoughtsIndexUpdateKind` is `Updated | Removed | Rescanned`.

## Services

Eleven handles, each borrowed from the runtime for the duration of one call:

```rust
runtime.settings()   runtime.providers()  runtime.sessions()   runtime.projects()
runtime.library()    runtime.agents()     runtime.prompts()    runtime.workstreams()
runtime.attachments() runtime.terminals() runtime.scripts()
```

Each handle's operational request/result types live in that service's module and
are supported alongside it: `hyprlayer_core::runtime::{settings_service,
providers, sessions, projects, library, agents, prompts, workstreams,
attachments, terminals, scripts}`. The handles themselves are also re-exported
at the crate root.

Every method takes `&self` and returns `Result<T>` unless shown otherwise.

### `SettingsService`

| Method | Returns |
|---|---|
| `get()` | `AppSettings` (not a `Result`) |
| `set(AppSettings)` | `()` |
| `update(impl FnOnce(&mut AppSettings))` | `AppSettings` |
| `binaries()` | `Arc<BinaryResolver>` (sync) |
| `config()` | `Arc<ConfigService>` (sync) |

`binaries()` and `config()` hand back the concrete component rather than a
DTO-shaped operation. They are deliberate desktop carve-outs, not compat
scaffolding: CLI self-update, the login-shell probe, the OpenCode installer, the
`git`/`gh` gating in workstream commands, and `effective_for`'s thoughts-config
read all need the component itself. They are the one place the facade hands out
an internal type, and they are supported for that reason.

### `ProviderService`

`claude_auth_status()`, `claude_probe()`, `codex_probe()`,
`codex_login_with_api_key(String)`, `codex_logout()`, `claude_model_windows()`,
`ollama_status()`, `ollama_set_endpoint(Option<String>)`,
`ollama_set_selected_models(Vec<String>)`, `ollama_set_default_model(Option<String>)`,
`generate_title_claude(&str, &str)`, `generate_title_codex(&str, &str)`,
`generate_title_opencode(&str, &str)`, `opencode_server()`,
`opencode_server_if_running()`, `invalidate_opencode_server()`,
`opencode_catalog(bool)`.

Network calls go through the internal `ProviderHttp` port; the desktop shell has
no HTTP client of its own.

### `SessionService`

| Method | Returns |
|---|---|
| `get(&str)` | `Option<Session>` |
| `list(&str)` (workstream id) | `Vec<Session>` |
| `list_active()` | `Vec<ActiveSession>` |
| `list_active_ids()` | `Vec<String>` (not a `Result`) |
| `log(&str)` | `Vec<SessionLogEntry>` |
| `log_after(&str, epoch: u64, after_seq: Option<i64>)` | `Vec<SessionLogEntry>` |
| `attach(&str)` | `SessionAttachment` |
| `prepare_start(StartStageSessionRequest)` | `StartedStageSession` |
| `launch(LaunchSessionParams)` | `Arc<dyn AgentDriver>` |
| `launch_and_converse(LaunchAndConverseParams)` | `()` |
| `rewind(RewindSessionRequest)` | `RewindSessionOutcome` |
| `kill_and_await_exit(&str)` | `()` (not a `Result`) |
| `live_driver(&str)` | `Option<Arc<dyn AgentDriver>>` (not a `Result`) |
| `permission_mode(&str)` | `Option<PermissionMode>` (not a `Result`) |
| `op_lock(&str)` | `Arc<tokio::sync::Mutex<()>>` (not a `Result`) |

`SessionAttachment { replay: Vec<SessionLogEntry>, live: mpsc::Receiver<Result<SessionEvent, SessionStreamError>> }`
delivers durable replay plus a bounded live stream (512 slots plus a reserved
terminal slot); a consumer too slow for it is detached with a lag cursor and
recovers history through `log_after`.

Sending a message, answering a question, and resolving a permission are driven
through the `AgentDriver` returned by `live_driver`, under the `op_lock` for that
session — there are no `send`/`answer`/`approve` methods on the service.

### `ProjectService`

`bootstrap()`, `find(&str)`, `source_of(&str)`, `inspect_repo_path(&str)` (sync),
`inspect(&str)`, `import_repository(ImportRepositoryRequest)`,
`import_folder(ImportFolderRequest)`, `clone(CloneProjectRequest)`,
`create(CreateProjectRequest)`, `github_owners()`,
`publish(PublishProjectRequest)`, `recent_repos()`,
`remove(&str, delete_files: bool)`, `reorder(Vec<String>)`, `profiles()`,
`create_profile(&str, Option<String>)`, `sync_thoughts(SyncThoughtsRequest)`,
`backend_health(&str)`, `worktree_files(&str)`,
`set_worktree_files(&str, Vec<String>)`, `check_worktree_files(&str)`,
`ai_status()`, `probe_default_thoughts()` (sync),
`run_onboarding(RunOnboardingRequest)`.

`find` returns `Ok(None)` for an unregistered path rather than an error.
`import_folder` is the cheapest useful mutation in the whole facade — filesystem
classification plus two database statements, no CLI and no network — which is
why both headless proofs use it. `remove(path, delete_files: true)` deletes the
registered project directory.

### `LibraryService`

`resolve_path(&str)` (sync), `query(ThoughtQuery)`, `get(&str)`,
`save(&str, String, base_mtime_ms: f64)`, `patch_frontmatter(&str, FmPatch, f64)`,
`create(CreateDocumentRequest)`, `thoughts_git_status(&str)`.

The `base_mtime_ms` argument on `save` and `patch_frontmatter` is the optimistic
concurrency token: a stale value fails the write rather than clobbering it.

### `AgentService`

`list(Option<String>)`, `save(AgentFile)`, `delete(String)`,
`new_file_path(String, String, Option<String>)`, `context_files(Option<String>)`,
`mcp_servers(Option<String>)`, `skills(Option<String>)`,
`export(AgentFile, AgentOverlay)`, `bundle_status()`, `update_bundle()`,
`bundle_versions()`, `repair_bundle()`, `install_bundle_version(Option<String>)`.

The `Option<String>` in most positions is a project path; `None` means the
user-global scope.

### `PromptService`

`list()`, `preview_import(PromptImportRequest)`,
`apply_import(PromptImportDecision)`, `export(PromptExportRequest)`.

Preview-then-apply is two calls on purpose: the preview reports conflicts, the
decision resolves them.

### `WorkstreamService`

`find(&str)`, `list(ListWorkstreamsRequest)`, `list_all()`, `list_archived()`,
`documents(WorkstreamDocumentsRequest)`, `attach_doc(&str, &str, role: &str)`,
`detach_doc(&str, &str)`, `set_base_branch(&str, &str)`,
`set_pr(&str, Option<u32>)`, `adopt_head_branches(Vec<HeadBranchObservation>)`,
`watch(Option<String>)`, `git_status(WorkstreamGitStatusRequest)`,
`pr_status(WorkstreamPrStatusRequest)`, `rename(RenameWorkstreamRequest)`,
`update(UpdateWorkstreamRequest)`, `create(CreateWorkstreamRequest)`,
`archive(ArchiveWorkstreamRequest)`, `restore(RestoreWorkstreamRequest)`,
`purge(PurgeWorkstreamRequest)`, `cleanup_archived(CleanupArchivedRequest)`,
`reprovision(ReprovisionWorkstreamRequest)`.

`find` returns `CoreError::WorkstreamNotFound` rather than `Ok(None)` — unlike
`ProjectService::find`. Create/archive/restore/purge run under a per-project
lock and roll back on failure.

### `AttachmentService`

`import(ImportAttachmentRequest)` and `save(SaveAttachmentRequest)`, both
returning `AttachmentPath` (an alias for `PathBuf`). Both write under the
workstream's `.hyprlayer/attachments/` and reject a path that escapes it. There
is no removal method: dropping an attachment from a draft is a client-side
concern and cannot delete a file.

### `TerminalService`

`open(OpenTerminalRequest)`, `attach(AttachTerminalRequest)` (returns
`Option<TerminalAttachment>` — `None` when the terminal is gone),
`write(WriteTerminalRequest)`, `resize(ResizeTerminalRequest)`,
`close(CloseTerminalRequest)`, `open_cli(OpenCliTerminalRequest)`,
`open_shell(OpenShellTerminalRequest)`, `open_agent(OpenAgentTerminalRequest)`,
`resize_agent(ResizeAgentTerminalRequest)`,
`capture_session(CaptureSessionRequest)`,
`mint_codex_session(MintCodexSessionRequest)`,
`session_exists(SessionExistsRequest)`.

`TerminalAttachment { terminal_id: String, live: mpsc::Receiver<TerminalEvent> }`.

### `ScriptService`

`settings(ScriptSettingsRequest)`, `run(RunScriptRequest)`,
`stop(StopScriptRequest)`, `output(ScriptOutputRequest)`,
`runtime(ScriptRuntimeRequest)`, `running_ids()`,
`run_lifecycle(RunLifecycleRequest)`.

`ScriptOutput` and `LifecycleResult` are aliases for `Option<String>` and
`Option<RunPhase>`; `None` from `run_lifecycle` means no script was configured
for that kind, which is a skip and not a failure.

## Errors

`CoreError` and the `Result<T, E = CoreError>` alias are both re-exported at the
crate root. `CoreError::code()` gives a stable string code and
`CoreError::remedy()` an optional call-to-action; the desktop shell converts
both into its IPC error envelope, and `src-tauri/tests/fixtures/compat/v0_7/app-errors.json`
pins the mapping. A `remedy` of `None` means there is no useful action to offer,
not that one was forgotten.

The lifecycle-specific codes: `runtime_in_use`, `runtime_already_started`,
`runtime_stopped`.

## Models

`hyprlayer_core::models` is `hyprlayer-models` re-exported whole. Its `serde` is
the only normal dependency, with `specta` optional (the desktop shell turns it
on to derive TypeScript bindings). The principal durable values are also
re-exported at the core crate root for convenience: `Harness`, `PermissionMode`,
`Stage`, `ProjectRecord`, `ProjectSource`, `RuntimeEvent`, `ScriptKind`,
`Session`, `SessionEvent`, `SessionLogEntry`, `SessionState`, `Workstream`,
`WorkstreamDoc`, `WorkstreamStatus`, `WorktreeMode`.

`AppSettings` is deliberately **not** a model: the file mixes secrets, provider
configuration, and desktop preferences, so the value and its store stay together
in `hyprlayer_core::settings` (ADR-0008, Type ownership).

## Not supported API

`hyprlayer-core` still declares 30 `pub mod`s. Only the ones named above are
supported. Everything else is implementation detail with **no stability
promise** — provider protocols (`claude`, `codex`, `opencode`), raw stores
(`store`), concrete registries and watchers (`index`, `runtime::script_registry`,
`runtime::terminal_registry`), Git and process implementations (`gitops`,
`process`, `worktree`), the CLI wrapper (`hyprcli`), and driver internals.

They are public only because `hyprlayer-app` still reaches through them for the
desktop carve-outs ADR-0008 enumerates. That is a migration aid, not a contract:
renamed fields and inherent methods under those modules change without notice,
and no pre-refactor path carries a compatibility guarantee.

`runtime::builder::RuntimeBuilder` is `#[doc(hidden)]` and compiled only under
`cfg(test)` or the non-default `test-support` feature. It is a failure-injection
seam for core's own tests and is not part of the facade; a headless embedder
uses `HyprlayerRuntime::open` and, where it needs to replace behavior,
`RuntimeOptions::with_credential_lookup`.

Adding a method to the supported surface requires an update to this document and
a headless integration test.
