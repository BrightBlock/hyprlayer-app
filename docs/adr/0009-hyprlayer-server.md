# ADR 0009: The runtime lives in a `hyprlayer-server` process; the desktop is a client

**Status:** accepted · 2026-09-03 · amended 2026-09-18 (the Keychain lookup is
deleted — the runtime reads no provider credentials) · amended 2026-09-19 (OpenCode
config I/O moves to the server) · amended 2026-09-19 (the server writes its own log
files; a spawned server's stderr goes to a file, not a pipe) · amended 2026-09-19
(the server is versioned and released apart from the app; the app sets a minimum) ·
amended 2026-09-26 (the environment overlay merges per key and validates the agent
socket) · amended 2026-09-30 (connections announce a display-only client identity;
transcript-sync events are targeted)

## Decision

The whole `HyprlayerRuntime` moves out of the Tauri process into a supervised child
process, `hyprlayer-server`, that the desktop spawns if none is listening and talks to
over a Unix socket (a named pipe on Windows). The same binary runs standalone on any
machine you own, reached over `ssh <host> … hyprlayer-server relay`. Sessions, terminals,
and scripts survive quitting the app. ADR-0008's graph gains one node:

*(Amended 2026-09-16 — the elision in that command is load-bearing. Until
`thoughts/shared/plans/2026-09-09-hyprlayer-server-remote-host-ui.md`'s Phase 7 the client
ran the bare name and took whatever the host's `PATH` resolved; it now invokes a
version-keyed absolute path, `~/.hyprlayer/server/<version>/hyprlayer-server relay`, and
installs that binary itself when it is absent. `PATH` is consulted nowhere on the remote
side. A packaged `/usr/bin/hyprlayer-server` still serves headless and manual operation and
is what a systemd unit runs — the two paths coexist and never collide.)*

```text
hyprlayer-models
        ↓
hyprlayer-core
  HyprlayerRuntime facade
        ↓
hyprlayer-server
  one runtime, the profile lock, the socket
        ↓
hyprlayer-app
  RPC client + desktop integration
```

The phases are in `thoughts/shared/plans/2026-09-03-hyprlayer-server.md`. This ADR
records the decisions that outlive them.

### What this supersedes in ADR-0008

Exactly one bullet under ADR-0008's "What we're not doing": *"No remote daemon, broad
`Host`, or remote-terminal abstraction."*

- **A remote daemon: yes.** A separate `hyprlayer-server` process hosts the one
  runtime, reached over a socket locally and SSH remotely.
- **A broad `Host` abstraction: still no.** There is no local/remote branch to
  abstract, because the shell is always a client.
- **A remote-terminal abstraction: still no.** Terminals are not abstracted; they are
  simply on the server.

Everything else in ADR-0008 stands: no separate core repository, no database
replacement, no `ProjectKey` migration, and no `hyprlayer-runtime` facade crate — the
server is a second consumer that strengthens the case for extracting one; it does not
do it.

### The server has its own version; the app sets a minimum

*(Added 2026-09-19.)* Until v0.8.2 the server took the workspace version, so every
app release was also a server release: the remote install directory was keyed by the
app's version, which blocked a remote host until the new binary was installed, and
"outdated" meant a different SHA-256, which every rebuild produces. Every app update
therefore asked every user to restart their server and lose its sessions.

- `crates/hyprlayer-server/Cargo.toml` carries its own `version`
  (`hyprlayer_server::VERSION`), sent in `Hello` as `version`. Servers up to 0.8.2 send
  none and are read as 0.8.2.
- The app declares `MIN_SERVER_VERSION` (`src-tauri/src/server_version.rs`). A server
  below it is replaced locally, the way a protocol mismatch is, and blocks a remote
  connection with `remote_server_outdated` until the user updates it. A server at or
  above it but older than the one the app ships is an optional update. Bytes are
  compared only in debug builds.
- A remote relay runs this app's version if it is installed, else the newest installed
  version at or above the minimum, so an app update never blocks on an install.
- **Bump rule.** `PROTOCOL_VERSION` covers the frame format and envelope. Any change
  under `crates/hyprlayer-{server,core,models,proto}` bumps the server version, and an
  app that depends on that change raises `MIN_SERVER_VERSION` to it. `release.sh`
  refuses a release whose server code changed since the last `server-v<version>` tag
  without a bump.
- **Publishing.** A new server version is tagged `server-v<version>` beside the app
  tag, and `release-local.sh` publishes its binaries, Homebrew formula, and
  `hyprlayer-server-bin` AUR package under that tag, requiring every platform.
  An app release with an unchanged server publishes none of them; the bundled binary
  is still rebuilt, at the same version.

### One server, one profile, one user

`ProfileLock` permits one runtime per `app_data_dir`, so one server serves one
profile: one `settings.json`, one `hyprlayer.db`, one session set. A connection may
announce a client identity — a stable client id, a device name, and the event kinds it
can decode (`announce_client`). It is display-only: it attributes user turns, drives
presence, and targets the transcript-sync events, and it grants no authority — every
attached client still sees the same state and can act on all of it. Core never reads
it; the server hands it to core as data. "Multi-client" means your laptop and your
desktop attached to your server; a second user means a second server on a second
profile.

Three consequences follow, accepted rather than designed around:

- **`set_foreground` is last-writer-wins.** It is a single `AtomicBool`
  (`crates/hyprlayer-core/src/runtime/mod.rs`) feeding the PR watcher's cadence, so a
  backgrounded client throttles polling for a foregrounded one.
- **The environment overlay is one overlay, merged per key.** Each attach overwrites
  only the keys it sends. A key is never cleared, so a departed client's value stays
  until another sends a valid one — the same staleness the opencode section already
  accepts. `SSH_AUTH_SOCK` is dropped unless it names a socket on the server, and a
  remote client sends neither `SSH_AUTH_SOCK` nor `DISPLAY`, so a laptop attaching
  over SSH no longer replaces a working agent.
- **Some events are targeted.** `SessionRevised`, `SessionReset` and `SessionViewers`
  reach only connections that announced them and are watching that session, because a
  released desktop tears its connection down on an event kind it cannot decode.

### The environment overlay: the exception to freeze-at-construction

`RuntimeEnvironment` captures `process_env` once, at construction
(`crates/hyprlayer-core/src/env.rs`). A server started by `systemctl --user` or
launchd has no `SSH_AUTH_SOCK`, so every `git fetch`/`push` and every `gh` call would
fail with exactly the failure ADR-0005 exists to prevent. So each attach carries the
client's `SSH_AUTH_SOCK`, `DISPLAY`, `TERM`, `LANG`, and `LC_*`, and the server merges
them over `process_env` at every child spawn (plan Phase 4 §7) — terminals, harness
drivers, lifecycle and Run scripts, and every `git`/`gh` shell-out. The one asymmetry is
opencode, recorded below. `RuntimeEnvironment`
therefore stops being a value fixed at `open()` and becomes a base plus a mutable
overlay. Nothing else in core reads the overlay; the exception is deliberate and
singular. Remotely, the value the client sends is a path on the *client*, so a remote
server needs its own agent access (`ssh -A`, or a key on the server).

#### opencode takes the overlay of whoever started it

Every other spawn reads the overlay at spawn time, so each key takes its newest valid
value. opencode cannot: one `opencode serve` is started lazily and cached for the
profile's lifetime (`opencode/runtime.rs`, `OpenCodeManager::ensure`), and its sessions
are HTTP calls into that one process — there is no per-session child to hand an
environment to. The overlay is therefore baked into `OpenCodeServerConfig` at spawn,
and **first attach wins**:
a server outliving the client that started it keeps that client's `SSH_AUTH_SOCK`.

Invalidating and respawning when the overlay changes was considered and rejected. A
reconnect almost always brings a new `SSH_AUTH_SOCK` — a new SSH session means a new
agent socket path — so that rule would kill the opencode server on every client
relaunch, destroying live sessions to fix a variable. Surviving a client relaunch is
the whole point of the server (Phase 4); trading it away here would be the larger bug.

What makes the asymmetry tolerable is what the overlay actually carries:
`SSH_AUTH_SOCK`, `DISPLAY`, `TERM`, `LANG`, `LC_*`. No secrets, no tokens. A stale
`SSH_AUTH_SOCK` fails the same way an absent one does — `ssh` cannot reach either
agent, so a non-interactive push fails rather than succeeding wrongly — which makes
first-attach-wins strictly better than the no-overlay behaviour it replaced, never
worse. The remaining cost is a confusing failure after a client swap, not a silent
one. Revisit only if the overlay ever grows a value where stale and absent differ.

### `RuntimePaths::detect` and hermeticity

ADR-0008's hermeticity contract says core operations do not read `dirs::`, `HOME`,
`XDG`, or `PATH`. The server has no Tauri to hand it `RuntimePaths`, so
`RuntimePaths::detect` and `RuntimeEnvironment::detect` (plan Phase 2 §6) lift the
desktop's resolution into `env.rs`. That is the one place core may read
`dirs::`/`env::var_os` for those paths, and it is called only by an embedder at
construction time, never by an operation. The contract's subject is what core reads
*during operations*; how an embedder builds the value it passes to `open` is outside
it. A hermeticity audit that flags `detect` is reading the rule too broadly.

`app_config_dir` and `app_data_dir` come from Tauri's identifier-derived paths and
cannot be lifted, so `detect` takes them as arguments: the desktop keeps passing
Tauri's answer and the server derives its own from `dirs::` with the same identifier.
Getting them wrong opens a different profile than the desktop did, which is the single
most damaging mistake this design makes available.

### Carve-outs that move to the server

Two of ADR-0008's Tauri carve-outs no longer describe the machine that matters:

- **OpenCode and Hyprlayer CLI installer/self-maintenance flows.** They install the
  binaries the runtime spawns, and the runtime is on the server; installing a CLI on
  the laptop does not help a server on another machine. Moving them deletes the
  shell's need for `augmented_path()`, so this is less code, not more.
- **macOS Keychain retrieval.** *(Deleted 2026-09-18, with `CredentialLookup` and
  `claude_model_windows`, its only consumer: the context-window size now comes from
  the `contextWindow` field `claude` reports in each turn's `result.modelUsage`. The
  user-agent packaging stands — the spawned `claude` still reads the login keychain.)*
  `KeychainCredentialLookup`
  (`src-tauri/src/commands/models.rs`) reads the credential the runtime consumes, so it
  belongs where the runtime is. `security find-generic-password` needs an unlocked
  login keychain, which a launchd *user agent* has and a system daemon does not — so
  the macOS packaging is a user agent under `~/Library/LaunchAgents`, never a
  `LaunchDaemon`. A system-daemon deployment would need the credential pushed from the
  client instead; none is planned.

### Carve-outs that stay client-side and gain a remote-disabled state

Editor discovery and launching (`src-tauri/src/editors.rs`), `reveal_doc`
(`src-tauri/src/commands/library.rs`), sound playback and Full Disk Access
(`src-tauri/src/commands/system.rs`), Omarchy theme polling and the updater
(`src-tauri/src/lib.rs`), and all window/menu/webview chrome. They act on the client's
machine. Against a local server they work unchanged; against a remote server they
return `CoreError::RemoteUnsupported { action, host }` rather than acting on a path
that does not exist where the user is looking (plan Phase 5).

A *path-taking* command is not in that set by virtue of taking a path. Six
`remote_unsupported` guards remain, and each names a reason of its own. Four are
permanent: `import_attachment` (`src-tauri/src/commands/attachments.rs`), whose bytes
have to cross the wire — `save_attachment` is the byte-carrying form; `reveal_doc` and
`detect_editors` (`src-tauri/src/commands/system.rs`), which act on the client's own
machine; and `set_binary_override` with `Some(path)` (same file), a pick-only flow with
no text field. The remaining two, `preview_prompt_import` and `export_prompts`
(`src-tauri/src/commands/prompts.rs`), survive until the plan's Phase 6 gives them
byte-carrying forms.

*(Amended 2026-09-16 —
`thoughts/shared/plans/2026-09-09-hyprlayer-server-remote-host-ui.md`'s Phase 5 took
three guards out, so this section no longer reads as though every path-taking command
is disabled remotely. `inspect_repo_path`, `import_project`, and `import_local_folder`
(`src-tauri/src/commands/projects.rs`) were guarded for the path's **provenance**, not
for what they do with it: the string came from the client's own native file dialog. One
component owns provenance now — `src/components/PathField.tsx` browses the machine the
path will be used on, the native dialog locally and `browse_directory`'s server-side
picker remotely — so the premise is gone. `inspect_repo_path` is in any case a pure
read whose worst answer for an absent path is `RepoPathStatus::Missing`, which is the
correct one and the one Add-Project's "a new git repository will be initialised" and
bare-repo messaging already handles. The removal also fixes `clone_project` and
`create_project`, which were never guarded and so quietly resolved a client-picked path
on the server.)*

*(Amended 2026-09-16 — Phase 6 took the remaining two guards out, so four
`remote_unsupported` guards remain, all permanent: `import_attachment`, `reveal_doc`,
`detect_editors`, and `set_binary_override` with `Some(path)`. `preview_prompt_import`
and `export_prompts` no longer take a server-side path at all — import reads a file's
contents (`source: String`) instead of a path, and export returns content for the
caller to write, so there is no longer a server-side path whose provenance the guard
would need to police. Two new commands join this carve-out list for the same reason
`reveal_doc` is here: `write_prompt_export` and `read_prompt_import`
(`src-tauri/src/commands/prompts.rs`) are plain client-side file I/O — they act on
the machine the user picked the path on, which is always the client, so neither takes
a guard. `read_attachment` (`src-tauri/src/commands/attachments.rs`) is different in
kind: it is the byte-carrying fix for attachment *rendering*, mirroring
`save_attachment`'s existing byte-carrying write, and it is an ordinary RPC call that
works identically whether the server is local or remote — it needs no guard either,
but for the opposite reason: it always crosses the wire, so there is no client-only
path to distinguish.)*

### The memory baseline

Phase 1 of the plan collapsed the PTY layer's two copies of every byte — a 256 KiB
scrollback deque and a 2048-slot `tokio::sync::broadcast` ring whose own comment
recorded a ~178 MiB worst case — into one byte ring per PTY, capped at
`RING_CAP_BYTES` (256 KiB), read through per-subscriber cursors
(`crates/hyprlayer-core/src/terminal/mod.rs`). No steady-state per-terminal memory
figure existed before that change, so this ADR is where the first one is recorded.

What a subscriber is guaranteed, and what it is not. A replay — after a trim, a
`clear_scrollback`, or a cursor that fell more than `RING_CAP_BYTES` behind — starts at
a flush end, and every flush ends on a char boundary because the coalescer holds a torn
trailing codepoint back for its continuation. So a replay never starts mid-codepoint. It
may start mid-line or inside an escape sequence: a flush end is a `read()` boundary,
not a line boundary, and aligning it would mean an escape-sequence parser in the
coalescer to fix a cosmetic defect that only appears at the head of a replay after
256 KiB of overflow. Lag is bounded in bytes rather than in 2048 flushes of unbounded
size, which is what the old ring's own comment asked for; the cost is that a slow
reader's slack is now 256 KiB, not up to ~178 MiB.

**Method** — the protocol from
`thoughts/shared/plans/2026-09-02-backend-owned-vt-state-libghostty.md` Phase 1, not a
second one: release build (`pnpm tauri build`); main process and web-content process
sampled separately; N = 1, 2, 4, 8 open idle terminals, left 60 s before sampling, each
reported as a delta from the zero-terminal steady state. Linux:
`/proc/<pid>/smaps_rollup` → `Pss` and `/proc/<pid>/status` → `VmRSS`; the web process
is `WebKitWebProcess`. macOS: `footprint <pid>` and `ps -o pid,rss -p <pid>`; the web
process is `com.apple.WebKit.WebContent`. `gdb`, `strace`, and `perf` are unavailable;
`/proc` counters and platform tooling are the instruments. Taken twice: at `2ad9f85`
(the commit before the ring change) and at the commit that lands it.

The "after" column below was taken on 2026-09-03 on the **debug dev build** through the
MCP bridge, macOS, N dock terminals: N idle shells, one mounted xterm at a time, because
the dock mounts only its active tab. RSS is `ps -o rss` in KiB; footprint is the
`footprint` tool's own total. The release-build pair the method calls for is **deferred as of 2026-09-15**, recorded with a date rather than left open a second time: the "before" column at `2ad9f85` was never taken, so the table below is a debug-build "after" only and the steady-state figure this ADR promised does not yet exist. What the ring change rests on is unchanged and does not need it — a documented ~178 MiB worst case deleted, one of two copies of every byte removed, lag bounded in bytes rather than in flushes of unbounded size. The measurement informs the *next* memory decision (Appendix A/B), which is where it should be taken.

| Terminals | Backend before (release, `2ad9f85`) | Webview before | Backend after (debug) | Webview after (debug) |
|---|---|---|---|---|
| 0 | TODO(manual verification) | TODO(manual verification) | 146,384 KiB / 52 MB | 396,336 KiB / 233 MB |
| 1 | TODO(manual verification) | TODO(manual verification) | 146,576 KiB / 52 MB | 397,776 KiB / 246 MB |
| 2 | TODO(manual verification) | TODO(manual verification) | 146,240 KiB / 52 MB | 401,056 KiB / 257 MB |
| 4 | TODO(manual verification) | TODO(manual verification) | 144,464 KiB / 52 MB | 375,104 KiB / 290 MB |
| 8 | TODO(manual verification) | TODO(manual verification) | 142,384 KiB / 52.4 MB | 399,680 KiB / 312 MB |

Read: eight idle PTYs add nothing the backend's footprint can see — 52 MB flat from
zero to eight — so an idle terminal's resident backend cost is under the 0.1 MB the
tool resolves. Each ring reserves 256 KiB, but untouched pages are not resident, and
the same holds for the three thread stacks each PTY adds. The webview grows 5 to 16 MB
per terminal *opened* with only one xterm mounted at a time, which is memory retained
after unmount in the frontend's territory (`src/lib/terminal/channelRelease.ts`), not a
per-live-terminal cost. Backend RSS drifting down across the run is page reclaim, which
is why footprint is the column to read.

What the figure will and will not say: a steady-state per-terminal delta for an idle
shell, which is the input every later memory decision needs. Not the cost of a
terminal holding a full 256 KiB of scrollback, and not a backend/webview
decomposition — `src/lib/terminal/channelRelease.ts`'s ~20.5 MB per dock remount is a
frontend leak figure and is not comparable.

## What we're not doing

- No multi-user, and no identity that grants authority: the announced client identity
  is display-only.
- No network listener, TLS, bearer tokens, or certificate management. Filesystem
  permissions on the socket are the authorization; SSH is the only remote path.
- No browser or mobile client — there is no listener one could reach, and adding one
  means adding authentication, which this decision deliberately does not design.
- No change to `TerminalEvent` or `SessionEvent`. `src/lib/bindings.ts` gains two
  things, not one: `browse_directory` (Phase 5) and the `serverDisconnected` event
  with its `ServerDisconnected` type (Phase 4's follow-up, recorded below).
  *(Corrected 2026-09-15 — this said "beyond the one command" while the same
  document already recorded the follow-up by name.)*
  *(Amended 2026-09-16 — superseded by
  `thoughts/shared/plans/2026-09-09-hyprlayer-server-remote-host-ui.md`'s Phase 1,
  which is the one phase across that plan's six that touches `bindings.ts`: four
  commands (`listHosts`, `saveHosts`, `testHostConnection`, `connectionStatus`),
  their supporting types (`Host`, `HostsFile`, `HostProbe`, `ConnectionStatus`,
  `ConnectionState`), and one added field, `path`, on `DirBrowseEntry` — a wire
  change that bumps `PROTOCOL_VERSION` 2 → 3. The taxonomy grows by four `AppError`
  codes: `remote_cli_missing`, `remote_auth_failed`, `remote_unreachable`, and
  `server_unavailable`, classified in `supervisor::classify_ssh_failure` from ssh's
  exit code and stderr. Two codes the 2026-07-30 plan proposed are deliberately not
  added: `remote_protocol_mismatch` (the handshake already produces
  `protocol_version_mismatch` with both versions and a one-sided remedy) and
  `remote_disconnected` (a mid-session drop already produces `server_disconnected`,
  now also the trigger for `AppState::mark_lost()`'s level-triggered
  `connection_status`). The new rule: a remote connect failure opens the app into
  a recovery state rather than exiting — `setup()` stops calling
  `std::process::exit(1)` on `supervisor::Outcome::Blocked` — except when the
  block is `runtime_in_use`: a second Hyprlayer holding this profile's lock is
  `tauri-plugin-single-instance`'s job, not a connection to recover from, and is
  the one arm that still shows the blocking dialog and exits.)*
  *(Amended 2026-09-16 — reversed. This bullet's "bumps `PROTOCOL_VERSION` 2 → 3"
  and the Phase 2 changelog entry's "`PROTOCOL_VERSION` is 2 from this phase"
  (below) both describe a compatibility surface that never shipped: at `v0.7.5`,
  the most recent tag, neither `hyprlayer-server` nor `hyprlayer-proto` existed —
  both crates live entirely in the unreleased commits since that tag, so no build
  carrying protocol version 1, 2, or 3 has ever been released. The per-version
  bumps were ceremony protecting nothing. `PROTOCOL_VERSION` is reset to 1 and
  stays there until the first release that ships `hyprlayer-server`; wire changes
  before that release are free, and the constant bumps on every wire-affecting
  change from that release onward. `remote-host-ui`'s Phase 1 (`DirBrowseEntry`
  gaining `path`) and Phase 6 (prompts and attachments carrying bytes over the
  wire, including the new `read_attachment` command) both landed under this
  policy, at version 1, not 3 or 4.)*
  *(Amended 2026-09-16 — that plan's Phase 8 adds one more command,
  `reconnectServer`, and one more code, `host_changed`. **A dropped connection to
  an unchanged host reconnects in place; a changed host still relaunches.** The
  asymmetry is not caution: `window.__HYPRLAYER_PROFILE__` is injected at window
  creation and every scoped zustand store fixes its key at store creation, so the
  localStorage namespace is a property of the window, not of the connection —
  reconnecting to the same destination cannot change it, and switching
  destinations cannot avoid changing it. `AppState::reconnected` swaps the client
  under the same lock `mark_lost` uses; the connect sequence after
  `supervisor::Outcome` is shared between launch and reconnect, so the environment
  overlay is re-applied to what may be a freshly respawned local server; and
  `mark_lost` now takes the client whose bridge is retiring, so a predecessor
  cannot declare the live connection lost. Asset-scope registration deliberately
  does not repeat — it derives from `client.worktree_root()`, unchanged for an
  unchanged host. This reverses "no automatic retry at all, because nothing
  respawns the server" for the transport case it was never argued from: the
  frontend retries at 1s, 2s, 5s, 15s and then every 30s indefinitely, plus
  immediately on window focus, single-flight, stopping only on success or on
  `protocol_version_mismatch`, which no amount of retrying resolves. Relaunching
  is now the answer to exactly one situation: `host_changed`.)*
- No change to codex's app-wide `startup_lock()`.

## Rationale

Today every session, terminal, and script dies with the Tauri process, and an app
*restart* is a session-killing event: `SessionRegistry` starts empty on every `open()`
and orphaned harness pids are logged and cleared, never reaped. The runtime already
runs headless — `crates/hyprlayer-core/tests/headless_runtime.rs` opens, imports,
observes, and shuts one down with no Tauri and no home directory on every
`cargo test` — so the server is the next consumer of a proven surface, not a new port.
Session fan-out already supports several attached clients and a disconnecting client
already does not kill a session; the terminal path needed the byte ring first, which
is why it shipped alone as Phase 1.

Socket-plus-SSH is what makes "no auth" safe: a process that cannot open the socket
file cannot talk to the server, and SSH brings its own identity. A network listener
would need all of the authentication machinery this decision declines to build.

## Consequences

- **Quitting the app is free; updating it usually is too.** Only a protocol change or
  a raised `MIN_SERVER_VERSION` makes an upgraded desktop shut down an older local
  server and spawn its own, which ends every live session and terminal on that
  profile. Release notes for such a version say so.
- The shell holds no runtime: `AppState` becomes an RPC client handle, and the 161
  Tauri commands keep their signatures while their bodies become RPC calls, so
  the frontend does not change *for the transport itself*. It changed for the
  consequences: a server that outlives the client needs a client that can notice
  the connection dropped and reattach. See the Phase 4 follow-ups below.
- `ProfileLock` stays in core and is taken by the server; `tauri-plugin-single-instance`
  keeps answering only "is another *window* open".
- The desktop never respawns a server that exited. A clean exit is the user's
  `systemctl --user stop`; an unclean one is a crash whose reason is written to
  `<app_data_dir>/.runtime-fatal.json` so a dialog can show it instead of a spinning
  respawn loop.
- Two of ADR-0008's seven known divergences are forced to move by the boundary
  (OpenCode model validation into `SessionService::set_model`;
  `ensure_registered_project`/`effective_for` into client calls); the other five stay.
  Each phase records what it moved here as it lands.
  - **Phase 2 (2026-09-03).** OpenCode model validation is in
    `SessionService::set_model`, and `opencode_server_if_running` is `pub(crate)`.
    `commands/mod.rs::effective_for` became `SettingsService::effective_config_for`.
    The nine `require()` command bodies in `commands/workstreams.rs` became
    `WorkstreamService` methods — a resolved `PathBuf` means nothing to a remote
    client — which also moved the PR attach/merge sequencing ADR-0008 listed as a
    divergence (`attach_pr`, `merge_pr`, `set_base_branch`); the Setup-script firing
    and the `worktree_write_file` mtime check stay, so four divergences remain. The
    CLI/OpenCode installers and the Keychain lookup moved into core per the
    carve-out changes above. `SettingsService` no longer exposes `binaries()` or
    `config()`, `SessionService::launch` returns `()`, and `RuntimePaths::detect`
    reads its five override variables from the supplied `RuntimeEnvironment`, so the
    only ambient reads in `env.rs` are `RuntimeEnvironment::detect` and `dirs::`.

    **The command table itself is deferred to Phase 3.** The plan asked for the
    concrete 150-line table in `hyprlayer-proto` this phase, but that crate's own
    automated criterion — serde as its only normal dependency — and the table's
    signatures (`Vec<Session>`, `Option<Session>`, …) cannot both hold: `Session`
    lives in `hyprlayer-core`, which proto cannot depend on. `commands.rs` ships the
    generic `commands!` macro instead (self-tested against a toy table); the real
    invocation and the `RuntimeEvent` wire envelope move to Phase 3, instantiated
    inside `hyprlayer-server`, the first crate where core's domain types and the
    wire-only proto crate are both in scope.
  - **Phase 3 (2026-09-03).** `crates/hyprlayer-server` exists, `[lib]` plus
    `[[bin]]`, and the desktop hosts it in-process: `src-tauri/src/lib.rs` still
    opens the runtime, then binds the profile socket against it and connects an
    `RpcClient` to its own socket, so `AppState` carries both for this phase. The
    command table lives in `hyprlayer-server/src/rpc.rs`: 137 request/response
    commands plus nine helpers (`find_project`, `project_source_of`,
    `resolve_doc_path`, `set_quit_confirm`, `invalidate_opencode_server`,
    `set_opencode_default_model`, `generate_title_{claude,opencode,codex}`) for
    the compositions the shell keeps. `hyprlayer-proto` stays serde-only — the
    `commands!` macro, the `ControlFrame`/`Outcome` envelopes, the strict handshake,
    and a pure frame codec; the async I/O wraps it in the server. Eighteen commands
    bypass the client: the 11 streaming ones (Phase 4), the five client-only
    carve-outs, `default_username` (Phase 5), and `opencode_auth_list` (OpenCode
    config I/O, a divergence that stays). The desktop's own compositions that
    needed the runtime — the workstream status batch and branch adoption, the
    `with_worktree` gitops helpers, branch-prefix read/write, archive listing, the
    Setup-script firing after create, and the `worktree_write_file` mtime check —
    now run in the server's dispatch arms rather than in a core service, so those
    two ADR-0008 divergences are relocated, not fixed. `probe_default_thoughts` is
    infallible in the bindings, so a dead transport degrades it to an empty probe
    rather than a wire change.
    *(Amended 2026-09-19 — the `opencode_auth_list` divergence did not stay. Remote
    hosts made it a bug: it and the key set/remove and default-model commands
    touched `auth.json` and `opencode.json` on the client, so a key added in
    Settings never reached a remote server. All four now run in `ProviderService`
    on the server, which invalidates `opencode serve` itself, so the
    `invalidate_opencode_server` helper is gone.)*
  - **Phase 4 (2026-09-03).** `AppState` is one field again
    (`src-tauri/src/state.rs`); `src-tauri/src/lib.rs`'s `setup()` no longer opens
    a runtime or binds an in-process socket — it calls the new
    `src-tauri/src/supervisor.rs`, and `client.rs` shrank to a bare
    `connect_socket` wrapper. The `RuntimeInUse` dialog is gone; a server already
    holding the lock is now the success case.

    **Streaming moved to the wire.** The eleven `Channel<T>` commands are a new
    request shape, `StreamRequest`/`StreamResponse`, declared in
    `hyprlayer-server::streams` rather than `hyprlayer-proto` for the same reason
    the 137-command table landed in `rpc.rs` in Phase 3 — their argument and
    return types (`Session`, `TerminalEvent`, `Stage`, …) are core's, and `proto`
    stays serde-only. `ControlFrame` gained a second request/response type
    parameter pair (`StreamRequest`/`StreamResponse`, five type parameters now)
    plus a `Cancel { stream_id }` variant, rather than folding streaming into the
    existing `Request`/`Response` enums: those are generated by the `commands!`
    macro as one flat enum with one `Dispatch` method per variant, and a
    streaming command needs the connection's outbound sender and stream registry
    that a plain `Dispatch` method never sees, so `read_requests` matches
    `StreamRequest` before it ever reaches `dispatch()`. The response is always
    sent — via the connection's one outbound queue — before the forwarder task
    that will write data-plane frames under the same stream id is even spawned,
    so a fast source cannot race a frame ahead of the id that names it. Data
    frames carry an 8-byte little-endian stream id ahead of the JSON-encoded
    event (`frame::encode_stream_frame`/`decode_stream_frame`), reusing
    `Frame::Data`'s existing shape rather than adding a new frame tag.
    `terminal.rs`'s and `scripts.rs`'s `pump` converged into one function in
    `terminal.rs`, now generic over nothing — it takes an `EventStream<TerminalEvent>`
    and calls `cancel_stream` on send failure, the wire equivalent of dropping a
    local receiver.

    **Lag recovery moved into the server's own stream forwarder**
    (`streams::run_stream`): on `SessionStreamError::Lagged` it calls
    `log_after`, re-sends the recovered entries under the same stream id, then
    re-attaches — the client never learns a lag happened. `spawn_live_pump`
    (`src-tauri/src/commands/sessions.rs`) lost its `Lagged` arm entirely and is
    now the same shape as `terminal.rs`'s `pump`. `emit_start_failure` stays
    shell-side exactly as the plan called for, because `start_stage_session`
    only sends `prepare_start`+`attach` synchronously over the stream; the
    fire-and-forget `launch_and_converse` became its own plain RPC command,
    with the server stashing the prepared `LaunchAndConverseParams` in a
    `StreamState` keyed by session id between the two calls (never serialized —
    it never leaves the server process).

    **`PROTOCOL_VERSION` is 2 from this phase** — streaming and the four new
    commands changed the wire, and a same-version server from an older build
    passes the handshake and then drops the first request it cannot parse,
    which the desktop now reports as a blocked start rather than running on a
    dead client. `tauri dev` rebuilds only the app, and a server outlives it,
    so both dev configs build the server binary in `beforeDevCommand`; the
    cross-process lock test builds it too, because `cargo test` builds a
    package's binaries only for integration tests and this crate has none.
    *(Amended 2026-09-16 — reversed; see the fuller amendment under "What we're
    not doing" above. Nothing has shipped at any `PROTOCOL_VERSION`, so the
    constant is reset to 1 and stays there until the first release that ships
    `hyprlayer-server`.)*
    **The handshake's version-mismatch arm now stays open for one more read.**
    `serve_connection` sends `Refused` and then, only if the peer's version is
    strictly greater than `PROTOCOL_VERSION`, reads one further control frame
    and honors it if it is `Shutdown`; an equal-or-older peer's connection ends
    there, so two mismatched clients cannot evict each other in a loop.
    `WireError::version_mismatch(ours, theirs)` attaches its remedy — "This
    server was started by a newer Hyprlayer. Update this client." — exactly
    when `ours < theirs`, which is also the branch `RpcClient::connect`'s own
    local `check_version` call takes when this side is the older one; the
    server's own `Refused` frame is never actually read by `connect()` (it
    already returns on its own check of the peer's `Hello`), so the remedy has
    to be correct from *either* side's independent computation, not just the
    server's.

    **A fatal `open` failure writes `<app_data_dir>/.runtime-fatal.json`**, but
    only for `RuntimeMigrationLedgerInvalid`/`RuntimeMigrationLedgerTooNew` —
    `RuntimeInUse` is deliberately excluded, because that is the supervisor's
    spawn-retry case (below), and writing the file for it would make a stale
    "in use" report look identical to a real corrupt-ledger failure on the next
    launch. The server deletes the file on the next successful `open`. The
    desktop reads it only *after* its own spawn attempt has exited, and only if
    it was written since that spawn: the first cut read it before spawning,
    which wedged every later launch on a stale report, because the one process
    that clears the file — a server that opens successfully — was never
    started (found in the 2026-09-07 manual pass; a restored ledger still
    produced the dialog until the file was deleted by hand).

    **The supervisor** (`src-tauri/src/supervisor.rs`) tries a connect first;
    on a version-mismatch refusal it either shows the "update this client"
    dialog (older) or runs the eviction dance — a fresh low-level handshake
    that sends `Shutdown` right after the peer's `Hello`, then polls for the
    socket to disappear, then spawns and connects. On `ENOENT`/`ECONNREFUSED`
    it checks `.runtime-fatal.json`, then spawns `hyprlayer-server serve` as an
    orphan (`process_group(0)`, no `kill_on_drop`, `DETACHED_PROCESS` on
    Windows — unverified, no Windows toolchain here) and races `try_wait()`
    against repeated connects. The spawned child's stderr is piped and drained
    into a shared buffer by a background task for as long as the child lives —
    not to avoid reading it, but so a *surviving* server's stderr writes never
    block on a full, unread pipe once the supervisor stops watching. One
    simplification against the plan's exact wording: only an early exit whose
    captured stderr contains `runtime_in_use` retries the *spawn* (five
    attempts, one second apart, per §4); any other early exit fails fast with
    the captured stderr in the dialog rather than spending all five attempts on
    a failure retrying cannot fix. The stderr buffer itself is unbounded for a
    long-lived server, accepted as a diagnostic-only cost rather than built out
    into a ring buffer.

    *(Amended 2026-09-19 — the pipe is gone. Its reader died with the app that
    spawned the server, after which every `eprintln!` in the server panicked on
    the broken pipe, and the relay's spawn sent stderr to `/dev/null`; the server
    also installed no tracing subscriber, so everything core logged was discarded.
    The server now writes daily files through `tracing-appender`
    (`crates/hyprlayer-server/src/logging.rs`): `hyprlayer-server.<date>.log` and
    `hyprlayer-relay.<date>.log`, seven days kept, filtered by `HYPRLAYER_LOG` or
    `RUST_LOG`, in the directory Tauri's `app_log_dir` rule gives the profile —
    the desktop passes its own as `HYPRLAYER_APP_LOG_DIR`, so a local server logs
    beside it. Both spawners send the child's stderr to
    `hyprlayer-stderr.log` in that directory and read back only what this
    spawn appended, which keeps the `runtime_in_use` retry and the early-exit
    dialog without a buffer that grows for the server's lifetime.
    `hyprlayer-server logs` prints the tails of all three, which is how the desktop
    fetches a remote host's logs over `ssh` with or without a running server.)*

    **Under `tauri dev` the server does not survive an app quit.** The Tauri
    CLI tears down the app's process tree when the app exits, and that takes
    the detached server with it — a server started by hand, or an app launched
    as a bare binary with vite already serving `devUrl`, survives the quit
    exactly as designed (both verified 2026-09-07). Production launches never
    go through the CLI, so this is a dev-harness caveat, not a product one:
    to exercise persistence during development, run `hyprlayer-server serve`
    yourself and let the app connect to it.

    **Three frontend gaps the manual pass surfaced, closed as a follow-up
    the same day** (the transport itself shipped no frontend change): the scripts
    dock now persists its terminal list and backend ids, so a relaunch
    reattaches through the existing `TerminalTab` path instead of opening a
    fresh terminal; `restoreSessionIfNeeded` asks `list_active_sessions` and
    resumes a session the server still reports live, so a mid-turn session
    keeps streaming after a relaunch instead of showing a frozen transcript;
    and a `ServerDisconnected` event, emitted when the runtime event stream
    ends, raises a sticky toast offering a relaunch, since nothing respawns a
    dead server. Closing the second one exposed a Phase 4 wire regression: the
    server replayed the persisted log on start/resume/restart/rewind streams
    where the shell had only ever replayed on `attach_session`, which would
    have duplicated the transcript a client restored before resuming;
    `prepare_stream` now hands the log only to `attach_session`. All three
    were verified live against a hand-started server on 2026-09-07.

    **The env overlay reaches every terminal PTY spawn and every `git`/`gh`
    shell-out.** *(Amended 2026-09-15: at the time this was written that was the
    literal truth and also the bug — it reached nothing else. Validation found
    harness drivers and script spawns had none of it, so a Commit- or Ship-stage
    agent pushing on its own, and any `.hyprlayer` Setup/Run script, failed on a
    daemonized server exactly as ADR-0005 predicts, while a terminal in the same
    session succeeded. Phase 6's manual `git push` check passed because it used a
    terminal — the one path that already worked — so a green check here was
    structurally incapable of catching it. Closed by extending the overlay onto the
    claude and codex driver env vectors (`runtime/sessions/service.rs`) and onto
    `script_env()` (`runtime/scripts.rs`), guarded by
    `the_attach_overlay_reaches_a_lifecycle_script` in
    `crates/hyprlayer-core/tests/workstream_runtime.rs`, which drives a real Setup
    script and fails without the fix.)* `RuntimeEnvironment` gained `set_env_overlay`/`overlay_env`
    (env.rs) and `HyprlayerRuntime::set_env_overlay` (mirroring
    `set_foreground`'s shape); `SetEnvOverlay` is a new plain RPC command the
    desktop calls once per connection, right after handshake, alongside the
    also-new `WorktreeRoot`/`SetForeground` commands, with
    `SSH_AUTH_SOCK`/`DISPLAY`/`TERM`/`LANG`/`LC_*` collected from the desktop's
    own environment. `TerminalService::open`/`open_cli`/`open_shell`/`open_agent`
    (`runtime/terminals.rs`) all append the overlay onto the `PATH`-only env
    vector they already built. `Exec` gained a bulk `envs()` builder method
    (`process/mod.rs`) for this.

    The `git`/`gh` half — scoped to the three files ADR-0005 already treats as
    the network boundary — required threading rather than a single seam:
    `WorktreeManager` (`worktree/manager.rs`) now holds a clone of the
    runtime's `env_overlay` `Arc` (set via a new `with_env_overlay`, sharing
    rather than copying, so a later attach is visible immediately), and its
    private `git_in` chokepoint merges it into every one of its ~20 callers
    inside that file, methods and free functions (`pull_base`, `fetch_remote`,
    `create_tracking_branch`, `delete_remote_branch`, `detect_base_branch`)
    alike. `repos.rs`'s eleven `Exec::new(git)`/`Exec::new(gh)` functions and
    `pr.rs`'s six each gained an `overlay: &[(String, String)]` parameter and
    an `.envs(overlay.iter().cloned())` call, uniformly rather than split by
    which individual function happens to touch the network today — the
    chokepoint framing that already applied to `git_in` — which cascaded to
    every call site in `runtime/projects.rs`, `runtime/workstreams.rs`,
    `runtime/settings_service.rs`, and one in `thoughts_repo.rs` (a
    genuinely local commit, passed an empty overlay rather than left
    uncompiled). `pr_watcher.rs`'s background poll loop reads the same shared
    `Arc` through a new `PrWatcherContext::env_overlay` field, since it calls
    `pr::pr_status` from outside any service's `self.runtime`. A new
    `worktree::manager::tests::the_overlay_reaches_a_real_git_child_through_git_in`
    proves it against a fake `git` shim that echoes `SSH_AUTH_SOCK`, rather
    than only through the PTY path the terminal tests already cover.

    **Two new `#[doc(hidden)]` test-support methods on `HyprlayerRuntime`**
    (`runtime/mod.rs`, alongside the existing `test_terminals`):
    `test_session_ingress` (raw `SessionEvent` injection into a session's
    actor, for forcing a lag without a real harness) and `test_db` (raw `Db`
    access, for seeding a session/workstream row directly). Both are behind
    `#[cfg(any(test, feature = "test-support"))]`, the same gate the file's
    existing helpers use. Enabling `hyprlayer-core`'s `test-support` feature as
    a `hyprlayer-server` dev-dependency (needed for both) has a side effect
    worth naming: it makes `hyprlayer-core`'s own `test-support`-gated
    integration tests (`library_runtime.rs`, `project_runtime.rs`,
    `prompt_runtime.rs`, `provider_runtime.rs`, `runtime_lifecycle.rs`,
    `session_runtime.rs`, `workstream_runtime.rs` — 102 tests across seven
    files) compile and run for the first time under a plain
    `cargo test --workspace` — nothing had ever requested that feature before.
    All of them passed; the one thing that did not was a `clippy::zombie_processes`
    warning newly surfaced in `runtime_lifecycle.rs`'s
    `open_reaps_an_opencode_server_orphaned_by_a_previous_run` (a pre-existing,
    latent false positive — every path already reached a `wait()`, clippy's
    control-flow analysis just could not see it through the loop), fixed with a
    one-line restructure to make the `wait()` call unconditional rather than
    adding a suppression.

    The cross-process lock test resolves the sibling `hyprlayer-server` binary
    from `std::env::current_exe()` two parents up (`target/<profile>/deps/... `
    → `target/<profile>/`), exactly as this ADR's phase already anticipated;
    `cargo test` builds the `[[bin]]` target as a side effect of building the
    package under test, so the binary is there by the time the test runs it.
  - **Phase 5 (2026-09-08).** `crates/hyprlayer-server/src/relay.rs` is the
    server-side half: `relay::run` connects to the profile's local socket,
    spawning `hyprlayer-server serve` as the same kind of deliberate orphan
    `src-tauri/src/supervisor.rs::spawn_detached` already used if nothing
    answers, then `tokio::io::copy_bidirectional`s the connection against a
    caller-supplied `S: AsyncRead + AsyncWrite + Unpin + Send + 'static` — the
    process's own joined stdin/stdout in production
    (`tokio::io::join(tokio::io::stdin(), tokio::io::stdout())`), an in-memory
    `tokio::io::duplex` pipe in the round-trip test, so no SSH child or real
    socket is needed to exercise the splice. `RpcClient::connect`'s genericity
    (Phase 3) needed no change to make this work — it was already generic over
    the transport, and this is its first non-Unix-socket caller. Production
    stdio needs one manifest addition beyond Phase 3's `net`: the workspace
    `tokio` features gained `io-std`, for `tokio::io::stdin`/`stdout`.

    **Real reachability, not just the relay's own half.** The plan's Phase 5
    file list never named `supervisor.rs`, but the phase's own goal — the
    desktop actually reaching a remote host — needs a client-side counterpart,
    so `supervisor::connect_remote(host: &hosts::Host) -> Outcome` was added
    there, reusing the existing `Outcome` enum rather than inventing a second
    one. It spawns `ssh <destination> hyprlayer-server relay`, takes the
    child's `stdin`/`stdout` and joins them with `tokio::io::join`, and hands
    the join straight to `RpcClient::connect` (not `connect_socket`, which is
    Unix-socket-only). A connect failure — spawn failure, handshake refusal, a
    version mismatch, anything — returns `Outcome::Blocked` naming the host,
    exactly like a local failure already does, and never falls through to
    `connect_or_spawn`: silently opening the local profile instead would be
    the "single most damaging mistake" the plan names for `RuntimePaths`
    misresolution, in spirit if not in letter, so the same discipline applies
    here. A refusal renders through `detail_for` rather than `ConnectError`'s
    `Display`, which is the message alone: a version skew's remedy is the only
    instruction that resolves it, and the remote path has to show it for the
    same reason the local one does. The remote arm deliberately does not run
    Phase 4's eviction dance — a desktop update should not end a shared remote
    host's live sessions on its own, so the skew is reported and left to the
    user. The relay child gets `kill_on_drop(true)` and is kept alive by a
    background `tokio::spawn(child.wait())` rather than stored anywhere,
    deliberately the opposite hygiene from the local server's orphan: killing
    this SSH connection must only end the relay, never the persistent daemon
    on the far end, so the child should die with the app, not outlive it.
    `src-tauri/src/lib.rs`'s `setup()` reads `hosts::active_host` before
    calling either connect path — the one decision that has to be made
    locally, since there is no server yet to ask which server to reach — and
    threads the result into `AppState::new(client, remote_host)`, a second
    field `state.rs` did not have before this phase.

    **`src-tauri/src/hosts.rs`** holds `Host { destination, label }` and
    `HostsFile { hosts, active }` at `<app_config_dir>/hosts.json`, tmp-then-
    rename like every other config write in this tree. `save` has no
    production caller yet — the picker UI that would call it is deferred, per
    the plan — so it carries `#[allow(dead_code)]` with a one-line reason
    rather than being deleted or given a fake caller; the round-trip test
    calls it directly, and it's what the deferred picker work will call next.

    **`browse_directory`** is `ProjectService::browse` in
    `crates/hyprlayer-core/src/runtime/projects.rs`, returning
    `DirListing { path, parent, entries: Vec<DirBrowseEntry> }`. The per-entry
    type is `DirBrowseEntry`, not the plan illustration's `DirEntry` — that
    name is already `gitops::DirEntry`, already imported into
    `hyprlayer-server/src/rpc.rs`, and means something different (a
    worktree-relative git listing, not an absolute filesystem one). Both types
    carry `specta::Type` directly, following `inspect_repo_path`'s and
    `check_worktree_files`'s precedent (a leaf, single-consumer DTO returned
    unwrapped through the Tauri command) rather than `ProjectView`'s
    shell-side-mirror-struct precedent, which exists for types with several
    consumers needing different `camelCase` shaping. `browse`'s own body is a
    thin wrapper around a private free function, `list_directory(dir: &Path)`,
    for the same reason `inspect_repo_path` wraps `repos::classify_repo_path`:
    it lets the unit test call the listing logic directly with a `tempdir`
    path, instead of opening a full `HyprlayerRuntime` for a test that only
    needs a filesystem. `browse_directory` itself is deliberately *not*
    remote-guarded — it is the one operation this phase adds specifically so a
    future picker can browse the server's filesystem when the server is
    remote, so gating it on remoteness would defeat its purpose.

    **`default_username` moved server-side**, as `SettingsService`'s
    `default_username()` reading `RuntimeEnvironment::var("USER")`/`"USERNAME"`
    — the server's own captured environment, not a direct `std::env::var` call
    from inside a core operation, keeping ADR-0008's hermeticity contract
    intact. A new `DefaultUsername` line in `hyprlayer-server/src/rpc.rs`
    carries it over the wire. The Tauri command changed shape more than the
    plan's wording implies: it needed to stay `pub async fn` and infallible
    (falling back to `"user"` on any wire error) so `defaultUsername: () =>
    __TAURI_INVOKE<string>("default_username")` stays byte-identical in
    `bindings.ts`, but an async command that borrows `State<'_, AppState>` and
    does *not* return a `Result` fails to compile — a Tauri constraint hit
    while implementing this, not a choice. So it takes `app: tauri::AppHandle`
    (owned, no lifetime parameter the macro can see) and calls
    `app.try_state::<AppState>()` internally to reach the same managed state.

    **Editor launching has no command of its own to gate.** `editors.rs`
    supplies `detect_editors`, called once to populate a menu; the actual
    "open with" (`openPath`/`revealItemInDir`) is invoked directly from the
    frontend against `@tauri-apps/plugin-opener`, bypassing `commands::*`
    entirely — confirmed by grep, there is no second command. `detect_editors`
    gained the `state: State<'_, AppState>` parameter and remote-unsupported
    guard instead: with no editors returned, the frontend's own "open with"
    menu has nothing to launch, which is the closest a backend-only phase can
    get to disabling launching without touching `src/`.

    **The four import-adjacent commands split three-and-one, matching the
    plan.** `import_project`, `import_local_folder`, `inspect_repo_path`
    (`commands/projects.rs`) and `import_attachment`
    (`commands/attachments.rs`) gained the guard; `clone_project` did not,
    deliberately — it takes a URL the server resolves itself, not a
    client-side path, so it is exactly as valid remote as local.
    `reveal_doc` (`commands/library.rs`) gained the same guard, checked before
    the `resolve_doc_path` round trip rather than after, since there is
    nothing useful to do with a resolved remote path on the client anyway.

    **Three more the plan's list did not name.** Sweeping the frontend for
    `@tauri-apps/plugin-dialog` call sites — the real definition of the class,
    rather than the plan's enumeration of it — found three further commands
    whose path is picked by a native dialog on the client and then interpreted
    by the server: `preview_prompt_import` and `export_prompts`
    (`commands/prompts.rs`, from `PromptLibrary.tsx`'s open and save dialogs)
    and `set_binary_override` (`commands/system.rs`, from `BinaryCard.tsx`'s
    Locate…). All three are guarded. `set_binary_override`'s guard is the one
    conditional in the set: it fires only when a path is supplied, because
    passing `None` *clears* an override and is valid against any server —
    a blanket guard would make a remote profile's override unclearable.
    `apply_prompt_import` needs none: it consumes a decision, and the preview
    that produces one is already refused. Left alone deliberately, on the same
    reasoning as `clone_project`: `create_project`'s `location`, which is a
    destination the server creates rather than a client file it must read, and
    which already defaults to the server's own repos root when absent.

    **Two more take a natively-picked path and are still unguarded, on
    purpose.** `create_thoughts_profile`'s `thoughts_repo`
    (`AddProjectDialog.tsx`'s "New profile…" Browse button) and
    `init_thoughts_repo`'s `ThoughtsSource::{Disk,Create}` path (onboarding
    step 3's `RepoSourcePicker`) both reach the server from a client dialog,
    and both are excluded for the `clone_project` reason: a thoughts repo is
    created or opened *on the server*, so the path is a destination, not a
    client file the server must read. Guarding `init_thoughts_repo` would also
    make remote onboarding impossible — a fresh remote profile shows the
    onboarding flow, verified against the container — turning a blind-but-
    workable path into a dead end. Both fields accept typed input today, so a
    remote user can name a server path; they just cannot see one. That is the
    directory picker's job, not a guard's. The line these guards actually draw
    is *the server would read a path that exists only on the client* — which
    is narrower than "a native dialog filled this field", and is the framing to
    apply to the next command that looks like one of these.

    **No `///` doc comment on any of the three touched or new commands.**
    tauri-specta reads a command function's doc comment into `bindings.ts` as
    a JSDoc block, and none of the existing 161 commands carry one (verified
    by grep across `commands/*.rs`) — apparently a deliberate house style, not
    an oversight, since this phase's own success criterion is a `bindings.ts`
    diff of exactly one added command. The explanatory comments above
    `browse_directory`, `detect_editors`, and `default_username` are plain
    `//` lines for exactly that reason; a first pass with `///` produced a
    correct-but-noisier diff that added a JSDoc block to two pre-existing,
    otherwise-untouched command signatures.

    **Verified live against a Linux container, 2026-09-08**, since the phase
    is untestable without a second machine: a Debian image built the server
    for its own arch and served it over sshd, and `ssh <host> hyprlayer-server
    relay` was driven by hand-written control frames (`C` + one JSON line, per
    `frame.rs`). Confirmed there: the relay spawns the daemon when none is
    listening; `default_username` answers with the *server's* user, not the
    client's; `browse_directory` lists the server's filesystem; a `WireError`
    with its remedy crosses intact; the daemon outlives each SSH connection
    and later relays reuse it rather than spawning a second (one `serve`
    process across the whole run); and a project imported through one relay is
    still there when read through the next.

    The client-side halves short-circuit in the shell and never reach the
    wire, so they were verified separately, the same day, by pointing a
    `tauri.dev.conf.json` build's `hosts.json` at that container: the app
    spawned `ssh hyprlayer-container hyprlayer-server relay`, the container's
    daemon auto-started, and **no local server was spawned** — the no-fallback
    rule holding in practice. All nine guards returned their named error
    ("… is not available while connected to the remote host Test Container"),
    `set_binary_override` cleared an override successfully in the same session
    that refused setting one, and the app rendered the *container's* state:
    onboarding rather than this machine's configured profile, with the name
    field pre-filled `hyprtest` from the relocated `default_username`. Still
    unverified, for want of a second app version on that box: the
    rollback/roll-forward check that `hosts.json` survives an older binary.

    **The remaining remote checks ran on 2026-09-09**, against the same
    container once it had `claude` 2.1.266 and the `hyprlayer` CLI 1.6.2
    installed and a project registered server-side (the import commands are
    guarded, so a remote project has to come from the server — the CLI's own
    `thoughts init` registered one). Confirmed: a terminal opened through the
    app reports the *container's* hostname and `aarch64`, running in the
    worktree the server created; a Claude session started on the host answered
    from the server's shell; and after `kill -9` on the relay — which left the
    server and its `claude` process untouched, as designed — a relaunched app
    reconnected and `list_active_sessions` still reported that session
    `running` with its original id and cost. `save_attachment` put a
    byte-identical file (sha256 verified both ends) into the server's
    worktree.

    **Two criteria as written do not hold, and the reasons are worth keeping.**

    *A remote attachment cannot render.* `save_attachment`'s transport half is
    correct, but `attachmentSrc` is `convertFileSrc(path)`
    (`src/features/session/attachments.ts`), which resolves an `asset://` URL
    against the **client's** filesystem — and the file is on the server, so
    `AttachmentCard`'s `<img>` 404s. The asset-scope registration compounds it:
    `lib.rs` allowlists `client.worktree_root()`, a path that does not exist on
    the client at all when the server is remote. Bytes reach the server; the
    picture never comes back. Fixing it means serving attachment bytes over the
    wire and rendering from a `blob:` URL, which is real work and is recorded
    as scope in `thoughts/shared/plans/2026-09-09-hyprlayer-server-remote-host-ui.md`
    rather than patched here.

    *Branch prefixes were never username-derived.* The criterion asks that a
    branch created remotely use the server's username; in this tree the prefix
    comes from `AppSettings.git.branch_prefix`, defaulting to the literal
    `hyprlayer/` (`settings.rs`), and a remote workstream duly landed on
    `hyprlayer/remote-smoke-test-0b78`. `default_username` feeds the *thoughts
    attribution user* instead — onboarding's "Your name" field, which
    pre-filled `hyprtest` from the server. The intent behind the criterion
    (server identity, not client identity) holds; the mechanism it names does
    not exist.
