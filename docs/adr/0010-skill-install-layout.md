# ADR 0010: Installed skills live in app-owned stores reached by per-skill links

**Status:** proposed · 2026-10-05 · amended 2026-11-11 (the server's embedded orchestration plugin is per-launch, not an installed skill)

## Decision

The server installs a skill from a source (a GitHub link now, skills.sh later) into a
store the app owns, then links it into the folders the harnesses read. The CLI never
manages these stores.

| | Real copy | Links | Lock |
|---|---|---|---|
| Global | `<config>/skills/<f>/` | `<claude_config_dir>/skills/<f>` and `<home>/.agents/skills/<f>`, absolute | `<config>/skills.lock` (has `installedAt`) |
| Project | `<checkout>/.agents/hyprlayer/skills/<f>/` | `.claude/skills/<f>` → `../../.agents/hyprlayer/skills/<f>`, `.agents/skills/<f>` → `../hyprlayer/skills/<f>`, relative | `<checkout>/.agents/hyprlayer/skills.lock` (no `installedAt`) |

`<config>` is the directory of the CLI's `config.json`, derived as `AgentService::store_root`
does, so there is no new path field. Project scope is the project checkout the Workshop
scope picker names, never a workstream worktree; workstreams see the skill once it is
committed and merged. An install only changes the working tree and stages nothing.

**Folder name is the key.** `<f>` is the preview's `name` at first install. It is the lock
key and the remove/update key, and an update never changes it.

**Ownership.** A link path is owned in exactly three cases: it is a link whose target
`agents::link_targets_store` places under that scope's store; it is a real directory
(the copy fallback) whose folder name has a lock entry and whose content hash equals the
lock's; or it is a small regular file whose trimmed content resolves to that skill's store
dir (a Windows checkout of a committed symlink). `installed` on a row, the collision test and `remove_skill` share this one
predicate. Nothing else is ever deleted.

**The CLI boundary.** `<config>/skills/` and `.agents/hyprlayer/skills/` are app-owned. They
are never under `hyprlayer/agents/`: the CLI treats any link into `agents/` as its own
(`link_targets_store`, CLI `src/agents.rs:1326-1340`, and its farm reconciliation). A future
CLI must not widen either over these stores. The sibling store is not a `config.json`
write, so ADR-0002 needs no exception.

**Windows.** Global: `symlink_dir`, then a junction on OS error 1314, then a copy. Project:
`symlink_dir`, then a copy (a junction is absolute and useless checked in). A copy is
recognised only through the lock and hash, and an update refreshes it.

**Locking.** Every install, remove, update and restore holds a per-scope async mutex for
its whole duration, keyed by the canonical path of `<config>/skills` or
`<checkout>/.agents/hyprlayer`. Previews and listing never take it.

**No new event.** A new `RuntimeEvent` kind disconnects desktops older than v0.8.6
(`client.rs:401-409`). Preview then install is the progress shape: preview stages the
network work under `<app_data_dir>/skill-staging/<token>/`, install commits from staging
and returns a step report. Tokens are single-use: install claims one by renaming its
directory and deletes it on every exit, so a failed install needs a fresh preview. Staging
older than an hour is swept at every preview.

**Not a store.** The `<app_data_dir>/agent-control/plugin/<version>/` dir is server-owned,
versioned and passed per launch; it is not a store, has no lock entry, and never links into
`~/.claude`.

## Updates and restore

A branch ref floats: `ls-remote` gives its tip and `update_available` is `tip != commit`. A
tag or SHA ref is pinned and never reports an update; only installing a different ref moves
it. An unreachable repo reports no `latest`. An update preview fetches the ref's tip, keeps
the installed folder name whatever the frontmatter says now, and carries a file-level change
summary. Its `preview.json` records `replaces: <scope>`, and installing that token takes the
replace branch: the store swaps in place, copy-fallback links are refreshed from the new
store (old kept until the end, undone in reverse on failure), missing links return, and the
lock's `commit`, `hash` and `installedAt` move. The lock's targets are authoritative.

Restore repairs from the lock. A store that is missing, or whose content hash differs from
the lock's (an update cut off before its lock write), is refetched at the locked commit:
matching the lock's hash it is moved in, setting the stale store aside until it is, otherwise
it returns for approval through the same replace branch. Missing, dangling and text-file links are re-created (symlink, then copy); a foreign
entry at a link path is never touched. On Windows a repaired project link shows as modified
in `git status` until the checkout supports symlinks. Entries with an unknown `source` are
skipped everywhere. `skillfish.json` is read-only: candidates come from the checkout's and
the home's file, and nothing ever writes it.

## Lockfile

```json
{ "skills": { "<f>": { "commit", "hash", "installedAt", "path", "ref", "repo", "source", "targets" } }, "version": 1 }
```

Keys are sorted; `installedAt` is global-only. `repo` is `owner/repo`, never a URL. `source`
is `github` or `skills.sh`; an unknown value is listed but never updated or restored, and an
unknown target is dropped. Writes are atomic under the scope lock, and an empty lock
deletes the file. A `version` above 1 is refused.

## Trust and portability

The preview lists every file and flags scripts, executables and `Bash` pre-approval. Only
regular files (`100644`/`100755`) are fetched, hashed and installed; symlinks and
submodules are never copied. The content hash is
`sha256(for each file in byte order of its "/"-joined path: path ‖ 0x00 ‖ u64 BE length ‖ bytes)`.
The mode bit is excluded so a Windows checkout and the lock agree. Every git run disables
prompts and user hooks, and sets `core.autocrlf=false`, `core.eol=lf` and no LFS smudge, so
bytes land exactly as committed on every host. Caps: 500 files, checked on trees before any
blob moves, and 20 MiB, checked after the batched checkout.

## Test knobs

`HYPRLAYER_SKILLS_GIT_BASE` and `HYPRLAYER_SKILLS_API_URL` (the skills.sh base) are
injected-env overrides beside `CLAUDE_CONFIG_DIR`. They are read only through
`RuntimeEnvironment::var`, never the env overlay, and each warns once when set. The git base
accepts only `https://` and `file://`; the API base accepts `https://` or loopback `http://`.
Anything else is ignored. An overridden git base is returned as the preview's `fetchedFrom`.

skills.sh search is the unauthenticated `GET /api/search`; `/api/v1` needs a Vercel OIDC token
and is not used. A pick installs from its `owner/repo` through the same git path, so the git
base knob also redirects it.
