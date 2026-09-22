# hyprlayer-app

## Build & verify

| Command | What it does |
|---|---|
| `cargo fmt --all` | Formats the Rust workspace — **run before every push** |
| `cargo test` | Rust workspace tests (1,070, including the E2E scenarios) + regenerates `src/lib/bindings.ts` |
| `pnpm typecheck` | Frontend type-check (`tsc --noEmit`) |
| `pnpm lint` | ESLint (includes import-boundary rules) |
| `pnpm test` | Vitest (1,491 tests) |
| `pnpm build` | typecheck + vite build |

Always run all of these before pushing. The release gate is `scripts/release.sh`, which runs the same checks plus the E2E container tier (`cargo test -p hyprlayer-e2e -- --ignored`), so it needs Docker running.

**`cargo fmt --all` is not optional.** There is no CI to catch drift (ADR-0001 —
releases are built locally by design), so an unformatted commit stays
unformatted, and the next person to run the formatter gets a diff touching files
they never edited. The tree was reformatted wholesale once for exactly that
reason; keep it clean so it never has to happen again.

```bash
cargo fmt --all              # fix
cargo fmt --all -- --check   # verify (exit non-zero on drift)
cargo clippy --workspace --all-targets   # must be warning-free
```

Clippy is warning-free across the workspace. When a lint is genuinely wrong for
the code — a `#[tauri::command]` whose arity *is* the IPC contract, say — add
`#[allow(clippy::...)]` with a comment saying why, rather than reshaping a
signature the bindings depend on.

## Architecture: two thin shells

The backend is three crates (ADR-0008): `crates/hyprlayer-models` (dependency-light
serde contracts) → `crates/hyprlayer-core` (zero Tauri deps, all logic, exposed
through the `HyprlayerRuntime` facade) → `src-tauri` (thin IPC shell). The frontend
mirrors that same contract via ADR-0003: routes are wiring, features are content,
`src/lib` and `src/components` are the shared foundation.

`docs/architecture/rust-runtime-library.md` is the runtime's API reference: what the
supported surface is, what the 30 `pub mod`s below it are not, and the hermeticity
contract `HyprlayerRuntime::open` holds to.

**Import direction is one-way downward.** No layer may import upward.

```
main.tsx / router.tsx   → may import anything
src/routes/              → may import features, lib, components
src/features/            → may import lib, components; never routes
src/lib/ src/components/ → shared foundation; never imports features or routes
```

When a shared module needs feature-side behavior, expose a registration seam
(see `lib/events.ts` / `lib/appNav.ts` / `lib/queries.ts`) and wire it in `main.tsx`.

## Where things live

| If it... | Put it in... |
|---|---|
| Is a shared vocabulary type (Stage, Harness, DocKind, PermissionMode) | `src/lib/domain/` — derived from Rust enum bindings, never re-declared |
| Is a shared UI primitive (popover, dialog, empty hint) | `src/components/ui.tsx` — check here before building one |
| Is a shared visual convention (row height, hover, indent, dialog width) | `src/components/ui.tsx` — check the vocabulary in ADR-0006 before inventing a new value |
| Is a shared formatting helper | `src/lib/format.ts` |
| Is a shared path helper | `src/lib/path.ts` |
| Is a terminal/PTY concern | `src/lib/terminal/` |
| Is an agent-bundle (`~/.claude`) backup/restore/diff concern | `crates/hyprlayer-core/src/agent_bundle.rs` — pure and filesystem-only; the GitHub probe is fetched in `crates/hyprlayer-core/src/runtime/agents.rs` through the `ProviderHttp` port, since ADR-0008 moved every service's network calls into core |
| Is used by exactly one feature | that feature's directory |
| Is used by *two or more* features | graduate to `src/lib/` or `src/components/` |
| Is a route/page definition | `src/routes/` — keep it dumb: layout, params, pane wiring only. No domain types, no store imports. |

## Making a change

The ADRs below define the contracts. This is the mechanical part they leave out.

**A new IPC command.** Logic in `crates/hyprlayer-core`, then a thin
`#[tauri::command]` in `src-tauri/src/commands/` returning `CmdResult<T>`, added
to the `collect_commands![]` list at `src-tauri/src/lib.rs:36` — miss that and it
never reaches the bindings. Regenerate and commit per ADR-0001. Frontend side,
`unwrap(commands.yourCommand(...))` from `src/lib/ipc.ts` throws the typed error.

**Errors.** `CoreError` converts into the ADR envelope on its own; app-level
failures use the constructors in `src-tauri/src/error.rs`. The UI reads them back
with `errorMessage` / `errorRemedy`, so a `remedy` of `None` renders no
call-to-action.

**Migrations.** `MIGRATIONS` in `crates/hyprlayer-core/src/store/migrations.rs`
is an append-only array applied by `PRAGMA user_version`. Add a step; never edit
a shipped one.

**Frontend state.** Cached, shared server state goes through the react-query
hooks in `src/lib/queries.ts`. One-shot imperative calls (PTY attach, resize)
invoke `commands.*` directly from the component. Client state is zustand, in the
feature that owns it.

**Tests.** End-to-end first. `crates/hyprlayer-e2e` runs the real
`hyprlayer-server` binary, built from the tree, in a sandboxed profile and
drives it over its socket; it needs `git` and the hyprlayer CLI installed. Each
scenario writes `target/e2e/<scenario>/` and compares its `report.json` with
`crates/hyprlayer-e2e/golden/`: `HYPRLAYER_E2E_BLESS=1` rewrites a golden, and
the diff is reviewed like code. `HYPRLAYER_E2E_KEEP=1` keeps the sandbox.
`cargo test -p hyprlayer-e2e -- --ignored` adds the container tier: a Linux
host in Docker, reached through the same `ssh … relay` the desktop uses, with
the server built for it in a `rust:1-bookworm` container.
Isolated tests sit beside their code (`mod tests`, `*.test.tsx`) and in the 20
files of `crates/hyprlayer-core/tests/`, some driving real binaries. Those files
need the `test-support` feature, so `cargo test -p hyprlayer-core <name>` alone
runs none of them: narrow with `cargo test -p hyprlayer-core --features
test-support <name>`, or `pnpm test <path>`.

**Running it.** `pnpm tauri dev`. Beware: this can trigger the separately
installed release app's auto-updater, so a fresh mtime on the release binary is
not evidence your change shipped. Check the bundle's contents instead.

## Comments

Prefer names, types, and small functions that make comments unnecessary. Add a
comment only for a surprising platform/protocol constraint, safety invariant,
or tool directive that the code cannot express. Keep it short and place it
above the relevant code. Never narrate control flow, restate a signature, add
section banners, or preserve implementation history.

@docs/adr/0001-tauri-workspace-and-cli-types.md
@docs/adr/0002-config-ownership.md
@docs/adr/0003-frontend-layering.md
@docs/adr/0004-vocabulary-standard.md
@docs/adr/0005-git-access.md
@docs/adr/0006-visual-conventions.md
@docs/adr/0007-harness-capability-layer.md
@docs/adr/0008-rust-runtime-library-boundary.md
