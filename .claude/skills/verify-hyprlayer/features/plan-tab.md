# Plan tab

The right sidebar's Plan tab previews the plan a session is building. It shows per-phase tasks and checks, a progress header that counts tasks, automated checks and manual checks separately, and an `Open manual checks` card listing every unticked manual check with its phase. Clicking a card row ticks it in the plan file. At the Ship stage, open manual checks show as a warning, but Ship stays enabled.

## Sub-features

- `plan-open` opens the preview for a plan.
- `plan-counts` shows the header count line (`N of M tasks · N of M automated · N of M manual`).
- `plan-rollup` shows the open manual checks card: phase tags, collapse, and a 220px scroll cap.
- `plan-rollup-empty` shows no card when no manual check is open.
- `plan-tick` checks a rolled-up row off, writing `[x]` to the file.
- `plan-live` updates the preview when the plan file changes on disk.
- `plan-ship-warning` shows `N manual checks open` in the Ship badge and a not-done row above `Ship`.

## How to get to it (user POV)

- Click a plan card in the transcript (`aria-label` `Preview plan: <title>`).
- The plan opens on its own when a live session writes its first plan.
- Choose the `Plan` tab in the right sidebar; it's offered once the session has a plan.

## Driving it with the MCP bridge

Preconditions:

- A **shared** instance (the `.dev` profile) on the hyprlayer-app project. It needs a workstream with an implement or validate session; the lifecycle (Validate/Review/Ship) only renders for those.
- A plan under `thoughts/shared/plans` in tracking status (`active`, `approved` or `implemented`) with at least one open manual check. Parse the plans to find one rather than grepping. `2026-08-21-scope-sidebar-docs-to-workstream.md` (1 of 9 open) and `2026-08-17-linux-tiling-wm-window-sizing.md` (22 of 41) have worked.
- A tracking plan with none open, for the empty case.
- A copy of every plan file you'll edit, saved in the evidence dir before the first edit.

- **Open.** Click the session's plan card: `webview_interact` `click` on `[aria-label^="Preview plan: "]`. The card may sit in another tab of the workstream (a Validate tab) and far up the transcript. A click at a negative `y` does nothing, so `scrollIntoView({ block: 'center' })` first. The store's `bySession[<sessionId>]` then holds the plan key with `followLatest: true`. To show a different plan in the same session, import the live `planPreviewStore` (SKILL.md, Drive step 5) and call `openPlanPreview(<sessionId>, { key: '<absolute canonical thoughts path>', cardId: null, followLatest: false })`. Session ids come from the live `sessionStore`'s `useSessionStore.getState().sessions`. That only chooses which plan to show; it doesn't verify `plan-open`. The title and progress bar render.
- **Counts.** The header's count line reads, for example, `16 of 16 automated · 8 of 9 manual`. A plan with no manual checks reads `2 of 2 tasks · 3 of 3 checks` instead.
- **Roll-up.** `webview_find_element` on `section[aria-label="Open manual checks"]` returns `Open manual checks · N`, followed by each row's title and `Phase <label>`. With 20+ open, the lifecycle rows and the current phase are still on screen.
- **Collapse.** Click `section[aria-label="Open manual checks"] > button`. The section's height drops to its header (~34px). Click again to restore it.
- **Tick.** Click `button[aria-label^="Mark done: "]`. Within ~3s, the file's line reads `- [x]`, and the card, the badge detail and the warning row clear. Restore the file from the copy right away.
- **Live update.** Tick the same box in the file with `sed`. The roll-up and counts update without a reload. Restore the file.
- **Ship warning.** On a plan at Ship, the badge reads `Ship  N manual checks open`, and a `N manual checks open` row sits above `Review approved`/`Ship`. `Ship` stays enabled.
- **Empty.** Open a tracking plan with no open manual checks. `section[aria-label="Open manual checks"]` doesn't exist.
- **Proof.** Take screenshots before and after each change, keep the file copies, and record the `sed -n <line>p` output showing `[x]` and then `[ ]`.

## Gotchas

- An HMR edit or a checkout switch resets `planPreviewStore`, so the aside falls back to the session's own plan. Re-select it through the live module URL.
- Plan files are canonical paths under `~/thoughts/repos/<repo>/…`. Edit them through the repo's `thoughts/` symlink and never `cd` into `~/thoughts`. Auto-sync commits and pushes each edit within seconds, so revert immediately. Tick plus revert leaves the same bytes but two sync commits. The tick's write reset `2026-08-21-scope-sidebar-docs-to-workstream.md` from `100755` to `100644`, and `cp` over it keeps that, so `cmp` passes but a mode change gets pushed. Check `git -C thoughts/shared diff --summary` and `chmod` it back.
- Rows aren't buttons for plans the app can't write to (Claude's own and inline plans).
- Manual checks under a phase's `Success Criteria` count. The "Manual Testing Steps" lists under Testing Strategy don't parse as checks.
