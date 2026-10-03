import { useMemo } from "react";
import { useNavigate } from "@tanstack/react-router";

import { PageError } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { openArchivedWorktrees } from "@/features/archive/archivedWorktreesStore";
import { mergedToday, type PipelineWaiting } from "@/features/board/pipeline";
import { PipelineBoard } from "@/features/board/PipelineBoard";
import { useFleet } from "@/features/dashboard/fleet";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { useAllWorkstreams, useArchivedWorkstreams, useBootstrap } from "@/lib/queries";
import { useSlowNow } from "@/lib/useSlowNow";

export function PipelinePage() {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const now = useSlowNow();

  const projects = useMemo(() => bootstrap.data?.projects ?? [], [bootstrap.data]);
  const workstreams = useAllWorkstreams(projects.length > 0);
  const archived = useArchivedWorkstreams();
  const fleet = useFleet(workstreams.data ?? []);

  const projectNames = useMemo(() => new Map(projects.map((p) => [p.path, p.mappedName])), [projects]);
  const projectName = (path: string): string => projectNames.get(path) ?? path.split(/[\\/]/).pop() ?? path;

  const waiting = useMemo(
    () =>
      new Map<string, PipelineWaiting>(
        fleet.live
          .filter((l) => l.activity === "needs-you")
          .map((l) => [l.view.item.id, { sessionId: l.sessionId, line: l.line }]),
      ),
    [fleet.live],
  );
  const merged = useMemo(() => mergedToday(archived.data ?? [], now), [archived.data, now]);

  const openWorkstream = (view: WorkstreamView, sessionId: string | null) => {
    void navigate({
      to: "/projects/$projectPath",
      params: { projectPath: view.item.projectPath },
      search: { item: view.item.id },
    });
    if (sessionId) void selectAgentTab(view.item.id, sessionId);
  };

  if (bootstrap.error) return <PageError error={bootstrap.error} />;

  return (
    <PipelineBoard
      views={workstreams.data ?? []}
      waiting={waiting}
      merged={merged}
      projectName={projectName}
      now={now}
      onOpen={openWorkstream}
      onOpenArchived={openArchivedWorktrees}
    />
  );
}
