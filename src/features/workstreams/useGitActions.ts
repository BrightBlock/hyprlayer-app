import { useState } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useWorkstreamPr, useWorktreeGitStatus } from "@/lib/queries";
import {
  commitWorkstream,
  shipWorkstream,
  useWorkstreamShipBusy,
} from "@/features/session/sessionStore";

/** How often to re-ask `gh` for the PR while a ship turn is running. */
const SHIP_POLL_MS = 10_000;

/**
 * Commit and Ship gating for a workstream, shared by the Git panel's buttons and
 * Root's `ws.commit` / `ws.ship` keybindings so the two never disagree.
 *
 * Gates on the live `useWorktreeGitStatus` count, the same query the Changes
 * panel renders, rather than the sidebar's `git.dirty` snapshot.
 *
 * Ship hides once the branch has an open or merged PR, since the PR bar takes
 * over. Commit hides while Ship is mid-turn or just started, because Ship
 * commits the same changes and a manual Commit would race it.
 *
 * Ship gates on `aheadOfBase` (commits vs the merge target), NOT `ahead`
 * (commits vs upstream). `ahead` drops to 0 the moment the branch is pushed,
 * but a pushed branch with no PR still has something to ship.
 */
export function useWorkstreamGitActions(view: WorkstreamView | null) {
  const id = view?.item.id ?? "";
  // A folder workstream never gets git/PR queries, not even failing ones.
  const enabled = view !== null && !isFolderWorkstream(view);
  const shipBusy = useWorkstreamShipBusy(view?.item.id ?? null);
  const [committing, setCommitting] = useState(false);
  const [shipping, setShipping] = useState(false);
  // Poll only while a ship turn runs, and only until a PR resolves. The PR
  // appears MID-turn (the stock Ship prompt opens it, then runs /describe_pr),
  // and the watcher is in its slowest cadence precisely because this branch has
  // no PR yet, so nothing else notices until the turn finally ends.
  const pr = useWorkstreamPr(id, enabled, shipBusy || shipping ? SHIP_POLL_MS : undefined);
  const status = useWorktreeGitStatus(id, enabled);

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
