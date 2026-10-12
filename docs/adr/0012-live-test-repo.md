# ADR 0012: Hand-run live tests use the private `hyprlayer-live-tests` repo

**Status:** accepted · 2026-10-12

## Decision

Any test that opens, merges or closes a real pull request, or has real agents write
thoughts docs, runs against **`jtsiros/hyprlayer-live-tests`** (private), never a real
project. It is cloned at `~/hyprlayer/repos/hyprlayer-live-tests` and imported in the dev
profile (`ai.brightblock.hyprlayer.dev`); its thoughts map to `repos/hyprlayer-live-tests/`.

**Use it** when behavior depends on GitHub or agent timing: the PR watcher, merge, close and
archive flows, doc status settlement, Ship and describe_pr. Run it before claiming such a
change works live.

**Don't use it** for repeatable checks: those are `crates/hyprlayer-e2e` scenarios with a stub
`gh`. The release gate's `pr_lifecycle` keeps its own fixture, `jtsiros/hyprlayer-e2e-fixture`.

**How:**

- Drive the dev instance (`pnpm tauri dev --config src-tauri/tauri.dev.conf.json`), never the
  release app. Setup goes through Tauri IPC over the bridge on 9223 (`import_project`,
  `create_workstream`, `merge_workstream_pr`, `archive_workstream`). Sessions need a Channel
  the bridge can't pass, so start them with an `RpcClient` on the dev profile's
  `hyprlayer.sock`.
- The PR watcher polls only while the server is foreground and polls a background workstream
  every 5–20 min. Focus the dev window or send `set_foreground(true)` over the socket, and
  `watch_worktree` the workstream under test. Put both back afterwards.
- `thoughts/` is gitignored on `main` and `thoughts/shared/pr_description.md` exists, so a
  Ship turn commits only real changes and describe_pr can write its record.
- Its thoughts sync to the real thoughts remote: name docs `zz-…`.

## Rationale

Testing against a real project leaves throwaway PRs, merges and branches in its history and
`zz-` docs beside its real plans. One dedicated repo keeps all of that in one place, and its
setup (import, ignore rules, PR template) is done once. Private keeps test PRs out of view.

## Consequences

- The repo accumulates merged and closed test PRs; that is its purpose. Archive deletes merged
  branches.
- Its thoughts dir accumulates `zz-` docs; prune it when it gets noisy.
- Running these tests needs push access to the repo and an authenticated `gh`.
