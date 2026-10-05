import { useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { Icon } from "@/components/Icon";
import { BAR_POPOVER_ROW, Button, Popover, SegmentedControl, cx } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useActiveSessions, useArchivedWorkstreams, useBootstrap } from "@/lib/queries";
import { useSlowNow } from "@/lib/useSlowNow";
import { openArchivedWorktrees } from "@/features/archive/archivedWorktreesStore";
import { useFleet } from "@/features/dashboard/fleet";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { NoWorktreeBlank } from "./NoWorktreeBlank";
import { mergedToday, PIPELINE_VIEWS, type PipelineLive } from "./pipeline";
import { sessionsByWorkstream } from "./pipelineData";
import { usePipelineViewStore } from "./pipelineViewStore";
import { PipelineBoard } from "./PipelineBoard";
import type { PipelineCardHandlers } from "./PipelineCard";
import { PipelineTable } from "./PipelineTable";
import { PipelineTriage } from "./PipelineTriage";

const byUpdated = (a: WorkstreamView, b: WorkstreamView) => (b.item.updatedAt ?? 0) - (a.item.updatedAt ?? 0);

export function ProjectPipeline({
  projectPath,
  projectName,
  views,
  folder,
  onSelect,
  onNew,
  onCardContextMenu,
}: {
  projectPath: string;
  projectName: string;
  views: WorkstreamView[];
  folder: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
  onCardContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
}) {
  const now = useSlowNow();
  const view = usePipelineViewStore((s) => s.view);
  const setView = usePipelineViewStore((s) => s.setView);
  const sorted = useMemo(() => [...views].sort(byUpdated), [views]);
  const fleet = useFleet(sorted);
  const active = useActiveSessions(views.length > 0);
  const archived = useArchivedWorkstreams();

  const live = useMemo(
    () =>
      new Map<string, PipelineLive>(
        fleet.live.map((l) => [l.view.item.id, { activity: l.activity, sessionId: l.sessionId, line: l.line }]),
      ),
    [fleet.live],
  );
  const sessions = useMemo(() => sessionsByWorkstream(active.data ?? []), [active.data]);
  const merged = useMemo(
    () => mergedToday((archived.data ?? []).filter((v) => v.item.projectPath === projectPath), now),
    [archived.data, projectPath, now],
  );

  const handlers: PipelineCardHandlers = {
    onOpen: (v, sessionId) => {
      onSelect(v.item.id);
      if (sessionId) void selectAgentTab(v.item.id, sessionId);
    },
    onContextMenu: onCardContextMenu,
  };

  if (views.length === 0) {
    return (
      <div className="flex h-full min-h-0 flex-col bg-surface">
        <NoWorktreeBlank projectPath={projectPath} folder={folder} onNew={onNew} onRestore={openArchivedWorktrees} />
      </div>
    );
  }

  const branches = views.filter((v) => !isFolderWorkstream(v)).length;

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface">
      <header className="flex flex-wrap items-center gap-x-3.5 gap-y-2 px-6 pt-[18px] pb-2">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <h1 className="text-h2 text-ink">Projects</h1>
          <ProjectPicker projectPath={projectPath} projectName={projectName} />
        </div>
        <SegmentedControl
          aria-label="Pipeline view"
          variant="bordered"
          value={view}
          options={PIPELINE_VIEWS}
          onChange={setView}
        />
        <Button variant="primary" onClick={onNew}>
          {!folder && <Icon name="branch" size="sm" />}
          {folder ? "New session" : "New worktree"}
        </Button>
      </header>
      {view === "board" ? (
        <PipelineBoard
          views={sorted}
          live={live}
          sessions={sessions}
          merged={merged}
          now={now}
          handlers={handlers}
          onOpenArchived={openArchivedWorktrees}
        />
      ) : view === "triage" ? (
        <PipelineTriage views={sorted} live={live} sessions={sessions} now={now} handlers={handlers} />
      ) : (
        <PipelineTable views={sorted} live={live} sessions={sessions} now={now} handlers={handlers} />
      )}
      {!folder && (
        <p className="flex items-center gap-1.5 px-6 pb-3 font-mono text-meta text-ink-dim">
          <Icon name="branch" size="xs" />
          {`${branches} ${branches === 1 ? "branch" : "branches"} active`}
        </p>
      )}
    </div>
  );
}

function ProjectPicker({ projectPath, projectName }: { projectPath: string; projectName: string }) {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const projects = bootstrap.data?.projects ?? [];

  return (
    <>
      <button
        ref={anchor}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex h-[26px] min-w-0 items-center gap-1.5 rounded-control border border-edge bg-panel px-2 text-ui font-semibold text-ink-2 hover:bg-edge/50"
      >
        <span className="text-ink-dim">
          <Icon name="folder" size="xs" />
        </span>
        <span className="truncate">{projectName}</span>
        <span className="text-ink-dim">
          <Icon name="chevronDown" size="xs" />
        </span>
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchor}
        matchAnchorWidth
        className="max-h-80 overflow-y-auto p-1.5 shadow-pop"
        aria-label="Switch project"
      >
        <div role="menu">
          {projects.map((p) => (
            <button
              key={p.path}
              type="button"
              role="menuitemradio"
              aria-checked={p.path === projectPath}
              onClick={() => {
                setOpen(false);
                if (p.path !== projectPath) {
                  void navigate({ to: "/projects/$projectPath", params: { projectPath: p.path }, search: {} });
                }
              }}
              className={cx(BAR_POPOVER_ROW, "w-full", p.path === projectPath && "font-semibold text-ink")}
            >
              <span className="min-w-0 flex-1 truncate text-left">{p.mappedName}</span>
              {p.path === projectPath && <Icon name="check" size="xs" />}
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}
