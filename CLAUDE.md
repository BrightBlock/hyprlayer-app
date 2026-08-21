# hyprlayer-app

## Build & verify

| Command | What it does |
|---|---|
| `cargo fmt --all` | Formats the Rust workspace — **run before every push** |
| `cargo test` | Rust workspace tests (422) + regenerates `src/lib/bindings.ts` |
| `pnpm typecheck` | Frontend type-check (`tsc --noEmit`) |
| `pnpm lint` | ESLint (includes import-boundary rules) |
| `pnpm test` | Vitest (847 tests) |
| `pnpm build` | typecheck + vite build |

Always run all of these before pushing. The release gate is `scripts/release.sh` which runs the same checks.

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

The backend mirrors ADR-0001: `crates/hyprlayer-core` (zero Tauri deps, all logic) + `src-tauri` (thin IPC shell). The frontend mirrors that same contract via ADR-0003: routes are wiring, features are content, `src/lib` and `src/components` are the shared foundation.

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
| Is an agent-bundle (`~/.claude`) backup/restore/diff concern | `crates/hyprlayer-core/src/agent_bundle.rs` — pure and filesystem-only; the GitHub probe is in `src-tauri/src/commands/agents.rs`, ADR-0001 keeps core dependency-light |
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

**Tests.** Rust unit tests live in a `mod tests` beside the code; the seven files
in `crates/hyprlayer-core/tests/` are integration tests, some driving real
binaries. Frontend tests sit next to their component as `*.test.tsx`. Narrow to
one with `cargo test -p hyprlayer-core <name>` or `pnpm test <path>`.

**Running it.** `pnpm tauri dev`. Beware: this can trigger the separately
installed release app's auto-updater, so a fresh mtime on the release binary is
not evidence your change shipped. Check the bundle's contents instead.

## Comments

Match the density of the file you are editing. This codebase comments heavily —
16% of Rust lines, 5% of TypeScript — and the existing comments are the spec for
new ones. Do not strip them to look terse, and do not pad a diff to look thorough.

A comment earns its place when it records something the code cannot say. Four
things qualify:

**The failure that produced this code.** Name the version, platform, and symptom.

```rust
// The 0.5.8 Linux regression: the deb/rpm/AUR packages put the app at
// /usr/bin/Hyprlayer, so forbidding current_exe()'s parent blacklisted
// all of /usr/bin, `hyprlayer` and `git` resolved to nothing and `gh`
// fell through to a mise shim. A normal bin dir must forbid nothing.
```
`crates/hyprlayer-core/src/process/binaries.rs:531`

**The alternative that was rejected, and why.** The reader's first instinct is
usually the thing you already tried, so name it: `crates/hyprlayer-core/src/repos.rs:235`
explains why it uses `gh api /user/repos` and not `gh repo list`.

**The constraint that makes obvious-looking code wrong.**

```ts
// Build the binary string in chunks: `String.fromCharCode(...bytes)` spreads
// every byte as an argument, which overflows the call stack on a large paste.
```
`src/lib/terminal/terminalCodec.ts:10`

**The accepted gap.** Say what the code does not handle and why that is fine:
`crates/hyprlayer-core/src/opencode/config.rs:118` records that a JSONC config is
left untouched rather than risk dropping the user's comments.

Cite the receipt where one exists — `issue #1011`, `PR #221`, `ADR-0002`,
`0.5.8`. A comment that survives is one a reader can go verify.

### What does not earn a comment

- Restating the next line. `// Increment the counter` above `count += 1`.
- Narrating the diff. No `// Added to fix the bug`, `// Changed from the old
  approach`, `// New helper`. Git already knows. Comments describe the code as
  it stands, not its history.
- Section headers inside a function — `// --- Validation ---`. If a function
  needs signposting, it needs splitting.
- Restating a type the signature already declares.
- Hedging about the code's own quality: `// This could probably be optimized`,
  `// Note that this is a simple implementation`.

### Form

- Above the code, never trailing at end of line.
- Doc comments (`///`, `//!`) carry the contract: what the caller must know,
  edge cases, degradation behavior. Inline `//` carries the local surprise.
- Module docs (`//!`) explain how the module fits the whole, not what its
  functions are named.
- Write the prose like the rest of the codebase: active voice, no adverb
  padding, no throat-clearing openers, no "it's not just X, it's Y".
- Delete a comment when the code it describes changes. A stale comment costs
  more than no comment.

@docs/adr/0001-tauri-workspace-and-cli-types.md
@docs/adr/0002-config-ownership.md
@docs/adr/0003-frontend-layering.md
@docs/adr/0004-vocabulary-standard.md
@docs/adr/0005-git-access.md
@docs/adr/0006-visual-conventions.md
