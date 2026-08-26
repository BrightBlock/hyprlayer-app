import { useState } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useWorkstreamPr, useWorktreeGitStatus } from "@/lib/queries";
import {
  commitWorkstream,
  shipWorkstream,
  useWorkstreamShipBusy,
} from "@/features/session/sessionStore";

const SHIP_POLL_MS = 10_000;

export function useWorkstreamGitActions(view: WorkstreamView | null) {
  const id = view?.item.id ?? "";
  const enabled = view !== null && !isFolderWorkstream(view);
  const shipBusy = useWorkstreamShipBusy(view?.item.id ?? null);
  const [committing, setCommitting] = useState(false);
  const [shipping, setShipping] = useState(false);
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
