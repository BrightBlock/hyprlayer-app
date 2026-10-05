import type { ReactNode, RefObject } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { useViewport } from "@/lib/useViewport";
import { RepoStatus } from "./parts";
import { SessionControlsCluster } from "./SessionControlsCluster";
import { repoSummary } from "./statusBarState";
import { SyncStatus } from "./SyncStatus";

const BAR_PAD_X = 12;

/**
 * Sits under the agent view: the open workstream's repo state and the focused session's
 * controls, then thoughts, sessions and the server in a cluster that lines up with the
 * right panel when `rightPanelWidth` is given.
 */
export function StatusBar({
  workstreamId,
  workstream,
  thoughts,
  sessions,
  tools,
  serverMenu,
  rightPanelWidth,
}: {
  workstreamId: string | null;
  workstream: WorkstreamView | null;
  thoughts?: ReactNode;
  sessions?: ReactNode;
  tools?: ReactNode;
  serverMenu?: (anchorRef: RefObject<HTMLElement | null>) => ReactNode;
  rightPanelWidth?: number;
}) {
  const { compact } = useViewport();
  const repo = workstream ? repoSummary(workstream) : null;
  return (
    <footer
      aria-label="Status bar"
      className="flex shrink-0 items-center gap-3 border-t border-edge bg-panel text-meta tabular-nums"
      style={{ height: STATUS_BAR_HEIGHT, paddingLeft: BAR_PAD_X, paddingRight: BAR_PAD_X }}
    >
      {repo && <RepoStatus repo={repo} compact={compact} />}
      <div className="flex-1" />
      <div className="flex min-w-0 items-center gap-1">
        <SessionControlsCluster workstreamId={workstreamId} compact={compact} />
        {tools}
      </div>
      <div
        className="flex shrink-0 items-center justify-end gap-2 self-stretch border-l border-edge pl-2"
        // A floor, not a width: a long server label would otherwise spill left over the session controls.
        style={rightPanelWidth ? { minWidth: rightPanelWidth - BAR_PAD_X } : undefined}
      >
        {thoughts}
        <SyncStatus compact={compact} divided={!!thoughts} menu={serverMenu} leading={sessions} />
      </div>
    </footer>
  );
}
