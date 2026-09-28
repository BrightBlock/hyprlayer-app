import type { ReactNode, RefObject } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { useViewport } from "@/lib/useViewport";
import { Avatar, Divider, RepoStatus } from "./parts";
import { SessionControlsCluster } from "./SessionControlsCluster";
import { repoSummary } from "./statusBarState";
import { SyncStatus } from "./SyncStatus";

/** App-wide bar: the open workstream's repo state, the connection, and the focused session's controls. */
export function StatusBar({
  workstreamId,
  workstream,
  archive,
  sessions,
  tools,
  settings,
  serverMenu,
}: {
  workstreamId: string | null;
  workstream: WorkstreamView | null;
  archive?: ReactNode;
  sessions?: ReactNode;
  tools?: ReactNode;
  settings: ReactNode;
  serverMenu?: (anchorRef: RefObject<HTMLElement | null>) => ReactNode;
}) {
  const { compact } = useViewport();
  const repo = workstream ? repoSummary(workstream) : null;
  return (
    <footer
      aria-label="Status bar"
      className="flex shrink-0 items-center gap-3 border-t border-edge bg-panel px-3 text-meta tabular-nums"
      style={{ height: STATUS_BAR_HEIGHT }}
    >
      {repo && <RepoStatus repo={repo} compact={compact} />}
      {archive && (
        <>
          {repo && <Divider />}
          {archive}
        </>
      )}
      <SyncStatus
        compact={compact}
        divided={repo !== null || Boolean(archive)}
        menu={serverMenu}
        leading={sessions}
      />
      <div className="flex-1" />
      <SessionControlsCluster workstreamId={workstreamId} compact={compact} />
      {tools}
      {settings}
      <Avatar />
    </footer>
  );
}
