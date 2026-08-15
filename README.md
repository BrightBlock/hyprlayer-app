# Hyprlayer: Desktop App for Spec-Driven AI Development

## Overview

Hyprlayer is a native desktop app for spec-driven development with AI coding
agents — research, plan, implement, validate, and ship without bouncing between
tools. Each piece of work runs in its own isolated git worktree with an embedded
Claude Code session, while your knowledge base (thoughts) and your team's
collaboration ride the storage backend you already use.

It pairs with the [hyprlayer](https://brightblock.ai/hyprlayer) CLI: the CLI owns
your config and your thoughts storage; the app gives you a workspace on top of it.

## Key Features

**The Observatory (home):**
- Your knowledge graph as a full-bleed, interactive home surface — every
  document a star colored by its type, every resolved link a strand; scroll to
  zoom, drag to pan, hover to spotlight, click to open
- Live agents pinned onto the field, stage-colored cards for what's running
  right now, and a feed of the latest workstream activity
- Global search and corpus stats front and center; a guided first-node state
  points new users at the setup wizard until a repo is connected

**The Pipeline:**
- Spec-driven stages — research → plan → implement → validate → ship — each
  backed by a Claude Code skill
- One isolated git worktree per work item, provisioned and torn down for you
- One-click Ship: commit the worktree, push the branch, and open a pull request
- Per-session model and reasoning-effort choices (Opus, Sonnet, Haiku, Fable),
  including 1M-context variants

**Embedded Agent Sessions:**
- Claude Code runs in-app with a live transcript, tool calls, and a context meter
- Permission prompts, agent questions, interrupt, resume, and kill — all in the UI
- Attach knowledge documents to a work item so the agent starts with context
- File and image attachments for prompts

**Knowledge Library:**
- Browse, search, and edit your thoughts as first-class documents with frontmatter
- Templated document creation and a Markdown preview with GitHub-flavored rendering
- Documents link into the home Observatory's graph via `related` frontmatter
  and shared tags/tickets

**Git & Review:**
- Fork-style Git panel for staging, unstaging, and discarding changes
- Side-by-side and unified diffs with syntax highlighting and an in-app file viewer
- Per-worktree status so you always see exactly what an agent changed

**Storage Backends:**
- Bring your own backend: Git, Obsidian, Notion, or Anytype
- The app reads and watches the CLI's config so your projects and thoughts stay
  in sync

## Platform Support

- **macOS** — Apple Silicon `.dmg`, Developer ID–signed and notarized, with
  in-app auto-updates
- **Windows** — x64 NSIS installer (Tauri 2 + WebView2), with in-app auto-updates
- **Linux** — x64 distro packages (Tauri 2 + WebKitGTK 4.1). Arch is the primary
  target via the AUR; `.deb` and `.rpm` are published for everyone else. No
  AppImage.

```sh
paru -S hyprlayer-desktop-bin            # Arch (or yay)
sudo apt install ./Hyprlayer_*_amd64.deb # Debian / Ubuntu
sudo dnf install ./Hyprlayer-*.x86_64.rpm # Fedora / RHEL / openSUSE
```

Each package pulls in WebKitGTK 4.1, GTK 3, `git` and `xdg-utils`. The `.deb`
and `.rpm` are built on Arch, so they need a reasonably current glibc — Ubuntu
24.04, Debian 13 or Fedora 40 and newer.

On macOS and Windows, running apps poll a signed manifest and install updates in
place; there is no separate update step. Linux packages belong to the package
manager, so the app only tells you a new version is out — `paru -Syu`, `apt` or
`dnf` installs it.

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

To exercise the app against real data, install the hyprlayer CLI and initialize
a repository:

```sh
brew install brightblock/tap/hyprlayer   # or scoop/winget on Windows
cd ~/code/your-repo && hyprlayer thoughts init
```

See `docs/RELEASING.md` for how releases and in-app updates are published.
