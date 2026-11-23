---
name: verify-skill-installs
description: Model-check skill install, update (replace), remove and restore against crashes, failed steps and other people's files with TLA+ (ADR-0010). Use before merging any change to skill_sources/install.rs (commit, replace, remove, adopt_into_store, the undo structs), skill_sources/layout.rs (link_state, owned, link_skill, relink, sweep_leftovers, sweep_store, lock_state), skill_sources/lock.rs's write, or SkillService::restore_entry, and whenever a skill is reported stuck installed, impossible to remove, or out of step with its lock.
---

# Verify skill installs

`SkillInstalls.tla` is a model of one skill folder in one scope: its store dir,
its two link paths (`.claude/skills/<f>` and `.agents/skills/<f>`) and its lock
entry, as install, update, remove and restore change them. Every filesystem step
can fail, which runs that operation's undo, or be cut off by a crash, after which
nothing runs. Between operations, someone else may delete the store or a link, or
put their own entry at a free link path. Operations never overlap, because each
holds the per-scope mutex (`skill_locks.lock(&key)` in `runtime/skills.rs`).
TLC checks every order of those steps up to the bounds in the two configs:
`SkillInstalls-unix.cfg` (links are always symlinks) and `SkillInstalls-windows.cfg`
(a link may fall back to a copy of the store).

## The rules

| Rule | In one sentence | The harm when it breaks |
|---|---|---|
| `ForeignUntouched` | An entry someone else put at a link path is never deleted or replaced. | The app deletes a person's own skill. |
| `AppEntriesOwned` | Between operations, every entry the app made at a link path is one the ownership predicate still claims. | A skill the harness still loads that the app can no longer remove, update or restore. |
| `FinishedOpsConsistent` | A finished install, update or restore leaves the store's content equal to the lock's and every link resolving to it; a finished remove leaves only other people's entries. | The lock (committed, in project scope) says one version while the checkout runs another, or a remove leaves the skill behind. |
| `LeftoversSwept` | A finished operation leaves no `.tmp-` or `.old-` dirs from earlier crashes. | Disk leak; in project scope, untracked dirs inside the checkout that can get committed. |
| `TypeOK` | The variables stay in their domains. | None, it only guards the model. |

## Run it

From the repo root:

```sh
.claude/skills/verify-skill-installs/check.sh
```

It needs `java` (11+) and `mise`. It installs TLC 1.7.4 through mise, pinned by
checksum (set `TLA2TOOLS=/path/tla2tools.jar` to skip mise). It runs TLC once per
rule per config and compares `<PASS|FAIL> <platform> <rule>` lines with
`expected.txt`, exiting non-zero when they differ. It takes about 13 s on 16 cores.
`OUT=dir` keeps the logs: `$OUT/<platform>/<Rule>/tlc.out`.

**Read every result with the pstack `/statechart` skill.** Give it this spec, one
of the two configs, and the view module `statechart-view.ts` beside it. It prints
each broken rule as its shortest path in plain words. Add a label in the view for
any new action. `check.sh` gives the verdict, and `/statechart` gives the story.

## Known findings (the baseline in `expected.txt`)

Every rule in `expected.txt` passes on the code at the sync commit. The findings below
were about the product, not modeling errors, and are kept for the record. Each path was walked against the code; none
was replayed on a real filesystem.

1. **Fixed: a crash mid-update left new content under the old lock, and restore called it fine.**
   `replace` moves the new store in before it writes the lock, so a crash in between
   leaves the store at the new commit and `skills.lock` at the old one. `lock_state`
   and `restore_entry` now compare the store's `content_hash` with the lock's `hash`
   (`Layout::store_matches`), and a store that differs is refetched at the locked
   commit like a missing one: `adopt_into_store` sets the stale store aside, renames
   the fetched copy in, and removes the old one (or renames it back on failure).
   `FinishedOpsConsistent` passes on unix; on Windows it passed only once finding 3 was fixed.
2. **Fixed: store-side leftovers were never swept.**
   A crash leaves `.tmp-<token>-*` (fresh install, update or adopt) or
   `.old-<token>-*` (update or restore) inside the store dir, and `skills.lock.tmp-*`
   beside the lock. Install, update, remove and restore now call `Layout::sweep_store`
   under the scope mutex before they start; it removes only names with exactly those
   prefixes. Remove and restore also run `sweep_leftovers` for their folders, which
   only update did before: without it the Windows model still fails, through a
   crashed copy refresh's `.<folder>.tmp-`/`.old-` left beside a link and then a
   finished remove or restore. `sweep_staging` is unchanged. `LeftoversSwept` passes
   on both platforms. The model has no `skills.lock.tmp-*` (the lock write is one
   atomic action), so that part of the sweep is checked only by reading the code.
3. **Fixed: a crashed copy-fallback link was disowned forever (Windows).**
   `link_state` claimed a copy only while the lock had an entry whose hash matched it,
   so a crash after the copy but before `lock::write` (a fresh install), or after
   `refresh_copy` but before the lock moved (an update), left a copy that read as
   `Foreign`: remove left it, restore skipped it, install refused the name, and the
   harness still loaded it. Every copy now carries `COPY_MARKER` (`.hyprlayer-copy`),
   holding the content hash of the rest of the copy, written into the tmp copy before
   it is renamed into place (`link_skill`'s copy fallback and `refresh_copy`). A copy
   whose marker matches its content but not the lock's hash is `LinkState::Stale`:
   owned, so remove takes it and install refuses the name, `lock_state` calls it
   `Broken`, restore replaces it, and update refreshes it. A folder without the marker
   is still claimed only by the lock's hash, as before, so a person's own folder stays
   `Foreign`; an edited copy no longer matches its marker and turns `Foreign` too.
   Install now also runs `sweep_leftovers` for its folder, since the copy fallback's
   tmp sits beside the link. `AppEntriesOwned` and `FinishedOpsConsistent` pass on
   both platforms.

When one is fixed in the code, change the model to match, run `check.sh`, and flip
that rule's lines in `expected.txt` to `PASS`.

## Does a change touch the verified code?

Run this from the repo root:

```sh
git diff --stat b34aba24157c3e53870f9aa0296b5878e8e79ece -- \
  crates/hyprlayer-core/src/skill_sources/install.rs \
  crates/hyprlayer-core/src/skill_sources/layout.rs \
  crates/hyprlayer-core/src/skill_sources/lock.rs \
  crates/hyprlayer-core/src/runtime/skills.rs
```

Anything listed means: read the diff against the table. Some changes do not move
the model: previews, fetching, hashing rules, skills.sh search, frontmatter, and
the listing's presentation. These do move it: the order of filesystem steps, what
an undo puts back, the ownership predicate, what restore or relink repairs, what
gets swept, and which calls hold the scope mutex.

| Model action | Code (at the sync commit) |
|---|---|
| `Owned`, `State` (a copy is `healthy` or `stale`, never `foreign`) | `layout::owned` / `link_state`, layout.rs:53/83; the marker check, `COPY_MARKER` / `mark_copy`, layout.rs:75/78 |
| `Linked` | `layout::link_skill`, layout.rs:369 (symlink; on Windows junction or copy; the copy is `copy_dir`, layout.rs:399: tmp beside, `mark_copy`, rename) |
| sweep at each start (`storeJunk' = 0`; install, remove and restore also `linkJunk' = 0`) | `Layout::sweep_store` (layout.rs:238) in `SkillService::restore`/`install`/`remove` under the mutex, runtime/skills.rs:521/700/718; `sweep_leftovers` (layout.rs:219) there for remove and restore, and in `install::commit` (install.rs:304) for install |
| `InstallStart` … `InstallLock` | `install::commit`, install.rs:245: the precheck, `remove_dir_all(store_dir)`, tmp copy, rename, links, `lock::write` |
| `InstallUndo` | `Undo::run`, install.rs:219 |
| `ReplaceStart` … `ReplaceClean` | `install::replace`, install.rs:500: `sweep_leftovers`, set the store aside, tmp copy and rename, per-link `refresh_copy` (tmp copy, `mark_copy`, set aside, rename)/`set_aside`/`link_skill` (480/491), `lock::write`, `remove_any` |
| `ReplaceUndo` (each set-aside link comes back as it was, recorded in `swapped`) | `ReplaceUndo::run`, install.rs:455 |
| `RemoveStart` … `RemoveLock` | `install::remove`, install.rs:415 (links, store, lock; no undo) |
| `RestoreStart` … `RestoreRelink` | `SkillService::restore_entry`, runtime/skills.rs:539: `Layout::store_matches` (layout.rs:234) → `adopt_into_store` (install.rs:615: tmp copy, set a stale store aside, rename, `remove_any`) → `Layout::relink` (layout.rs:283) |
| atomic lock write | `lock::write`, lock.rs:73 (tmp + rename) |
| one operation at a time | `skill_locks.lock(&key)` in `restore`/`install`/`remove`, runtime/skills.rs:518/698/716 |

Granularity: each action is one filesystem call (a rename, a symlink, a copy into
a fresh tmp, a `remove_dir_all`, the lock's tmp-and-rename) or a read-only check.
An undo is one action. That is coarser than the code, where each undo step is a
separate `let _ =` call; see the next section.

## Bounds and what the model leaves out

- One folder, two versions (`A`, `B`), both link targets, at most 1 crash, 1 failed
  step and 2 outside changes per behavior.
- Left out: two link targets that share one physical dir (`link_paths` dedup);
  text-file links in the initial state (only restore and update handle them);
  the junction step (it behaves like a symlink here); the copy fallback's tmp,
  marker write and rename (one step here, so no leftover of its own); a copy
  someone edited (its marker stops matching and it reads as foreign); an undo that itself fails
  or is cut off part-way; power loss (no fsync anywhere); restore's
  `NeedsApproval` (a locked commit whose content no longer matches its hash);
  unknown `source` entries; two processes on one checkout (the mutex is
  in-process); and git switching branches under the files.
- A PASS covers these bounds only.

## Real bug or modeling error?

Walk the trace (from `/statechart`, or `$OUT/<platform>/<Rule>/tlc.out`) through
the code, one action at a time. If every step can happen, it is a bug: report it
in the code's words, and do not weaken the rule or the model. If a step cannot
happen, the model is wrong. For example, an operation must start with fresh locals,
the way a new call does. Fix the action, re-run, and say what you changed.

## When a change is intended

1. Change the actions that mirror the changed code, keeping one action per
   filesystem call, and update the table.
2. Prove the rule you touched can still fail. Break that path in the model on
   purpose (for `ForeignUntouched`, have `RemoveStart` take every non-empty link
   path instead of the owned ones) and see `check.sh` report it, then undo the
   break.
3. Run `check.sh`. Update `expected.txt` only for verdicts the change was meant to
   move, and say so in the PR.
4. Move the sync commit (in the `git diff` above and the line below) forward only
   after you have read the covered code at the new commit and confirmed the model
   describes it.

Last confirmed against: `b34aba24157c3e53870f9aa0296b5878e8e79ece`.
