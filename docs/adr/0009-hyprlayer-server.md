# ADR 0009: The runtime lives in a `hyprlayer-server` process; the desktop is a client

**Status:** accepted · 2026-09-03 · amended 2026-09-18 (the Keychain lookup is
deleted — the runtime reads no provider credentials) · amended 2026-09-19 (OpenCode
config I/O moves to the server) · amended 2026-09-19 (the server writes its own log
files; a spawned server's stderr goes to a file, not a pipe) · amended 2026-09-19
(the server is versioned and released apart from the app; the app sets a minimum) ·
amended 2026-09-26 (the environment overlay merges per key and validates the agent
socket) · amended 2026-09-30 (connections announce a display-only client identity;
transcript-sync events are targeted) · amended 2026-10-06 (the Cloud account belongs to the
server profile; `AccountChanged` is targeted by capability alone) · amended 2026-10-07
(the cross-process lock test is an integration test, so it no longer builds the server
binary mid-run; the release gate runs its tests under `cargo nextest`) · amended 2026-10-16
(`runtime_in_use` opens the recovery screen instead of a native dialog and exit) ·
amended 2026-10-16 (`GoalChanged` is targeted by capability alone) · amended 2026-11-11
(agent connections: per-spawn identity, a narrower command set; PtyAgentChanged and
UiRequested are targeted; the `statusline` face; PTY codex over its own app-server;
`portable-pty` is vendored) · amended 2026-11-21 (a heartbeat gives up a link gone silent;
events leave large frames and stream data their share; a queued revision gives way to a
newer one, and a device streaming a session hears none of its commits)

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

*(Amended 2026-10-06 — the Hyprlayer Cloud account (ADR-0011) belongs to the server
profile too: one `account.json` per server host, so every attached client sees one
signed-in state, and signing in on one host signs in none of the others.)*

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
  *(Amended 2026-10-06 — `AccountChanged` is targeted by capability alone: it reaches
  every connection that announced it, watching a session or not.)*
  *(Amended 2026-10-16 — `GoalChanged` is targeted the same way, so a goal's notices reach
  every announced device and a released desktop never receives a kind it cannot decode.)*
- **A session stream sends what its client keeps.** *(Added 2026-11-19.)* A `with_meta`
  stream on an announced connection gets only the partial text that client renders:
  main-thread text from its own turns, an agent's, or an unattributed one. Another
  device's typing and subagent typing stay on the server unless the stream sets
  `all_partials`. Partials are coalesced per stream: the first goes out at once, then at
  most one frame per 50 ms. A live row carries `createdAt`, so it is a resume cursor. A
  `list_session_turns` page stops before the row that would pass 256 KiB of payload and
  always holds one. The frame decoder has no size cap, because one row, one attachment
  upload or one doc read is a single frame of any size. A cap low enough to protect a
  phone would drop the connection on real data.
  *(Amended 2026-11-20 — a `tail` cursor reads the newest page and `before` the page
  ahead of a verified row, so a long session's newest rows arrive in one page and the
  older ones follow: 18 round trips and 4.7 MB before the first paint became one and
  133 KB. A rewind refuses a `before` cursor whose row it cut, as it does an `after`.
  A `tail` page also carries `open_asks`: the ask rows older than it that the live
  harness still waits on, within the page's budget, so a pending prompt shows at
  first paint. A server from before this answers `unsupported_command`, and the
  client reads forward instead.)*
  *(Amended 2026-11-20 — an ask that leaves the harness's pending set is logged as a
  `permissionResolved` or `questionAnswered` row, so a reload never offers an answered
  ask again. Those rows are targeted like the events above: a connection gets them only
  if it announced them. A `tail` page also names `open_request_ids`, every ask the live
  harness waits on, and the view closes any ask outside it.)*

*(Amended 2026-11-11 — agents spawned by the runtime carry `HYPRLAYER_CTL`,
`HYPRLAYER_SOCKET` and a per-spawn `HYPRLAYER_AGENT_TOKEN`. The `hook`, `ctl` and `mcp`
faces of this binary connect with them, and `announce_agent(token)` tags the connection.
The token is not a security boundary: any same-uid process can open the socket. Identity
grants nothing. An agent connection gets a narrower, default-deny command set and no event
stream: read verbs at `read`, mutating verbs at `full`, never the excluded table. A bypass
caller resolves to `full` unless agent control is `off`. `PtyAgentChanged` and
`UiRequested` are targeted by capability alone. Codex PTY launches pass
`--dangerously-bypass-hook-trust`, which also trusts the user's own unreviewed `~/.codex`
hooks for that launch. A hooked PTY claude's statusLine is the `statusline` face: it reports
model, effort and context, then runs the user's own statusLine command with the same input
and prints its output. On Unix, when the installed codex has `--remote`, a PTY codex runs over
a per-terminal `codex app-server` that the server spawns, and reaps with the terminal or, after
a crash, at its next start; the TUI attaches with `--remote`, so the status bar changes
settings through that app-server. Otherwise, and on Windows, codex keeps the embedded launch. `portable-pty` is vendored (`vendor/portable-pty`) without
`PSEUDOCONSOLE_INHERIT_CURSOR`: that handshake blocks a ConPTY child until a terminal answers
`ESC[6n`, and a PTY no desktop has mounted never gets one.)*

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

*(Amended 2026-10-16: the overlay rides behind the resolver's augmented `PATH`, which
`child_env` (`BinaryResolver`, `HyprlayerRuntime`) puts first on every spawn. The
`git`/`gh` shell-outs had taken only the overlay, so a server holding launchd's `PATH`
ran git without Homebrew's dirs and a git-lfs checkout failed. Guarded by the
`git_child_path` E2E scenario.)*

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
`SSH_AUTH_SOCK`, `DISPLAY`, `TERM`, `LANG`, `LC_*`. No secrets, no tokens *(amended
2026-10-06: still true; the Cloud tokens live only in the server's `account.json`,
ADR-0011)*. A stale
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
path to distinguish. Both carry the bytes as one base64 string, 1.33× the payload on
the wire; a JSON number array cost 3.57×.)*

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

*(Amended 2026-11-19 — over the socket that promise did not hold. The terminal's
forwarder kept draining the ring into a 256-frame attach channel and a 1,024-frame
connection queue, so the ring never dropped anything: a stalled reader with `yes`
running grew the server by 82 to 161 MiB in 8 s. It holds now because nothing reads the ring
until the connection has room (see *The connection* below). A stalled reader costs the
ring plus 32 KiB queued per terminal, the stream's share of the writer: the
server's forwarder reads the ring itself, with no core forwarder or attach channel
between them. A blocked forwarder
can no longer see its terminal exit, so `TerminalRegistry::insert` watches for the exit
itself and drops the entry whether or not anyone reads.)*

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

### The connection: priority, credit, gaps and liveness

One socket carries responses, runtime events and every stream's data, so a reader on a
slow or stalled link must not make any of them wait on, or grow behind, the others. The
2026-11-19 measurements on a 256 KiB/s link: a small call answered 7.1 s late behind one
1,222-row `list_session_turns`, 237 MB of terminal output queued ahead of a small RPC,
events dropped with nothing telling the client, and an unread stream growing the client
by 652 MB in 12 s. `PROTOCOL_VERSION` stays 1; everything below is negotiated.

- **The hello's `features`.** Both sides list `flow_control`. A side sends part frames,
  `credit` or `events_missed` only to a peer that listed it, so a released desktop keeps
  the base protocol. `link_credit` and `heartbeat` (below) are negotiated the same way.
- **One writer, in priority order** (`hyprlayer-server/src/outbound.rs`): frames up
  to 16 KiB, then runtime events, then larger responses and stream data taking turns.
  A frame over 16 KiB goes out as **part frames** — `P`, a `u32` length, a lane byte, a
  `last` byte, then a slice of the whole frame's encoding — so a small frame waits behind
  at most one part. Priority alone was the smaller change, but a large frame already on
  the wire would still hold everything for its full length; parts are what keep that
  wait to one slice. The decoder reassembles parts per lane, so callers see only
  control and data frames.
- **Large frames and streams take turns of 16 KiB.** Within each of those two classes the
  writer runs deficit round-robin: every large response, and every stream, sends up to
  16 KiB a turn, so a 2 MiB answer no longer holds a 27 KiB one (8.5 s before, 0.27 s
  now), nor one session row a terminal's ticks (6.6 s, 0.22 s). Each split frame holds a
  lane id of its own from its first part to its last, out of 256; a frame waits for one
  when all are taken. The `flow_control` decoder already kept one partial frame per lane
  byte, so a peer from #408 reads the interleaving as is.
- **The client writes the same way.** `RpcClient` queues through the same `Outbound`:
  requests, `credit`, `cancel` and `received` first, a request over 16 KiB in parts. A
  2 MiB upload on a 256 KiB/s uplink held a small call and a keystroke 9.1 s and stopped
  the stream it held credit for; now 0.5 s. A server that listed `flow_control` decodes
  parts, so this needs no new feature. A desktop terminal sends one write at a time
  (`terminalInput.ts`), so a large paste and the Enter after it can't swap.
- **Stream credit.** Each stream may have `STREAM_WINDOW` (512 KiB) of event bytes
  unread at the client. The server's hello names that window (`stream_window`), and the
  client returns credit (`credit { stream_id, bytes }`) as its consumer reads, in batches
  of a quarter of the named window, so either side's constant can change alone. Each
  stream may also have 32 KiB queued in the writer, two turns' worth, so a stream of
  small rows fills its turn: with one frame in flight it got one ~3 KiB row per 16 KiB
  terminal part. The terminal forwarder waits for both before it reads the ring
  again, so output piles up in the ring, which coalesces it and drops the oldest. It
  reads at most 16,362 bytes, so a frame fits one part, and cuts on a char boundary
  since a resume can start there. A session stream that waits instead lags its
  broadcast and recovers from the log, as before. The client queues a stream's frames
  without waiting, so one unread stream
  never stalls the read loop; a stream past 64 MiB unread is closed.
- **Each link holds about what it carries in a round trip and a half.** A buffering link
  (an SSH channel, a bufferbloated cell link) takes whatever is written, and a small call
  then waits behind all of it: 7.4 s behind three terminals at 256 KiB/s, 8.9 s behind one
  at 64 KiB/s. With `link_credit` on both hellos, each side keeps no more written and not
  yet reported read than `link.rs` allows: 1.5 × the link's recent peak rate × its
  smallest round trip, from 128 KiB to 8 MiB. Each side sends `received { bytes, read_us }`
  whenever it has decoded all it read, and before it parses a frame over 16 KiB, stamped
  with when it read them. The rest waits in the writer, where priority and turns apply.
  - Frames up to 16 KiB and events may run 128 KiB past the window, so a report held up
    behind an upload doesn't hold calls or answers the other way, yet an event flood
    can't fill the link. Reports go first and aren't counted, or reporting a report
    would never end.
  - Both ways, because a client's upload filled ssh's ~2 MiB channel ahead of its
    reports, and the server stopped writing: a keystroke's echo during a 4 MiB upload
    waited 14.6 s. Windowing the client's own writes keeps that pipe to its window.
  - Every byte counts, not stream bytes only. A cap on stream frames as they are queued
    stops every stream behind one large row until the link carries it, and a cap on
    bytes consumed lets an unread stream hold the whole budget.
  - Rates come from the reader's stamps, so a report late on its way back neither slows
    the measured rate nor reads as a queue: its lateness past the smallest seen is taken
    off its round trip. One burst's reports, how far apart the reader read them, give
    the link's rate; for a burst that arrived in one read, how late it came past the
    round trip two agreeing small frames showed. Without those, a window-limited sender
    grows eightfold a round trip until it sees a queue.
  - Cut fast: a round trip past three times the smallest forgets the peak and sets the
    window to the current rate × the smallest round trip. A sample from a sender the
    window wasn't holding back can raise it, never lower it, and a sender that hasn't
    filled the window for 1 s, or a link idle that long, starts again from 128 KiB.
  - The smallest round trip is measured again when none has come near it for 2 s: the
    window holds to 16 KiB until a part sent then is reported, whose round trip, and
    lag on the way back, replace the old ones. Only that probe can raise them, so a
    queue can't ratchet them up.
  - The stated bound: after a rate drop, a small call waits about 1.5 × the smallest
    round trip × the old rate ÷ the new one, less when stream credit holds less in
    flight. From 4 MiB/s to 256 KiB/s at 100 ms behind three terminals it waited 2.4 to
    2.5 s, then 0.5 s once the window had cut.
  - Rejected: a fixed 128 KiB window, which kept 2 s at 64 KiB/s but made a 5.3 MiB
    read take 1.18 s at a 20 ms round trip and 4.6 s at 100 ms. The smallest round trip
    over the connection's life, which pinned a path that grew from 4 ms to 150 ms at
    0.86 MB/s against 8.5 MB/s fresh; the probe gets 8.2. A sliding minimum, because a
    link that never empties ratchets it up with its own queue. Gain 2, which gives 3.2 s
    after that drop. Trusting one small frame as the bare round trip: one 20 to 40 ms
    slow let 2.8 to 5.6 MB onto a 1 MB/s link. Taking a burst's spread past its first
    round, since then `ahead` includes what was delivered but not reported: it measured
    409 MB/s on a 4 MiB/s link. Answers and events wholly outside the window, which
    would let an event flood fill a buffering link.
  - The cost: a connection's first large read pays one more round trip. A 5.3 MiB read
    took 0.37 to 0.42 s at 100 ms and 0.23 s at 20 ms, then 0.27 s and 0.18 to 0.19 s,
    against 0.27 s and 0.18 s before; without two small calls first, 0.47 to 0.52 s at
    100 ms. A local socket is unaffected, 0.17 s against 0.16 s. While an upload fills
    the uplink, a stream fell from 1.5 MiB/s to 383 KiB/s.
- **Requests in flight, not reading, are capped, in two pools.** The read loop never
  waits, so `credit`, `cancel` and `received` always land. A request whose answer carries
  a file's or a transcript's contents (`Request::answers_in_bulk`: attachments,
  transcript pages, docs, worktree and review files, terminal and script output, skill
  files) waits for one of 16 slots per connection, and any other call, a stream's open
  included, for one of 64. Each holds its slot until its answer is written. Once the
  connection closes, what it sent still runs without one and only the answer is lost: a
  start dropped mid-prepare must still end as a failed start, and a launch sent just
  before the drop must still run (`failed_start`). Four take none, since their answers
  are a few bytes: `wait_session` and `wait_pty_agent` park for their timeout, and
  `terminal_write` and `terminal_resize` must not queue behind slow calls such as `gh`.
  With one pool, a `respond_permission` behind 16 large reads at 256 KiB/s waited 4.5 s
  while every slot held an answer; now 67 ms. A reader that stopped reading holds at
  most 16 bulk and 64 other answers, each serialized straight into its frame rather
  than copied there: 64 pipelined 2 MiB reads grew the server by 279 MiB before and
  60 to 67 MiB now, and 512 pipelined 316 KiB directory listings by 125 MiB with no
  slot for them and 25 MiB with one. The bound is 16 times the largest bulk answer
  plus 64 times the largest other. The 4 MiB pause on unwritten responses it replaces stopped `credit`
  and `cancel` too, and lifted only once a whole response was written, so one 4.7 MiB
  answer held a small call 10.4 s.
- **A stream ends with its connection.** Closing a connection wakes a stream waiting
  for credit. A connection task aborted at shutdown still aborts its streams. A stream
  id is reserved before the response that names it is written, so a `cancel` that
  arrives first still cancels it. The client's queue has no cap, so a dropped receiver's
  `cancel` never waits. The client and each stream returning credit keep its writer
  running; once the last is dropped, what they queued is still written.
- **Events carry a per-connection `seq`.** The server holds at most 256 events or
  64 KiB per connection, and an empty queue takes one event of any size. Each event is
  serialized once per broadcast, and only once some queue has room. Past the cap, and
  whenever the forwarder lags the runtime's broadcast, events are dropped and the
  connection gets `events_missed { first_seq, count }` in their place. A lag carries
  only a count, so every connection that hears events is told. The client counts the
  markers, any `seq` jump, and its own 1,024-event queue overflowing in
  `RpcClient::events_missed`.
- **Re-reads are paced.** The desktop's `GapPacer` emits `EventsMissed` once a burst of
  gaps has been quiet for 500 ms, and never twice within 5 s, so a refetch can't feed a
  flood more gaps. The window then re-reads what a reconnect re-reads, minus the session
  stream reattach, since those streams are intact.
- **Events lead large frames and stream data by at most 16 KiB.** While a larger
  response or stream data waits, the writer counts the event bytes it sends, and past
  16 KiB the next unit is theirs. Events wait for it even when only the link window holds
  it back, since a flood of events keeps the window full by itself. Small frames still go
  first. On a 32 KiB/s link a 25 s flood of revisions held a 128 KiB answer 22.5 s
  against 4.1 s alone, and paused a ticking terminal 4.9 s; now 6.5 to 7.4 s and 0.9
  to 1.0 s.
- **A queued revision gives way to a newer one.** A `SessionRevised` takes the place and
  the `seq` of its session's latest queued event when that one is a revision by the same
  author and both carry turns or neither does, so a turn's end or a released send never
  folds into a commit. It overtakes queued events about other sessions, never one about
  its own, and never crosses a gap marker. A reader stalled through two 120-step turns got
  210 of their 484 revisions and missed 276, the latest among them; now it gets 4, the
  last of them the latest, and no gap.
- **A device streaming a session hears none of its commits.** A client that announces
  `revisions_in_stream` gets no `SessionRevised` that carries turns for a session one of
  its streams carries with `with_meta` and echoed user messages: that stream carries
  every such row with its rev and author. Revisions without turns still go, since a
  turn's end and a released send move no row and an attached view rechecks on them
  (`transcriptSync.test.ts` T18). The desktop announces it, and its attached view needs
  nothing more: a revision there only raises `committedRev` and names the working device
  (T1, T20), which the stream's rows do too, held or applied, and the stream's `turnDone`
  flushes the queue. An ended view holds no stream, so it still hears every commit (#417,
  #418). One 120-step turn sent 242 revisions, 74,940 bytes, to a device that also
  streamed it; now 1. A released desktop announces no such thing and hears all of them.
- **A link gone silent is given up.** With `heartbeat` on both hellos, each side times
  how long its reads have had nothing. At 10 s it sends `ping`, which the peer answers
  with `pong`; at 30 s the read fails. The server then drops the connection, which
  unregisters its viewers, and the client fails its pending calls and ends its events,
  which starts the desktop's reconnect. Both frames go with the reports: ahead of
  everything, outside the window, and never reported as read. Any byte read resets the
  clock, and a `link_credit` peer reports what it reads, so a link busy either way is
  never quiet; a ping goes only on a link idle both ways, which it crosses in a round
  trip. 30 s outlasts a cell handover and TCP's retransmit backoff across a stall of 10
  to 15 s, and a wrong give-up costs one reconnect, which resumes from the held cursors
  and offsets. 10 s also keeps a carrier NAT's mapping alive, for 8 bytes each way. With
  both directions held, a call stayed pending and the phone stayed a viewer past 90 s; now
  the call fails, the events end and the server drops the viewer 30 s in. A peer without
  `heartbeat` keeps today's behaviour, and SSH's `ServerAliveInterval=15` ×
  `ServerAliveCountMax=3` stays under it as a backstop.

What it does not cover:

- A client without `link_credit` still lets the link fill: over `ssh … relay` the SSH
  channel window holds up to ~2 MB that no priority can reorder.
- Past 16 large reads, or 64 other calls such as the board's per-PR `gh` polls
  (`pipelineData.ts`), the next call waits for one to finish.
- A request waiting for its slot is held whole, so a client that pipelines large
  requests past its slots grows the server by what it sends.
- A request over 16 KiB goes in parts, so a later small one overtakes it: an interrupt
  can reach the server before a long `send_user_message` it was meant to stop. The
  desktop sends attachments separately, so a message rarely reaches 16 KiB.
- A dropped `RpcClient` stops reporting, so the server writes to it only until its
  window fills, and holds the rest until the client's last frames are written and the
  connection closes, which the heartbeat bounds: the client no longer answers pings.
- A peer without `heartbeat` has no bound: over `ssh … relay` it has SSH's 45 s, and on
  any other transport none.
- Only `SessionRevised` gives way to a newer event. A flood of anything else, such as
  `SessionViewers`, still overflows a slow reader's queue and is reported missed.

The guard is `crates/hyprlayer-e2e/tests/connection_backpressure.rs`, on a proxy that
stalls the link and then throttles it to 256 KiB/s:

| Measure | Before | After |
|---|---|---|
| Server RSS growth, stalled reader with `yes`, 8 s | +82 to +161 MiB | +1 MiB |
| Small call after the stall ends | > 15 s (timed out) | 55 to 77 ms |
| `SessionRevised` during the stall: sent / delivered / reported missed | 484 / 0 / 0 | 484 / 210 / 276 |
| Small call behind a large response that takes ~13 s | > 30 s (timed out) | 40 to 95 ms |
| Client RSS growth, one stream unread for 6 s | +313 to +325 MiB | +0 MiB |
| Re-reads during a 12 s event flood at 32 KiB/s | 6, 2.3 s apart | 4, 5.0 s apart |

The heartbeat, the events' share and the revisions are guarded by
`crates/hyprlayer-e2e/tests/liveness.rs` and `connection_backpressure.rs` (2026-11-21).
Before is `67ef402f`:

| Measure | Before | After |
|---|---|---|
| Both directions held: a pending call fails, events end, server drops the viewer | never, at 45 s | 30.0 s, 30.0 s, 30.1 s |
| An idle link, a 16 KiB/s link printing `yes`, and a client without `heartbeat`, 45 s on | up | up |
| 128 KiB answer and longest terminal gap during a revision flood at 32 KiB/s (4.1 s alone) | 22.5 s, 4.9 s | 6.5 to 7.4 s, 0.9 to 1.0 s |
| Revisions to a stalled reader through two 120-step turns: delivered / missed / ends on the latest | 210 / 276 / no | 4 / 0 / yes |
| `SessionRevised` per 120-step turn to a device streaming it, and to one only watching | 242, 242 | 1, 242 |

The turns, the link window, the client's lanes and the request slots are guarded by
`crates/hyprlayer-e2e/tests/transport_fairness.rs` (2026-11-20). Before is `8edf381b`,
rerun on `b2eb8713`; the far-link rows' before is this change's fixed 128 KiB window, the
permission and uplink rows' its first adaptive window with one pool (`15df1bb6`), and the
listings row's a build that gave those calls no slot:

| Measure | Before | After |
|---|---|---|
| Server RSS growth, 64 pipelined 2 MiB reads, reader stalled | +279 MiB | +60 to +67 MiB |
| Small calls behind a 4.7 MiB response at 512 KiB/s | 46 ms, then 10.4 s | 26 to 54 ms |
| 27 KiB response behind a 2 MiB one at 256 KiB/s (108 ms alone) | 8.5 s | 0.27 to 0.28 s |
| Small call and keystroke echo during a 2 MiB upload, 256 KiB/s up | 9.1 s, 9.1 s | 0.45 to 0.46 s, 0.45 to 0.47 s |
| A terminal stream during that upload, 2 s | 0 | 766 KiB |
| Session turn beside a busy terminal at 256 KiB/s, against alone | 55× | 2.0× |
| Terminal tick gap and echo beside a 1.5 MiB session row | 6.6 s, 5.6 s | 0.17 to 0.22 s, 0.06 to 0.10 s |
| Small call on a 4 MiB-deep link: 3 terminals at 256 KiB/s | 7.4 to 7.8 s | 0.57 to 0.60 s |
| The same, 1 terminal at 64 KiB/s | 8.0 to 8.9 s | 1.9 to 2.1 s |
| 5.3 MiB read at a 100 ms round trip, no rate cap: first after two small calls, next | 4.6 s, 4.6 s (fixed window) | 0.37 to 0.42 s, 0.27 s |
| The same at 20 ms | 1.18 s, 1.18 s (fixed window) | 0.23 s, 0.18 to 0.19 s |
| Small call during that read on a 1.5 MiB/s, 100 ms, 4 MiB-deep link | 106 to 126 ms (fixed window) | 152 to 253 ms |
| Small call after 4 MiB/s drops to 256 KiB/s at 100 ms, three terminals | 0.5 s, one stream held to 1.2 MiB/s (fixed window) | 2.4 to 2.5 s, then 0.5 s |
| `respond_permission` behind 16 large reads at 256 KiB/s | 4.5 s | 66 to 67 ms |
| Small call, echo and event during a 4 MiB upload into a 2 MiB-deep uplink at 256 KiB/s | 1.9 s, 14.6 s, 7 ms | 0.51 to 0.52 s, 0.46 s, 2 to 9 ms |
| Server RSS growth, 512 pipelined 316 KiB listings, reader stalled | +125 MiB | +25 MiB |

### Terminal output and resume

A terminal stream used to carry each flush as `{"kind":"output","data":"<base64>"}`,
and every attach replayed the whole ring. Measured 2026-11-19 on main over three runs:
408,900 bytes of `seq` output crossed as 618,624 to 622,392 (1.51 to 1.52×) in 1,781 to
1,874 frames, and each reattach with no new output sent 348,381 to 349,697 bytes.

- **Raw output for a peer that reads it.** The hello's `features` gains `raw_terminal`.
  To a peer that listed it, an output event is `O`, the output's absolute offset as a
  `u64` little-endian, then the bytes (`frame::encode_terminal_output_frame`). The exit
  stays JSON, and JSON never starts with `O`, so a client reads both forms without
  tracking which it negotiated. Any other peer gets the JSON above, byte for byte.
  Credit counts what follows the stream id in either form, on both ends.
- **Attach from an offset.** `terminal_attach` takes an optional `offset`. While the
  ring holds it (head to tail), the stream continues from there and the response says
  `resumed: true`. Otherwise, or with no offset, it replays the scrollback as before,
  with `resumed: false`, which is also what an older server's response decodes to.
  Every output carries its start offset, and the offsets are what a client acts on:
  `resumed` is the ring's answer when the stream opened, and a burst can trim past the
  offset before the first read. The desktop starts a view over only when a reattach's
  first output doesn't begin at the offset it asked for, with a full reset queued behind
  the output already written; a gap later in a live stream keeps writing, as before.
  `TerminalEvent::Output` hands the webview the same `offset`.
- **Fewer, larger flushes.** Raw bytes alone gave 1.12×: a PTY delivers `seq` 14 bytes
  per read, so frames averaged 179 bytes and the 22-byte header was the excess. The PTY
  coalescer flushes at once, as before, until four flushes have gone out less than 5 ms
  apart; past that a burst flushes once per 5 ms (VS Code's `TerminalDataBufferer`
  interval) or at 64 KiB. Below the streak it never waits on a timer, which on Windows
  rounds up to the ~15.6 ms system tick: an echo takes 0.04–0.08 ms here as on main,
  where a 1 ms settle wait had made it 1.5 ms. The streak also lets a repaint the PTY
  splits across reads out whole; holding from the first flush delayed the rest of one
  32 KiB write by 7.7 ms. It holds bytes before the ring, not in it: a PTY delivers `yes`
  at ~150 MB/s here, so a 5 ms hold in front of the ring read would trim it.
- **Who resumes.** The desktop's agent runtime and `reconcileAgentTerminals` attach from
  the last offset they read. A mounted `TerminalTab`, agent tab or script view now
  reattaches from its own offset after a reconnect, where before its stream simply
  ended. A mount still replays the ring, because its terminal emulator starts empty.
- **The webview hop stays base64.** `TerminalStream` turns raw output into a
  `TerminalEvent` for the Tauri channel: 1.33× the output, in process.

The guard is `crates/hyprlayer-e2e/tests/terminal_resume.rs`:

| Measure | Before | After |
|---|---|---|
| Wire bytes per output byte, `seq 1 60000`, unthrottled | 1.51–1.52× (~1,800 frames) | 1.0013–1.0016× (9–10 frames); 1.0019× in 30 once a read fits one part |
| Reattach with no new output | ~349 KB | 114 bytes (`resumed: true`) |
| Reattach 1,000 bytes back | the scrollback | 1,136 bytes |
| Reattach at a trimmed offset | the scrollback, unannounced | the scrollback, `resumed: false` |
| A client without `raw_terminal` | base64 in JSON | the same bytes |

## What we're not doing

- No multi-user, and no identity that grants authority: the announced client identity
  is display-only.
- No network listener, TLS, bearer tokens, or certificate management. Filesystem
  permissions on the socket are the authorization; SSH is the only remote path.
  *(Amended 2026-10-06 — still no listener. Since ADR-0011 the server sends a bearer
  token to the Cloud API as a client; nothing reaches the server with one.)*
- No browser or mobile client — there is no listener one could reach, and adding one
  means adding authentication, which this decision deliberately does not design.
- No change to `TerminalEvent` or `SessionEvent`. *(Amended 2026-11-19 —
  `TerminalEvent::Output` gained `offset`, and its wire form is negotiated; see
  "Terminal output and resume" under The connection.)* `src/lib/bindings.ts` gains two
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
  *(Amended 2026-10-16 — that exception is gone. Only the server takes the profile
  lock, and a second copy of the app is stopped by single-instance or simply connects
  to the running server, so `runtime_in_use` now means the lock is held by something
  not serving the socket: a server still starting after the five spawn retries, a
  wedged one, or a headless process. Once it quits, Relaunch recovers. So `setup()`
  treats it like every other `Outcome::Blocked`: `AppState::failed`, and the recovery
  screen shows its message and remedy. No native dialog, no `process::exit`; see
  ADR-0006's "Native alerts".)*
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
    local receiver. *(Amended 2026-11-19 — it takes a `TerminalStream`, which reads
    either terminal form; see The connection.)*

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
    *(Amended 2026-10-07 — the lock test no longer builds it: it is now
    `tests/profile_lock.rs`, an integration test, so cargo builds the binary
    first and passes it as `CARGO_BIN_EXE_hyprlayer-server`.)*
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
    *(Amended 2026-10-07 — superseded: the test moved to `tests/profile_lock.rs`
    and runs `CARGO_BIN_EXE_hyprlayer-server`. Its nested `cargo build` re-linked
    `target/<profile>/hyprlayer-server` while tests running in parallel under
    `cargo nextest` spawned it.)*
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
