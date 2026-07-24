import { useState } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useWorkstreamPr, useWorktreeGitStatus } from "@/lib/queries";
import {
  commitWorkstream,
  shipWorkstream,
  useWorkstreamShipBusy,
} from "@/features/session/sessionStore";

/**
 * The Commit / Ship gating + actions for a workstream, shared by the GIT panel's
 * buttons and Root's `ws.commit` / `ws.ship` keybindings so the two never
 * disagree. Gates on the live `useWorktreeGitStatus` count (the SAME query the
 * Changes panel renders) — not the sidebar list's `git.dirty` snapshot — so the
 * buttons stay in lockstep with the panel.
 *
 * Ship opens the PR, so it's pointless once one is live: hidden while the branch
 * already has an open or merged PR (the PR bar takes over). Commit is hidden
 * while Ship is mid-turn (`shipBusy`) or just kicked off (`shipping`) — Ship
 * commits these same changes itself, so a manual Commit would race it.
 *
 * Ship gates on `aheadOfBase` (commits vs the merge target), NOT `ahead` (commits
 * vs the upstream): `ahead` drops to 0 the moment the branch is pushed, but a
 * pushed-yet-PR-less branch still has something to ship — so pushing must not
 * make the button vanish.
 */
export function useWorkstreamGitActions(view: WorkstreamView | null) {
  const id = view?.item.id ?? "";
  // A folder workstream never gets git/PR queries — not even failing ones.
  const enabled = view !== null && !isFolderWorkstream(view);
  const pr = useWorkstreamPr(id, enabled);
  const status = useWorktreeGitStatus(id, enabled);
  const shipBusy = useWorkstreamShipBusy(view?.item.id ?? null);
  const [committing, setCommitting] = useState(false);
  const [shipping, setShipping] = useState(false);

  const changedCount =
    (status.data?.staged.length ?? 0) + (status.data?.unstaged.length ?? 0);
  const hasLivePr = pr.data?.state === "open" || pr.data?.state === "merged";

  const canCommit = enabled && changedCount > 0 && !shipBusy && !shipping;
  const canShip = Boolean(
    enabled && view && !hasLivePr && (changedCount > 0 || view.git.aheadOfBase > 0),
  );

  const doCommit = () => {
    if (!view) return;
    setCommitting(true);
    void commitWorkstream(view.item.id).finally(() => setCommitting(false));
  };
  const doShip = () => {
    if (!view) return;
    setShipping(true);
    // The backend names the workstream's target in the directive it composes.
    void shipWorkstream(view.item.id).finally(() => setShipping(false));
  };

  return {
    changedCount,
    hasLivePr,
    canCommit,
    canShip,
    committing,
    shipping,
    doCommit,
    doShip,
  };
}
