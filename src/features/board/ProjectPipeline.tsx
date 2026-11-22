import { useMemo, useRef, useState, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";

import { Icon } from "@/components/Icon";
import { BAR_POPOVER_ROW, Button, Popover, SegmentedControl, cx } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useActiveSessions, useArchivedWorkstreams, useBootstrap } from "@/lib/queries";
import { useSlowNow } from "@/lib/useSlowNow";
import { openArchivedWorktrees } from "@/features/archive/archivedWorktreesStore";
import { useFleet } from "@/features/dashboard/fleet";
import { setLeadDockExpanded, useLeadAsk, useLeadDockExpanded, useProjectLead } from "@/features/lead/lead";
import { LeadChip } from "@/features/lead/LeadChip";
import { LeadDock } from "@/features/lead/LeadDock";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { NoWorktreeBlank } from "./NoWorktreeBlank";
import { mergedToday, PIPELINE_VIEWS, type PipelineLive } from "./pipeline";
import { sessionsByWorkstream } from "./pipelineData";
import { usePipelineViewStore } from "./pipelineViewStore";
import { PipelineBoard } from "./PipelineBoard";
import { LeadCard, type PipelineCardHandlers } from "./PipelineCard";
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
  leadChat,
}: {
  projectPath: string;
  projectName: string;
  views: WorkstreamView[];
  folder: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
  onCardContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
  leadChat?: ReactNode;
}) {
  const now = useSlowNow();
  const view = usePipelineViewStore((s) => s.view);
  const setView = usePipelineViewStore((s) => s.setView);
  const sorted = useMemo(() => [...views].sort(byUpdated), [views]);
  const fleet = useFleet(sorted);
  const active = useActiveSessions(views.length > 0);
  const archived = useArchivedWorkstreams();
  const lead = useProjectLead(projectPath);
  const mainId = lead?.view.sessionId ? lead.view.workstreamId : null;
  const shown = useMemo(() => (mainId ? sorted.filter((v) => v.item.id !== mainId) : sorted), [sorted, mainId]);
  const driven = useMemo(() => new Set(lead?.view.driving), [lead?.view.driving]);
  const ask = useLeadAsk(lead, shown);
  const expanded = useLeadDockExpanded(projectPath) && !!lead?.view.sessionId;

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

  const openSession = (workstreamId: string, sessionId: string | null) => {
    onSelect(workstreamId);
    if (sessionId) void selectAgentTab(workstreamId, sessionId);
  };
  const handlers: PipelineCardHandlers = {
    onOpen: (v, sessionId) => openSession(v.item.id, sessionId),
    onContextMenu: onCardContextMenu,
  };
  const expandLead = () => {
    if (!lead?.view.sessionId || !lead.view.workstreamId) return;
    void selectAgentTab(lead.view.workstreamId, lead.view.sessionId);
    setLeadDockExpanded(projectPath, true);
  };
  const dock = lead && (
    <LeadDock
      lead={lead}
      ask={ask}
      expanded={expanded}
      chat={leadChat}
      onExpand={expandLead}
      onMinimize={() => setLeadDockExpanded(projectPath, false)}
    />
  );


  if (views.length === 0) {
    return (
      <div className="relative flex h-full min-h-0 flex-col bg-surface">
        <div inert={expanded} className="min-h-0 flex-1">
          <NoWorktreeBlank projectPath={projectPath} folder={folder} onNew={onNew} onRestore={openArchivedWorktrees} />
        </div>
        {dock}
      </div>
    );
  }

  const mainView = mainId ? sorted.find((v) => v.item.id === mainId) : undefined;
  const mainLive = mainId ? live.get(mainId) : undefined;
  const leadWaiting = mainLive?.activity === "needs-you" && mainLive.sessionId === lead?.view.sessionId;
  const leadCard =
    mainView && mainLive && leadWaiting && !ask?.subject ? (
      <LeadCard view={mainView} live={mainLive} now={now} handlers={handlers} />
    ) : null;

  const branches = views.filter((v) => !isFolderWorkstream(v)).length;

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-surface">
      <div inert={expanded} className="contents">
        <header className="flex flex-wrap items-center gap-x-3.5 gap-y-2 px-6 pt-[18px] pb-2">
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            <h1 className="min-w-0 text-h2 text-ink">
              <ProjectPicker projectPath={projectPath} projectName={projectName} />
            </h1>
            {lead && (
              <LeadChip
                driving={lead.view.driving.length}
                of={views.filter((v) => v.item.id !== lead.view.workstreamId).length}
              />
            )}
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
            views={shown}
            live={live}
            sessions={sessions}
            merged={merged}
            driven={driven}
            leadCard={leadCard}
            ask={ask}
            now={now}
            handlers={handlers}
            onOpenArchived={openArchivedWorktrees}
          />
        ) : view === "triage" ? (
          <PipelineTriage views={shown} live={live} sessions={sessions} now={now} handlers={handlers} />
        ) : (
          <PipelineTable views={shown} live={live} sessions={sessions} now={now} handlers={handlers} />
        )}
        {!folder && (
          <p className="flex items-center gap-1.5 px-6 pb-3 font-mono text-meta text-ink-dim">
            <Icon name="branch" size="xs" />
            {`${branches} ${branches === 1 ? "branch" : "branches"} active`}
          </p>
        )}
      </div>
      {dock}
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
        className="-mx-1.5 inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-control px-1.5 hover:bg-edge/50 focus-visible:bg-edge/50"
      >
        <span className="truncate">{projectName}</span>
        <span className="text-ink-dim">
          <Icon name="chevronDown" size="sm" />
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
