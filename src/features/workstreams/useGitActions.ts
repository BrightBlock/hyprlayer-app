import { useState } from "react";

import type { WorkstreamView } from "../../lib/bindings";
import { useWorkstreamPr, useWorktreeGitStatus } from "../../lib/queries";
import {
  commitWorkstream,
  shipWorkstream,
  useWorkstreamShipBusy,
} from "../session/sessionStore";

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
 */
export function useWorkstreamGitActions(view: WorkstreamView | null) {
  const id = view?.item.id ?? "";
  const enabled = view !== null;
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
    view && !hasLivePr && (changedCount > 0 || view.git.ahead > 0),
  );

  const doCommit = () => {
    if (!view) return;
    setCommitting(true);
    void commitWorkstream(view.item.id).finally(() => setCommitting(false));
  };
  const doShip = () => {
    if (!view) return;
    setShipping(true);
    // Pass the workstream's target so the opened PR is filed against it.
    void shipWorkstream(view.item.id, view.item.baseBranch).finally(() =>
      setShipping(false),
    );
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
