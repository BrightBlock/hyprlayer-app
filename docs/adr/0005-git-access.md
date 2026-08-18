# ADR 0005: git2 for local reads; the system `git` binary for the network

**Status:** accepted · 2026-08-18

## Decision

Git access splits on whether an operation touches a remote.

- **Local, read-mostly work** — status, diffs, refs, branch and commit inspection —
  uses **git2**, declared `git2 = { version = "0.19", default-features = false }`
  in `crates/hyprlayer-core/Cargo.toml`. Turning default features off compiles
  out the ssh and https transports.
- **Anything touching a remote** — `clone`, `fetch`, `push`, `pull` — shells out
  to the **system `git` binary**, via `git_in(git, repo, args)`
  (`crates/hyprlayer-core/src/worktree/manager.rs:522`), which runs
  `git -C <repo> …` through `Exec`.
- The binary is resolved through the augmented-PATH resolver in
  `crates/hyprlayer-core/src/process/binaries.rs`, never assumed to be on `PATH`
  (a macOS GUI app inherits launchd's minimal one).

The split is about the *network*, not about the word "remote": reading
remote-tracking refs (`git2::BranchType::Remote` in `gitops.rs:109`) touches only
`refs/remotes/*` on disk and stays on the git2 side.

## Rationale

The user's git authentication does not live anywhere git2 can reach it. It is
spread across credential helpers (`osxkeychain`, `gh auth`'s helper), SSH agents
and 1Password's agent socket, per-host `~/.ssh/config` rules, and `insteadOf`
url rewrites. Driving the network through git2 would mean re-implementing each
of those, and failing differently from the `git` the user runs in their own
terminal — the failure mode is a silent auth error or an unexpected prompt in a
GUI app that has no TTY to prompt on.

Shelling out inherits all of it for free, plus commit signing, hooks, and
anything else the user has configured.

Compiling the transports out also drops libssh2 and vendored OpenSSL from the
dependency tree, for the same reason ADR-0001 declines to import the CLI crate.

## Consequences

- Remote operations need the resolved `git` binary. Its absence is a
  user-visible error, not a fallback to an in-process implementation.
- Remote operations are async process spawns returning exit-status-shaped
  results, not in-process calls returning typed git objects. Callers that need
  structured output parse porcelain.
- `default-features = false` is load-bearing. A git2 remote call against an
  https or ssh URL fails at runtime on an unsupported transport rather than
  quietly bypassing the user's credentials — a tripwire, not a compile error, so
  new remote work belongs on the `git_in` path by construction.
- Tests that build fixture repos pass `-c commit.gpgsign=false` so a user's
  global signing config (e.g. a 1Password SSH signing key that cannot sign
  non-interactively) cannot break a headless run.
