import { useState } from "react";

import { closeReview, currentAppLocation } from "@/lib/appNav";
import type { Session, WorkstreamView } from "@/lib/bindings";
import { harnessLabel } from "@/lib/domain/harnessing";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { knownPr, prFailureText } from "@/lib/domain/pr";
import { useWorkstreamPr, useWorkstreamSessions, useWorktreeGitStatus } from "@/lib/queries";
import {
  commitWorkstream,
  shipWorkstream,
  useSessionStore,
  useWorkstreamShipBusy,
} from "@/features/session/sessionStore";
import { isWorking } from "@/features/session/transcript";

import { openShipCheck, setShipping, useShipCheck } from "./shipCheck";

const SHIP_POLL_MS = 10_000;

export function newestImplementSession(sessions: Session[]): Session | null {
  let newest: Session | null = null;
  for (const s of sessions) {
    if (s.stage !== "implement" || s.archivedAt != null) continue;
    if (!newest || (s.startedAt ?? 0) > (newest.startedAt ?? 0)) newest = s;
  }
  return newest;
}

// A row's `working` only refreshes on lifecycle events, so a loaded entry wins over it.
export function useImplementWorking(workstreamId: string, enabled = true): boolean {
  const sessions = useWorkstreamSessions(workstreamId, enabled);
  const newest = newestImplementSession(sessions.data ?? []);
  const storeWorking = useSessionStore((s) =>
    Object.values(s.sessions).some(
      (e) => e.workstreamId === workstreamId && e.stage === "implement" && isWorking(e),
    ),
  );
  const newestLoaded = useSessionStore((s) => (newest ? newest.id in s.sessions : false));
  return enabled && (storeWorking || (!newestLoaded && (newest?.working ?? false)));
}

export function useWorkstreamGitActions(view: WorkstreamView | null) {
  const id = view?.item.id ?? "";
  const enabled = view !== null && !isFolderWorkstream(view);
  const shipBusy = useWorkstreamShipBusy(view?.item.id ?? null);
  const [committing, setCommitting] = useState(false);
  const shipping = useShipCheck((s) => s.shipping);
  const pr = useWorkstreamPr(id, enabled, shipBusy || shipping ? SHIP_POLL_MS : undefined);
  const status = useWorktreeGitStatus(id, enabled);
  const implementWorking = useImplementWorking(id, enabled);
  const sessions = useWorkstreamSessions(id, enabled);
  const implementLabel = harnessLabel(newestImplementSession(sessions.data ?? [])?.harness);

  const changedCount =
    (status.data?.staged.length ?? 0) + (status.data?.unstaged.length ?? 0);
  const lookup = pr.data;
  const known = knownPr(lookup);
  const hasLivePr = known?.state === "open" || known?.state === "merged";
  const prBlocked =
    lookup?.kind === "failed"
      ? `PR status unknown: ${prFailureText(lookup.cause)}`
      : lookup === undefined || lookup.kind === "unknown"
        ? "Checking GitHub for this branch's PR"
        : null;

  const canCommit = enabled && changedCount > 0 && !shipBusy && !shipping;
  const hasWork = Boolean(
    enabled && view && !hasLivePr && (changedCount > 0 || view.git.base.ahead > 0),
  );
  const shippable = hasWork && prBlocked === null;
  const canShip = shippable && !implementWorking;
  const shipBlocked = hasWork
    ? (prBlocked ?? (implementWorking ? `Ship unlocks when ${implementLabel} finishes` : null))
    : null;

  const doCommit = () => {
    if (!view) return;
    setCommitting(true);
    void commitWorkstream(view.item.id).finally(() => setCommitting(false));
  };
  const doShip = () => {
    if (!view) return;
    openShipCheck({ workstreamId: view.item.id, projectPath: view.item.projectPath });
  };
  const shipNow = () => {
    if (!view) return;
    if (currentAppLocation()?.search.review) closeReview();
    setShipping(true);
    void shipWorkstream(view.item.id).finally(() => setShipping(false));
  };

  return {
    changedCount,
    prLookup: lookup,
    hasLivePr,
    canCommit,
    canShip,
    committing,
    shipping,
    implementWorking,
    shipBlocked,
    doCommit,
    doShip,
    shipNow,
  };
}
