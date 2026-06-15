# Hyprlayer (desktop app)

Native macOS + Windows companion app for [hyprlayer](https://brightblock.ai/hyprlayer) —
spec-driven development with AI agents, without bouncing between tools. Research, plan,
implement, validate, and ship from one place, with your knowledge base (thoughts) and your
team's collaboration riding the storage backend you already use.

**Status: pre-alpha.** Current milestone (Phase 0): environment bootstrap — the app reads your
hyprlayer CLI config, lists mapped projects, verifies the toolchain (`hyprlayer`, `claude`,
`git`, `gh`), runs backend health checks, and can sync thoughts. Knowledge library, work
items with isolated worktrees, embedded Claude Code sessions, document review/comments, and
PR shipping land in the next phases — see `docs/` and the implementation plan.

## Architecture

- **Tauri 2** shell (`src-tauri/`): thin IPC layer; commands/events exported to TypeScript via
  [tauri-specta](https://github.com/specta-rs/tauri-specta) into `src/lib/bindings.ts`
  (generated — do not edit).
- **`crates/hyprlayer-core`**: all real logic, no Tauri dependencies. Re-declares the hyprlayer
  CLI's config schema (the CLI has no lib target), resolves binaries (GUI apps don't inherit
  your shell PATH), and wraps `hyprlayer` CLI invocations.
- **React 19 + TypeScript** frontend (`src/`): TanStack Router + Query, zustand, Tailwind v4.

Ownership rules worth knowing:

- The hyprlayer **CLI owns its config file**; the app reads and watches it.
- The app never edits thought documents behind your back; documents live in your thoughts
  storage (git repo / Obsidian vault), never in the app.

## Development

Prerequisites: Rust (stable), Node 22+, pnpm 11+, and the
[Tauri platform deps](https://tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm tauri dev          # run the app (debug)

cargo test --workspace  # rust tests (also regenerates src/lib/bindings.ts)
pnpm typecheck && pnpm lint && pnpm test   # frontend checks
pnpm tauri build        # production bundle
```

To exercise the app against real data, install the hyprlayer CLI and initialize a repository:

```sh
brew install brightblock/tap/hyprlayer   # or scoop/winget on Windows
cd ~/code/your-repo && hyprlayer thoughts init
```
