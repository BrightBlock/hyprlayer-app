# hyprlayer-app

## Build & verify

| Command | What it does |
|---|---|
| `cargo fmt --all` | Format the Rust workspace — **run before every push** |
| `cargo clippy --workspace --all-targets` | Must be warning-free |
| `cargo test` | Rust tests, including E2E scenarios; regenerates `src/lib/bindings.ts` |
| `cargo nextest run --workspace` | The same tests, one process each, in parallel; optional locally, required by the release gate |
| `pnpm typecheck` / `pnpm lint` / `pnpm test` | tsc, ESLint (enforces import boundaries), Vitest |
| `pnpm build` | typecheck + vite build |

Run all of these before pushing. There is no CI (releases are built locally), so
unformatted code or a drifted `bindings.ts` stays in the tree until someone trips on
it. The release gate, `scripts/release.sh`, runs the Rust tests as one `cargo nextest
run --run-ignored all`, which adds the container tier and the desktop's ssh sign-in;
it needs Docker and cargo-nextest 0.9.145+ (`brew install cargo-nextest`).

When a clippy lint is wrong for the code (a `#[tauri::command]` whose arity *is* the
IPC contract), add `#[allow(clippy::...)]` with a comment saying why rather than
reshaping a signature the bindings depend on.

## Architecture

```
hyprlayer-models → hyprlayer-core → hyprlayer-server → src-tauri (hyprlayer-app)
 serde contracts    all logic,        one runtime,       thin client: IPC,
                    HyprlayerRuntime  socket/SSH relay   desktop integrations
                    facade, no Tauri
```

`hyprlayer-proto` holds the wire protocol; `hyprlayer-e2e` drives the real server.
`docs/architecture/rust-runtime-library.md` is the runtime API reference.

The frontend mirrors this: routes are wiring, features are content, `src/lib` and
`src/components` are the shared foundation. Imports only point downward
(see `.claude/rules/frontend-layering.md` before editing `src/`, and
`.claude/rules/ia-vocabulary.md` for the naming glossary; Claude Code loads these
automatically, other agents should read them).

## Where things live

| If it... | Put it in... |
|---|---|
| Is a shared vocabulary type (Stage, Harness, DocKind, PermissionMode) | `src/lib/domain/`, derived from Rust bindings, never re-declared |
| Is a shared UI primitive or visual convention (popover, dialog, row height, hover) | `src/components/ui.tsx` — check there and ADR-0006 before inventing one |
| Is an icon | `<Icon name size>` from `src/components/Icon.tsx`: add a semantic key to its registry, never import `lucide-react` or draw an `<svg>` |
| Is a shared formatting / path helper | `src/lib/format.ts` / `src/lib/path.ts` |
| Is a terminal/PTY concern | `src/lib/terminal/` |
| Is an agent-bundle (`~/.claude`) backup/restore/diff concern | `crates/hyprlayer-core/src/agent_bundle.rs`; its GitHub probe goes through `ProviderHttp` in `runtime/agents.rs` |
| Is a skill install, source, lock or link concern | `crates/hyprlayer-core/src/skill_sources/` via `SkillService` |
| Is a Hyprlayer Cloud account or sign-in concern | `crates/hyprlayer-core/src/account/` + `runtime/account.rs`, per `docs/adr/0011-cloud-account.md`; the loopback runs from `src-tauri/src/commands/account.rs` |
| Is used by one feature / by two or more | that feature's directory / graduate to `src/lib/` or `src/components/` |
| Is a route | `src/routes/` — layout, params, pane wiring only; no domain types or store imports |

## Making a change

**A new IPC command.** Logic in a `hyprlayer-core` runtime service. Add a line to the
command table in `crates/hyprlayer-server/src/rpc.rs` and implement it in
`crates/hyprlayer-server/src/dispatch.rs`. Then add a thin `#[tauri::command]` in
`src-tauri/src/commands/` that calls `state.client()?.<command>(…)` and returns
`CmdResult<T>`, and list it in `collect_commands![]` in `src-tauri/src/lib.rs` — miss
that and it never reaches the bindings. Run `cargo test` (or `cargo nextest run`) to
regenerate and commit `bindings.ts`. On the frontend,
`unwrap(commands.yourCommand(...))` from `src/lib/ipc.ts` throws the typed error.

**Server versioning.** Changes under `crates/hyprlayer-{server,core,models,proto}`
need a server version bump before release (`release.sh` enforces it). Raise
`MIN_SERVER_VERSION` only when the app depends on the change.

**Errors.** `CoreError` converts into the error envelope on its own; app-level
failures use the constructors in `src-tauri/src/error.rs`. The UI reads them with
`errorMessage` / `errorRemedy`; a `remedy` of `None` renders no call-to-action.

**Migrations.** `MIGRATIONS` in `crates/hyprlayer-core/src/store/migrations.rs` is
append-only, applied by `PRAGMA user_version`. Add a step; never edit a shipped one.

**Frontend state.** Shared server state goes through the react-query hooks in
`src/lib/queries.ts`. One-shot imperative calls (PTY attach, resize) call
`commands.*` directly. Client state is zustand, in the feature that owns it.

**Tests.** End-to-end first. `crates/hyprlayer-e2e` runs the real `hyprlayer-server`
in a sandboxed profile (needs `git` and a released hyprlayer CLI: each sandbox is seeded
from that release's agent bundle in `target/e2e/cli-assets/`, fetched once per CLI version). Each
scenario writes `target/e2e/<scenario>/` and compares its `report.json` with
`crates/hyprlayer-e2e/golden/`: `HYPRLAYER_E2E_BLESS=1` rewrites a golden (review the
diff like code), `HYPRLAYER_E2E_KEEP=1` keeps the sandbox, and `HYPRLAYER_E2E_SERVER_BIN`
runs a prebuilt server instead of building one per test process (once per run under
nextest; the gate points it at the sidecar build). The `--ignored` container
tier runs a Linux host in Docker over the same `ssh … relay` the desktop uses; the
desktop's own ssh sign-in (askpass prompts, shared connections) is
`src-tauri/tests/ssh_sign_in.rs`, in the app package because its helper is the app binary.
Under nextest a container test outside the `docker` test group in `.config/nextest.toml`
panics: that group caps how many share the Docker VM.
Isolated tests sit beside their code (`mod tests`, `*.test.tsx`) and in
`crates/hyprlayer-core/tests/`, which need the feature flag:
`cargo test -p hyprlayer-core --features test-support <name>`.

**Running it.** `pnpm tauri dev`. This can trigger the installed release app's
auto-updater, so a fresh mtime on the release binary is not evidence your change
shipped; check the bundle's contents.

## Comments

Prefer names, types, and small functions over comments. Comment only a surprising
platform/protocol constraint, safety invariant, or tool directive, briefly, above the
code. Never narrate control flow, restate a signature, add section banners, or keep
implementation history.

## ADRs

The decisions in `docs/adr/` are binding. They are long, so they are not loaded
automatically: **read the relevant ADR before changing its area.**

| ADR | Read before touching | The rule in one line |
|---|---|---|
| 0001 | workspace layout, `bindings.ts`, CLI config types | Bindings are generated by `cargo test` and committed; CLI config types are re-declared in `core/src/config/types.rs`, not imported |
| 0002 | anything that reads or writes the CLI's `config.json` | The CLI is its only writer; the app reads, watches, and mutates through CLI commands (two named exceptions) |
| 0003 | frontend imports, registration seams | Imports point downward only; the foundation reaches features through seams wired in `main.tsx` |
| 0004 | closed-vocabulary enums, naming | Vocabulary enums live in Rust with serde + specta; commands are `domain_verb`, PTY commands `pty_*` |
| 0005 | git operations | git2 for local reads; the system `git` binary (`git_in`) for anything touching a remote |
| 0006 | UI styling: dialogs, rows, hover, menus, type scale, icons | `Dialog` takes `size`, never a `max-w-*` class; rows use `rowClass`; floating panels use `Popover`; Tailwind's `text-xs`…`text-3xl` don't exist — use the named type tokens |
| 0007 | stage guardrails, harness launch policy, codex | `capability::launch_policy` is the single stage → harness mapping; codex floor is 0.155.1 |
| 0008 | crate boundaries, runtime facade, core hermeticity | No Tauri in core; no HTTP client or `Db` handle in the shell; core reads no ambient env/home paths |
| 0009 | `hyprlayer-server`, RPC, remote hosts, server versioning | The runtime runs in the server; the desktop is always a client |
| 0010 | skill installs, skill stores, skills.lock | Installed skills live in app-owned stores reached by per-skill links; the CLI must not claim them |
| 0011 | Cloud sign-in, `account.json`, the Cognito pools | The server holds the session and the PKCE verifier; the desktop only runs the loopback and the browser; the pool follows the build profile |
