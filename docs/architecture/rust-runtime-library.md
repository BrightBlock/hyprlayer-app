# The `hyprlayer-core` runtime library

The supported Rust API for embedding Hyprlayer without a webview. It is the
facade ADR-0008 describes: `hyprlayer-models` (dependency-light contracts) →
`hyprlayer-core` (all application logic, exposed through `HyprlayerRuntime`) →
`hyprlayer-app` (Tauri transport and desktop carve-outs).

`crates/hyprlayer-core/tests/headless_runtime.rs` is a complete embedder that
runs on every `cargo test --workspace` and in the release gate's `cargo nextest run`:
open, start, import, observe an event, shut down twice concurrently, and assert
nothing was written outside the supplied paths. Read it alongside this document — it
is the worked example, and unlike a prose snippet it cannot rot silently.

This document describes the surface **as it exists**, which in several places is
not the surface ADR-0008's outline table or the implementation plan sketched. The
tree is right and this document follows the tree.

## The second consumer

`crates/hyprlayer-server` is the first real embedder besides the desktop, and the
reason this facade had to be a facade (ADR-0009). It opens one runtime, holds the
profile lock, and serves it over a Unix socket — a named pipe on Windows — so the
desktop became an RPC client rather than a host. The same binary is reached
remotely as `ssh <host> hyprlayer-server relay`.

What that consumer needed, and therefore what this surface gained:

- **`RuntimeEnvironment::detect` and `RuntimePaths::detect`** (`env.rs`). A server
  has no Tauri to hand it paths, so the desktop's resolution moved into core. It is
  the one place core may read `dirs::`/`env::var_os` for those paths, and only an
  embedder at construction time may call it — see ADR-0009 on why that does not
  breach ADR-0008's hermeticity contract, whose subject is what core reads *during
  operations*.
- **An environment overlay.** `RuntimeEnvironment` stopped being fixed at `open()`
  and became a base plus a mutable overlay (`set_env_overlay`/`overlay_env`,
  mirrored by `HyprlayerRuntime::set_env_overlay`). A daemonised server has no
  `SSH_AUTH_SOCK`, so each attaching client sends one and the server merges it over
  `process_env` at every child spawn. Nothing else in core reads the overlay.
- **Four accessors removed, and the methods that replaced them.** `SettingsService`
  no longer exposes `binaries()` or `config()`, `SessionService::launch` returns
  `()` rather than an `Arc<dyn AgentDriver>`, and `opencode_server_if_running` is
  `pub(crate)`. A resolved `PathBuf` or a trait object means nothing to a caller in
  another process, so each leak became a method that does the work where the
  runtime is: `SessionService::{send, respond_permission, answer, interrupt, kill,
  set_model, set_effort, is_live}`, `SettingsService::{set_binary_override,
  redetect_binaries, github_username, default_username, effective_config_for}`, and
  the `WorkstreamService` methods that absorbed `commands/workstreams.rs`'s nine
  `require(BinaryKind::Git|Gh)` bodies.
- **`ProjectService::browse`** — a directory listing on the server's filesystem,
  for a client that cannot open a file dialog on a machine it is not running on.

The rule this produces for anything added later: **a facade method may not hand back
a value that only means something inside the runtime's own process.** A path, a file
descriptor, a trait object and a live handle are all the same mistake. If a caller
in another process could not act on it, the work belongs behind the method.

## Constructing a runtime

Everything the runtime reads is supplied: no `dirs::` fallback, no ambient
`HOME`/`XDG`/cwd read. Two deliberate exceptions, both named in ADR-0008's
*Hermetic construction* section rather than hidden here: `process/binaries.rs`
reads `PATH` as a fallback when resolving a binary, and `runtime/terminals.rs`
reads `SHELL` to pick a default shell.

```rust
use hyprlayer_core::{HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths};

let options = RuntimeOptions::new(environment, paths);

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
```

No port is injectable in production; every one is fixed at `open`, and the
test-only `RuntimeBuilder` replaces them. The runtime reads no provider
credentials: each provider CLI resolves its own login, and an explicit API key
from Settings is passed to it as an environment variable.

### What hermeticity does and does not promise

**It promises:** `HyprlayerRuntime::open` and every core operation read and
write only under the supplied `RuntimePaths`, the supplied
`RuntimeEnvironment`, and the project/thoughts/worktree roots read back out of
those supplied files, and, for a project's Claude Code memory, a directory
named by an `autoMemoryDirectory` in `claude_managed_settings_files` or
`claude_config_dir/settings.json`, which `WorkstreamService::project_memory`
reads and `save_project_memory_file` writes. Core does not call
`dirs::home_dir`, does not read `HOME`/`XDG`/`PATH` from the process, and does
not call `std::env::current_dir`.

**It does not promise anything about child processes.** Hermeticity constrains
core's *own* reads, never what a child inherits. PTY terminals, provider CLIs,
and `git` receive the desktop environment on purpose, with only `PATH` swapped
for the augmented one, so a user's shell keeps `SSH_AUTH_SOCK`, locale, and
custom variables. Two specific
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
and a call after shutdown returns `CoreError::RuntimeStopped`. It spawns four
tracked tasks — a startup task (login-shell PATH enrichment, config migration
if needed, thoughts index rescan), the config-file poll loop, the PR watcher,
and the hourly archive sweep. Service methods check the same state: calling one on a stopped runtime
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
`hyprlayer_core::RuntimeEvent`) has these variants, among others:

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
| `DocStateChanged` | `ids: Vec<String>`, `change: DocStateChange` |
| `AccountChanged` | — |
| `PtyAgentChanged` | `terminal_id`, `workstream_id`, `engine_id`, `state: PtyAgentState`, `provider_session_id`, `activity`, `last_reply` (all three `Option<String>`) |

`ThoughtsIndexUpdateKind` is `Updated | Removed | Rescanned`. `DocStateChange` is `Read | Agent`; read state never rides `ThoughtsIndexUpdated`, which clients treat as a write.

## Services

Seventeen handles, each borrowed from the runtime for the duration of one call:

```rust
runtime.settings()   runtime.providers()  runtime.sessions()   runtime.projects()
runtime.library()    runtime.agents()     runtime.prompts()    runtime.workstreams()
runtime.attachments() runtime.terminals() runtime.scripts()    runtime.reviews()
runtime.skills()     runtime.account()    runtime.goals()      runtime.agent_control()
runtime.pty_controls()
```

Each handle's operational request/result types live in that service's module and
are supported alongside it: `hyprlayer_core::runtime::{settings_service,
providers, sessions, projects, library, agents, prompts, workstreams,
attachments, terminals, scripts, reviews, skills, account, goals, agent_control,
agent_control::pty_controls}`. The handles
themselves are also re-exported at the crate root.

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
`codex_login_with_api_key(String)`, `codex_logout()`,
`ollama_status()`, `ollama_set_endpoint(Option<String>)`,
`ollama_set_selected_models(Vec<String>)`, `ollama_set_default_model(Option<String>)`,
`generate_title_claude(&str, &str)`, `generate_title_codex(&str, &str)`,
`generate_title_opencode(&str, &str)`, `opencode_server()`,
`opencode_server_if_running()`, `invalidate_opencode_server()`,
`opencode_catalog(bool)`, `opencode_auth_list()`,
`opencode_set_provider_key(&str, &str)`, `opencode_remove_provider(&str)`,
`opencode_set_default_model(Option<String>)`.

Network calls go through the internal `ProviderHttp` port, which GETs and
form-POSTs to `String`; the desktop shell has no HTTP client of its own.

The OpenCode key and default-model methods edit `RuntimePaths::opencode_auth_file`
and `opencode_config_file`. Setting or removing a key invalidates the cached
`opencode serve`, which reads `auth.json` only when it spawns. A malformed
`auth.json` fails the call instead of being overwritten.

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
| `prepare_start_with(StartStageSessionRequest, SessionLineage)` | `StartedStageSession` (records `origin` and `parent_session_id`) |
| `launch(LaunchSessionParams)` | `Arc<dyn AgentDriver>` |
| `launch_and_converse(LaunchAndConverseParams, Option<ClientRef>)` | `()` |
| `rewind(RewindSessionRequest)` | `RewindSessionOutcome` |
| `kill_and_await_exit(&str)` | `()` (not a `Result`) |
| `live_driver(&str)` | `Option<Arc<dyn AgentDriver>>` (not a `Result`) |
| `permission_mode(&str)` | `Option<PermissionMode>` (not a `Result`) |
| `op_lock(&str)` | `Arc<tokio::sync::Mutex<()>>` (not a `Result`) |
| `send(&str, &str, &[String], Option<&str>, Option<&str>, SendContext)` | `()` |
| `list_turns(&str, LogCursor, u32, Option<&str>)` | `SessionTurns` |
| `revs(&[String], Option<&str>)` | `Vec<SessionRev>` |

`SessionAttachment { replay: Vec<SessionLogEntry>, live: LiveReceiver, rev: u64 }`
delivers durable replay plus a bounded live stream (512 slots plus a reserved
terminal slot); a consumer too slow for it is detached with a lag cursor and
recovers history through `log_after`. `live.recv()` yields events and
`live.recv_item()` yields `SessionStreamItem`s; `rev` is the session's rev at
attach.

Sending a message, answering a question, and resolving a permission go through
`send`, `respond_permission` and `answer` on the service. Only `send` takes the
session's `op_lock`; it also checks `SendContext.base_rev` against other
authors' turns and refuses a stale one with `stale_transcript`. Sent mid-turn,
the message reaches the harness at once and the driver echoes its `UserMessage`
when the harness takes it in at its next step; `cancel_queued_message` withdraws
one still waiting.

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

`resolve_path(&str)` (sync), `query(ThoughtQuery)`, `counts(Option<String>, ThoughtQuery)`,
`get(&str)`, `save(&str, String, base_mtime_ms: f64)`,
`patch_frontmatter(&str, FmPatch, f64)`, `save_parts(&str, String, FrontmatterEdit, f64)`,
`resolve_related(&str, Vec<String>)`, `create(CreateDocumentRequest)`,
`mark_read(&str)`, `mark_unread(&str)`, `thoughts_git_status(&str)`.

The `base_mtime_ms` argument on `save`, `patch_frontmatter` and `save_parts` is the
optimistic concurrency token: a stale value fails the write rather than clobbering
it. `save_parts` writes nothing when the composed bytes equal the file, and a
malformed block changes only through `FrontmatterEdit::Raw`. The `doc_round_trip`
E2E scenario is its headless integration test; it drives the facade through the
real server without a webview.

### `AgentService`

`list(Option<String>)`, `save(AgentFile)`, `delete(String)`,
`new_file_path(String, String, Option<String>)`, `context_files(Option<String>)`,
`mcp_servers(Option<String>)`, `skills(Option<String>)`,
`harness_counts()`, `export(AgentFile, AgentOverlay)`, `bundle_status()`, `update_bundle()`,
`bundle_versions()`, `repair_bundle()`, `install_bundle_version(Option<String>)`.

The `Option<String>` in most positions is a project path; `None` means the
user-global scope. `harness_counts()` counts each `ai status` platform's `agents/`
files (`.md`; `.toml` for codex) and `skills/*/SKILL.md` dirs on the server host;
`agent_files` is its headless test.

### `SkillService`

`list(Option<String>)`, `read_file(Option<String>, String)`,
`read_asset(Option<String>, String, String)`,
`preview_install(Option<String>, SkillSourceInput)`,
`install(Option<String>, String, SkillScope, Vec<SkillTarget>)`,
`remove(Option<String>, SkillScope, String)`, `search(String)`,
`leaderboard(SkillsShView, u32)`,
`lock_status(Option<String>)`, `check_updates(Option<String>)`,
`preview_update(Option<String>, SkillScope, String)`,
`restore(Option<String>, SkillScope)`, `list_skillfish_candidates(Option<String>)`.

`None` is user-global. `list` returns one row per real skill folder, carrying
`targets` (`claude`, `agents`) and `installed`; links to one folder from
`.claude/skills` and `.agents/skills` collapse. The Claude user root follows
`CLAUDE_CONFIG_DIR`. `read_file` refuses a path outside the skill roots, or not a
`SKILL.md`, with `invalid_path`. `read_asset` reads a file inside a listed skill's folder
and refuses anything whose canonical path leaves it; a binary or over-1 MiB file comes back
with `text: None`. `install` with no targets refuses with `skill_targets_empty`. `leaderboard` reads one 200-row page of the
skills.sh All time, Trending or Hot list from the `/api/skills/{view}/{page}` JSON the
site pages through itself; rows whose source isn't `owner/repo` drop out, as in `search`. `preview_install` stages a GitHub skill
and returns its files, trust flags and conflicts; `install` commits the staged files
after re-verifying their hash, and consumes the token. `name` on `remove` is the folder
name. `search` queries skills.sh (a query under 2 characters returns `[]` with no request);
`preview_install` also takes a `SkillsSh` pick. Writes serialize per scope, and refusals
use the `skill_*` codes. `lock_status` is local; `check_updates` runs one `ls-remote` per
repo, and only a branch ref can report an update (tags and SHAs are pinned).
`preview_update` returns a preview with a change summary whose token installs as a replace
(the lock's scope and targets win). `restore` refetches missing stores and re-creates
missing links; changed content comes back as `needsApproval` with a preview. Candidates
come from a read-only `skillfish.json`. The
`skill_sources` E2E scenario is its headless integration test.

### `AccountService`

`status()`, `finish_sign_in(&str, &str)` and `sign_out()` return `AccountStatus`
(`signedOut`, or `signedIn` with `name` and `email`); `start_sign_in(u16)` returns
`SignInStart { state, logout_url, authorize_url }` for the browser leg.

The port is a loopback the caller bound with `account::loopback::Loopback` on the
machine running the browser; the PKCE verifier stays in the runtime (ADR-0011).
`finish_sign_in` takes the pending attempt only on an exact `state`, once: a mismatch
is `account_sign_in_stale` and leaves the attempt in place. Success writes
`<app_config_dir>/account.json` (0600 on unix) and sends `AccountChanged`, as does a
`sign_out` that removed a file. A file from the other Cognito pool reads as signed out
and is left alone. `status()` may refresh a stale token and re-check the session, but
never waits behind a refresh in flight: it answers from the last status then. A stored
session is also refreshed at runtime start and every hour; only `invalid_grant` signs out, and
`sign_out` revokes the refresh token best-effort (ADR-0011). The `account_sign_in` E2E
scenario is its headless integration test.

### `PromptService`

`list()`, `preview_import(PromptImportRequest)`,
`apply_import(PromptImportDecision)`, `export(PromptExportRequest)`.

Preview-then-apply is two calls on purpose: the preview reports conflicts, the
decision resolves them.

### `WorkstreamService`

`find(&str)`, `list(ListWorkstreamsRequest)`, `list_all()`, `list_archived()`,
`list_archived_details()`, `refresh_archived()`, `keep_archived(&str)`,
`purge_archived(Vec<String>)`, `sweep_expired_archives()`,
`documents(WorkstreamDocumentsRequest)`, `attach_doc(&str, &str, role: &str)`,
`detach_doc(&str, &str)`, `set_base_branch(&str, &str)`,
`set_pr(&str, Option<u32>)`, `adopt_head_branches(Vec<HeadBranchObservation>)`,
`watch(Option<String>)`, `git_status(WorkstreamGitStatusRequest)`,
`pr_status(WorkstreamPrStatusRequest)`, `rename(RenameWorkstreamRequest)`,
`update(UpdateWorkstreamRequest)`, `create(CreateWorkstreamRequest)`,
`archive(ArchiveWorkstreamRequest)`, `restore(RestoreWorkstreamRequest)`,
`purge(PurgeWorkstreamRequest)`, `cleanup_archived(CleanupArchivedRequest)`,
`reprovision(ReprovisionWorkstreamRequest)`, `project_memory(&str)`,
`save_project_memory_file(&str, &str, String, base_mtime_ms: f64)`.

`find` returns `CoreError::WorkstreamNotFound` rather than `Ok(None)` — unlike
`ProjectService::find`. Create/archive/restore/purge run under a per-project
lock and roll back on failure.

Archives are purged 30 days after archiving unless kept (`workstream_archives`).
A rebuilt record is never due sooner than seven days out, the length of the
client's warning. `list_archived_details` reads only the database: sizes and
dirty/unpushed counts are scanned once at archive time, and `refresh_archived`
scans what never was and asks `gh` about unknown or open PRs, each at most every
15 minutes. The sweep purges like `purge` but never deletes a remote branch and
deletes the local one only with `git branch -d`, so it can lose uncommitted
files, never commits. `restore` with a `target` rewrites the workstream's path
and its sessions' `cwd` in the unarchive transaction, then copies each Claude
transcript to the new path's project key for `--resume`. The `archive_retention`
E2E scenario is their headless test.

`project_memory` resolves the workstream's project to Claude Code's auto-memory
directory. That is `claude_config_dir/projects/<key>/memory`, or an
`autoMemoryDirectory` read from `claude_managed_settings_files` and then
`claude_config_dir/settings.json`. It lists the directory's `*.md` files with
`MEMORY.md` first and returns an empty listing when the directory is missing.

The key encoding and the index bounds are ported from Claude Code. They were
verified against 2.1.252, 2.1.258, 2.1.270, 2.1.273 and 2.1.274, whose builds
carry the same logic but not byte-identical code. There is no runtime version
gate.

`keyCheck` compares the key against where the workstream's latest Claude
session wrote its transcript. It is `mismatch`, with no files, when a Claude
Code release has moved its project directories.

`save_project_memory_file` takes a file name, not a path. It only overwrites an
existing regular `*.md` file in the project's memory directory and refuses a
symlink, and `base_mtime_ms` is the same optimistic-concurrency token as
`LibraryService::save`.

### `ReviewService`

`get(&str)`, `set_viewed(SetReviewFileViewedRequest)`,
`add_comment(AddReviewCommentRequest)`, `edit_comment(&str, &str, &str)`,
`delete_comment(&str, &str)`, `resolve_comment(&str, &str, bool)`,
`resolve_note(&str, &str, bool)`, `submit(SubmitReviewRequest)`,
`mark_delivered(&str, u32)`. The first `&str` is always the workstream id.

One review per workstream, created by the first call that needs it. `get` diffs
the branch against its merge-base with the base branch, uncommitted work
included. A file reads as viewed while its content hash still matches the one it
was viewed at. Each comment is re-anchored on read: its line moves with the text
it was left on, searched within 50 lines of where it was, and `current_line` is
`None` once that text is gone. The base side is read from the merge-base, at a
rename's old path.

A reply is placed on its parent's line, whatever the request says. Edit and
delete refuse a comment that is sending or sent (`review_comment_locked`), and a
comment or note id from another workstream's review is not found. Every mutation
sends `RuntimeEvent::ReviewChanged`; `get` sends nothing, though it may create
the review.

`submit` records a round. Comment and Approve save the pending comments on it.
Request changes moves every unsent comment to sending and returns the message
(`review/message.rs`) for the caller to deliver; `mark_delivered` then marks them
sent. Until then the round refuses the next submit (`review_round_undelivered`).
After a delivered round, `changed_since_round` lists the files whose content
differs from the round's snapshot, and a sent comment in one of them is
`addressed` once its line or the lines around it changed. The `review_lifecycle`
E2E scenario is their headless test.

### `GoalService`

`start(StartGoalRequest, Option<ClientRef>)`, `start_with(StartGoalRequest,
Option<ClientRef>, SessionLineage)` (what a `NewSession` target's row records; `start` uses
origin `goal`), `pause(&str)`, `resume(&str)`,
`stop(&str)`, `update(&str, GoalPatch)`, `get(&str)` and
`list(Option<&str>, active_only: bool)`. The `&str` is a goal id; `list` filters by an
optional workstream id. The first five return the `Goal`, `get` a `GoalDetail` (the
goal and its iterations, oldest first), and `list` the goals newest first.

A goal is a condition, a work prompt, checks and limits attached to one session. The
server sends an iteration into that session, runs the checks, and either sends the next
iteration with the failed checks as feedback or ends the goal. `start` takes a
`GoalTarget`: an existing `Session`, or a `NewSession`, which first launches a General,
Research or Implement session. A session has one running or paused goal at most
(`goal_active`); a partial unique index decides a race between two starts. The caller's
`ClientRef` signs every iteration's message and stays on it after that client leaves.

A spec the server cannot run is refused as `goal_invalid` before anything is written:
it needs a Command check (the other kinds are refused), a command and a timeout of a
second or more, a directory (if any) inside the worktree, 1 to 200 iterations, 60 seconds to 48 hours of active time, and a spend
limit (dollars on Claude and OpenCode, tokens on Codex). Spend is recorded on the goal
but not yet enforced.

Each running goal has one driver task (`runtime/sessions/goal_loop.rs`), spawned by
`start` and `resume`; `shutdown` stops it before it kills any harness. An iteration is
one turn, sent through `SessionService::send_app_turn` under the prompt name
`hyprlayer:goal`. It ends at the turn's own echoed message, a `TurnDone` and two quiet
seconds, so a Stop-hook loop inside the turn is still one iteration. The runner then
runs the Command checks in the workstream's worktree, or the directory under it a check
names (a missing one fails the check by name), each in its own process group with
bounded output, and decides. The goal is `met` when every check passes. It is
exhausted at the iteration or active-time limit, and failed as `stalled` after three
iterations in a row that leave the same diff, HEAD and failure. It pauses when a line of
the agent's last text starts `NEEDS-HUMAN:`, after two failed turns, when the harness
exits unexpectedly, or when asked. The goal is stored in `goals`, each iteration with
its check results in `goal_iterations`.

`pause` waits for the iteration and its checks (step `pausing`). `stop` interrupts the
turn (step `stopping`) and kills the harness if it ignores the interrupt for ten
seconds. `resume` runs a paused goal again, and an exhausted one whose limit `update`
has raised. `update` changes the condition, work prompt, checks or limits at the
runner's next boundary and leaves the running iteration alone. Interrupting the session
pauses its running goal, killing the session ends the goal as `session_ended`, and
archiving the workstream ends it as `workstream_archived`. Archiving or deleting a
session with a live goal is refused as `goal_active`. Opening the runtime reaps the
harness a previous run left behind for a running or paused goal, then resumes the running
ones once the first client's environment overlay arrives (at most 30 s): the cut-off
iteration closes as `cut_off`, or the checks re-run if the goal went down checking. More
than three restarts without a finished iteration pause the goal as `restart_loop`. The
other codes are `goal_not_found`, `goal_invalid` and `goal_finished`.

Every status, step or iteration change sends `RuntimeEvent::GoalChanged`. The server
forwards it only to connections that announced `GoalChanged`, watching the session or
not; a `subscribe()` receiver hears every one. While a goal is running or paused its
session carries a `GoalBrief` in `Session.goal`; an ended goal is read with `get`. The
`goal_met`, `goal_iteration_limit` and `goal_user_stop` E2E scenarios and the
`goal_runtime` suite are its headless tests.

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

`launch_agent(LaunchAgentRequest)` returns `(TerminalAttachment, PtyLaunched)`. It
resolves the engine from `settings.multiplexer.engines` (else the builtin), builds the
argv itself, and gives the PTY the agent's `HYPRLAYER_*` env unless agent control is
`off`. `claude-code`, and `codex` 0.164 or newer, also get hooks that report through
`hyprlayer-server hook`, and reach the `hyprlayer` MCP server: claude's `--mcp-config`
and `--settings` files live under `<app data>/agent-control/launch/<terminal id>/` until
the PTY exits. An engine whose
args carry their own `--settings` launches unhooked, and so does any other engine. An
unhooked codex with no session id is minted with `codex exec` first, and
`PtyLaunched.provider_session_id` carries the id. `list_pty_agents(Option<&str>)`
returns the hooked agents, plus unhooked ones an agent spawned (`PtyAgent.spawned_by`,
`hooked: false`), optionally for one workstream; each state change sends
`RuntimeEvent::PtyAgentChanged`, which the server forwards only to connections that
announced it. The `pty_hook_signals` E2E scenario is their headless test.
`LaunchAgentRequest.lineage` is `None` for the desktop's launches; an agent's spawn sets
`PtyLineage { parent, depth, mode }`, which becomes the PTY's identity and an in-memory
link to its parent until it exits.

### `AgentControlService`

What an agent spawned by the runtime may ask of it. `identity_env(AgentIdentity)` mints a
per-spawn token and returns the `HYPRLAYER_*` env plus the `TokenGuard` that revokes it, or
`None` with no endpoint or at `off`; `session_lineage(&Session)` gives a session's parent
and depth (one per agent hop). `identify(&str) -> AgentIdentity` looks a token up (`agent_control_denied` once its
spawn has exited); `level_of(&AgentIdentity)` resolves the level per call from the
session's live permission mode, so a bypass caller is `full` unless the setting is `off`;
`whoami(&AgentIdentity) -> AgentWhoami`. `notify` and `request_attention` send
`RuntimeEvent::UiRequested { Notify }` for the caller's workstream, which the server
forwards only to connections that announced it. `check_workstream`, `check_session`,
`check_goal`, `check_doc` (a write, attaching included, takes only a doc mapped to the
caller's project; a doc attached to its workstream only reads), `check_project`,
`check_terminal` (a PTY agent the server launched, never a plain shell or script) and
`check_own_goal` (the caller's or a descendant's goal) refuse out-of-scope ids as
`agent_control_denied`, and
`project_scope` returns what a list verb keeps. `report_signal(&str, &AgentSignal)` folds
a hook's signal into its PTY agent, and titles it from its first prompt;
`report_status(&str, AgentStatusReport)` applies a claude statusLine run.
`model_switch_pending(&str) -> bool` tells a claude's `PreModelSwitch` hook whether the
status bar is switching it right now.

### `PtyControlService`

The status bar's changes to a hooked claude PTY, made session-only by driving its TUI
against a terminal-emulated copy of its screen. `set_model(&str, &str) -> PtyControlOutcome`
picks the model's row in `/model` by the way round that never lands on "More models" and
presses `s`, confirmed by the picker's `PostModelSwitch`, the statusLine or claude's own note;
`Relaunch` asks the desktop to restart an idle agent with `--resume`, `--model` and the
session's `--permission-mode` and `--effort` when claude never confirmed it. Any failure with a
picker on screen sends it one Esc. `set_effort(&str, &str)` slides `/effort`, and
`set_permission_mode(&str, &str)` takes claude's own mode key (`auto` included) and reaches
it with `/plan` or shift+tab, confirmed by the footer. Each holds the terminal's input lock,
which an agent's message also takes, and refuses an agent that is asking its user something
or not showing its input box (`pty_control_failed`). It never sends a digit, an Enter in a
picker or a typed `/model <x>`: each saves a claude default, the model one on the account.

Mutating verbs need `require_full(&AgentIdentity)`: level `full`, not report-only, and not a
codex session or PTY outside bypass (codex approves the `hyprlayer` tools unasked).
`check_goal_checks(&AgentIdentity, &[GoalCheck])` refuses a command check, a shell on the
server, from a caller outside bypass; the server runs it for `start_goal` and `update_goal`.
`check_descendant(&AgentIdentity, &str, or_self: bool)` refuses a session or terminal the
caller did not spawn. Lineage no row can hold (anything a terminal spawned, and terminals a
session spawned) lives in memory until the child's PTY exits or its session row is deleted;
a session whose terminal parent is gone counts as `max_depth` deep. `check_spawn(&AgentIdentity,
&str, Stage, Option<Harness>, Option<PermissionMode>) -> SpawnCheck` (it also takes the
attached docs, each of which must pass the read scope check, or the write one for an
Implement or Validate start, which activates the plans it attaches) runs every spawn
refusal before anything is written: level, project, `max_depth`
and `max_live_children` (`agent_limit_reached`), a permission mode above the caller's
own (Plan < Default < AcceptEdits < Bypass; unset means the caller's), and, outside bypass, a
stage whose pre-approved tools the caller's own stage lacks
(`pipeline::pre_approves_within`; a terminal counts as General). The `SpawnCheck` holds the
caller's spawn lock until it is dropped, after the child is live.
`spawn_session(&AgentIdentity, SpawnSessionRequest) -> Session` starts a composer session
with origin `agent` and no stream: its first turn is sent before it returns, headed
`[from agent "<caller>" · <caller id>]` and attributed to `agent:<caller id>`, and a failed
launch leaves no row. It sends `UiRequested { OpenSessionTab { focus: false } }`.
`wait_session(&AgentIdentity, &str, WaitUntil, u32) -> WaitOutcome` returns once the
session is idle, needs a person, or exits (each `until` also stops at the later ones), when
an idle session has not moved for 5 s (`stalled`), or at the timeout (capped at 600 s).
`send_message(&AgentIdentity, &str, &str)` sends a headed, attributed turn and counts a
hop once it is delivered; past `max_agent_hops` without a human turn it refuses with
`agent_limit_reached` and notifies the user once. A target running in a higher permission
mode than the caller is refused unless the caller spawned it. `note_human_turn(&str)` resets a target's hops; the server calls
it for a desktop's `send_user_message`, and `TerminalService::write` for typed input.
The `agent_spawn_and_message` E2E scenario is the headless test of these.

PTY agents: `spawn_pty_agent(&AgentIdentity, SpawnPtyAgentRequest) -> PtyAgent` refuses an
engine id that is neither configured nor builtin, runs the
spawn checks (bypass only from a bypass caller), launches through `launch_agent` at 120×32,
sends `UiRequested { OpenTerminalTab }`, and pastes an optional first prompt once the TUI is
ready (its first idle when hooked, 3 s of quiet otherwise). `send_pty_message(&AgentIdentity,
&str, &str)` pastes a headed message into a descendant PTY agent as a bracketed paste, then
Enter 100 ms later; it counts a hop, never a human turn, and refuses while the target is
asking its user something. `read_terminal(&AgentIdentity, &str, u32) -> String` returns the
last lines (at most 500) of a PTY agent's scrollback with escape sequences stripped.
`wait_pty_agent(&AgentIdentity, &str, WaitUntil, u32) -> WaitOutcome` is `wait_session`
over a hooked agent's folded state; an unhooked one is refused. `interrupt_pty_agent` writes
Esc into a descendant PTY agent and `kill_pty_agent` closes its terminal. When an
agent-spawned PTY starts asking (permission or question), the registry sends
`UiRequested { Notify { terminal_id } }`, since no desktop may have its tab shown. `start_goal(&AgentIdentity,
StartGoalRequest) -> Goal` runs a goal on the caller or a descendant, or on a new session
under the spawn checks with origin `agent`. `reveal(&AgentIdentity, RevealTarget)` sends
`UiRequested { Reveal }` for the caller's workstream. The `agent_breadth` E2E scenario is
the headless test of these.

The server decides which commands an agent connection may call (`agent_access.rs`); this
service only resolves identity, level and scope. The `agent_control_read` E2E scenario is
its headless test.

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

`hyprlayer-core` still declares 31 `pub mod`s. Only the ones named above are
supported. Everything else is implementation detail with **no stability
promise** — provider protocols (`claude`, `codex`, `opencode`), raw stores
(`store`), concrete registries and watchers (`index`, `runtime::script_registry`,
`runtime::terminal_registry`), Git and process implementations (`gitops`,
`process`, `worktree`), the CLI wrapper (`hyprcli`), log scrubbing (`redact`), and
driver internals.

They are public only because `hyprlayer-app` still reaches through them for the
desktop carve-outs ADR-0008 enumerates. That is a migration aid, not a contract:
renamed fields and inherent methods under those modules change without notice,
and no pre-refactor path carries a compatibility guarantee.

`runtime::builder::RuntimeBuilder` is `#[doc(hidden)]` and compiled only under
`cfg(test)` or the non-default `test-support` feature. It is a failure-injection
seam for core's own tests and is not part of the facade; a headless embedder
uses `HyprlayerRuntime::open`.

Adding a method to the supported surface requires an update to this document and
a headless integration test.
