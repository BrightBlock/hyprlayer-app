import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { Button, PageError } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { useAllWorkstreams, useArchivedWorkstreams, useBootstrap, useDocs } from "@/lib/queries";
import type { Project, WorkstreamView } from "@/lib/bindings";
import { openArchivedWorktrees } from "@/features/archive/archivedWorktreesStore";
import { AddProjectDialog } from "@/features/projects/AddProjectDialog";
import { DiscoverDialog } from "@/features/dashboard/DiscoverDialog";
import { buildGraph } from "@/features/dashboard/graph";
import { useFleet } from "@/features/dashboard/fleet";
import { KnowledgeGraph } from "@/features/dashboard/KnowledgeGraph";
import {
  ActivityPanel,
  FirstNodeField,
  GraphPins,
  HeroSearch,
  LiveNowPanel,
  SetupPill,
  StarterStats,
  StatPills,
  heroStatus,
  liveTone,
  stageDot,
  type GraphPin,
  type RecentRow,
} from "@/features/dashboard/Observatory";
import { stageLabel, stageOf } from "@/lib/domain/stages";
import { selectAgentTab } from "@/features/search/selectAgentTab";

const GRAPH_DOC_LIMIT = 200;

const RECENT_LIMIT = 5;

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

const EMPTY_GRAPH = buildGraph([]);

const scrimStop = (pct: number, at: string) =>
  `color-mix(in srgb, var(--color-surface) ${pct}%, transparent) ${at}`;
const PANEL_SCRIM =
  "linear-gradient(180deg, transparent 0%, " +
  [
    scrimStop(6, "15%"),
    scrimStop(22, "30%"),
    scrimStop(43, "45%"),
    scrimStop(65, "60%"),
    scrimStop(84, "75%"),
    scrimStop(97, "90%"),
  ].join(", ") +
  ", var(--color-surface) 100%)";

function useSlowNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

function greetingFor(now: number, name: string | null): string {
  const hour = new Date(now).getHours();
  const day = hour >= 5 && hour < 12 ? "morning" : hour >= 12 && hour < 18 ? "afternoon" : "evening";
  return name ? `Good ${day}, ${name}` : `Good ${day}`;
}

export function Dashboard() {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const now = useSlowNow();

  const projects = useMemo(() => bootstrap.data?.projects ?? [], [bootstrap.data]);
  const workstreams = useAllWorkstreams(projects.length > 0);
  const archived = useArchivedWorkstreams();
  const fleet = useFleet(workstreams.data ?? []);

  const docsQuery = useDocs({ sort: "mtime", limit: GRAPH_DOC_LIMIT });
  const docs = useMemo(() => docsQuery.data?.items ?? [], [docsQuery.data]);
  const graph = useMemo(() => buildGraph(docs), [docs]);

  const [addProject, setAddProject] = useState<{ url: string | null } | null>(null);
  const [discoverOpen, setDiscoverOpen] = useState(false);

  const projectNames = useMemo(
    () => new Map(projects.map((p) => [p.path, p.mappedName])),
    [projects],
  );
  const projectName = (path: string): string =>
    projectNames.get(path) ?? path.split(/[\\/]/).pop() ?? path;

  const recentRows = useMemo<RecentRow[]>(() => {
    const active = fleet.idle.map<RecentRow>((view) => ({
      kind: "active",
      at: view.item.updatedAt ?? view.item.createdAt ?? 0,
      view,
      project: projectName(view.item.projectPath),
    }));
    const gone = (archived.data ?? []).map<RecentRow>((view) => ({
      kind: "archived",
      at: view.item.archivedAt ?? view.item.updatedAt ?? 0,
      view,
      project: projectName(view.item.projectPath),
      merged: view.info.pr.state === "merged",
    }));
    return [...active, ...gone].sort((a, b) => b.at - a.at).slice(0, RECENT_LIMIT);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fleet.idle, archived.data, projectNames]);

  const lastShipped = useMemo(() => {
    const view = (archived.data ?? []).find((v) => v.item.prNumber !== null);
    return view
      ? {
          title: view.item.title,
          pr: view.item.prNumber,
          at: view.item.archivedAt ?? view.item.updatedAt ?? 0,
        }
      : null;
  }, [archived.data]);

  const weekCount = useMemo(() => {
    const cutoff = now - WEEK_MS;
    const touched = (workstreams.data ?? []).filter(
      (v) => (v.item.updatedAt ?? v.item.createdAt ?? 0) >= cutoff,
    ).length;
    const retired = (archived.data ?? []).filter((v) => (v.item.archivedAt ?? 0) >= cutoff).length;
    return touched + retired;
  }, [workstreams.data, archived.data, now]);

  const openProject = (project: Project) =>
    void navigate({
      to: "/projects/$projectPath",
      params: { projectPath: project.path },
      search: {},
    });

  const openWorkstream = (view: WorkstreamView, sessionId?: string | null) => {
    void navigate({
      to: "/projects/$projectPath",
      params: { projectPath: view.item.projectPath },
      search: { item: view.item.id },
    });
    if (sessionId) selectAgentTab(view.item.id, sessionId);
  };

  const livePins = fleet.live.slice(0, 3).map<GraphPin>((item) => {
    const running = item.activity === "working" || item.activity === "background";
    const pr = item.view.item.prNumber;
    const tone = liveTone(item);
    return {
      key: item.view.item.id,
      label: `${item.view.item.title}${pr !== null ? ` → PR #${pr}` : ""} · ${
        running ? stageLabel(stageOf(item.view)).toLowerCase() : item.activity.replace("-", " ")
      }`,
      dot: tone.dot,
      pulse: tone.pulse,
      border: tone.border,
      onClick: () => openWorkstream(item.view, item.sessionId),
    };
  });
  const quietPins = [...fleet.idle]
    .sort((a, b) => (b.item.updatedAt ?? 0) - (a.item.updatedAt ?? 0))
    .slice(0, Math.max(0, 3 - livePins.length))
    .map<GraphPin>((view) => ({
      key: view.item.id,
      label: view.item.title,
      dot: stageDot(view),
      pulse: false,
      onClick: () => openWorkstream(view),
    }));
  const pins = [...livePins, ...quietPins];

  const startTarget =
    [...(workstreams.data ?? [])].sort(
      (a, b) => (b.item.updatedAt ?? 0) - (a.item.updatedAt ?? 0),
    )[0]?.item.projectPath ??
    projects[0]?.path ??
    null;
  const startAgent = startTarget
    ? () =>
        void navigate({
          to: "/projects/$projectPath",
          params: { projectPath: startTarget },
          search: { create: true },
        })
    : null;

  if (bootstrap.error) return <PageError error={bootstrap.error} />;

  const config = bootstrap.data?.config;
  const userName =
    config?.state === "ready" && config.user
      ? config.user.charAt(0).toUpperCase() + config.user.slice(1)
      : null;

  const thoughtsConfigured = config?.state === "ready" && config.configured;
  const firstRun = bootstrap.data !== undefined && !thoughtsConfigured;
  const hasProjects = projects.length > 0;
  const openWizard = () => void navigate({ to: "/onboarding" });

  const date = new Date(now);
  const dateLabel = `${date.toLocaleDateString(undefined, { weekday: "long" })} · ${date.toLocaleDateString(undefined, { month: "long", day: "numeric" })}`;

  const runningStages = [
    ...new Set(
      fleet.live
        .filter((l) => l.activity === "working" || l.activity === "background")
        .map((l) => stageLabel(stageOf(l.view)).toLowerCase()),
    ),
  ];
  const status = heroStatus(fleet.counts, {
    hasProjects: firstRun || hasProjects,
    loading: hasProjects && workstreams.isLoading,
    runningStages,
  });

  const emptyHint =
    projects.length === 0
      ? "Add a repo to start dispatching agents."
      : workstreams.isLoading
        ? "Loading workstreams…"
        : "Nothing running.";

  return (
    <div className="relative h-full min-h-[560px] overflow-hidden bg-surface">
      {firstRun ? (
        <>
          <div className="absolute inset-0">
            <KnowledgeGraph graph={EMPTY_GRAPH} />
          </div>
          <FirstNodeField />
          <div className="pointer-events-none absolute inset-x-0 top-1/2 flex -translate-y-1/2 flex-col items-center gap-3.5 px-7 lg:px-11">
            <HeroSearch total={docsQuery.data?.total ?? 0} />
            <StarterStats agentsLive={fleet.counts.running} />
          </div>
        </>
      ) : (
        <>
          <div className="absolute inset-0">
            <KnowledgeGraph graph={graph} />
          </div>

          {hasProjects && (
            <div
              className="pointer-events-none absolute inset-x-0 bottom-0 h-[300px]"
              style={{ background: PANEL_SCRIM }}
            />
          )}

          <GraphPins pins={pins} />
        </>
      )}

      <div className="pointer-events-none absolute inset-0 flex flex-col px-7 pb-6 pt-7 lg:px-11">
        <header className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-label text-ink-dim">
              {dateLabel}
            </div>
            <h1 className="mt-1.5 truncate text-h1 text-ink">
              {greetingFor(now, userName)}
            </h1>
            <div className="mt-2.5 flex items-center gap-2 text-body text-ink-dim">
              <span
                className={`h-[7px] w-[7px] shrink-0 rounded-full ${status.dot} ${
                  status.pulse ? "animate-pulse" : ""
                }`}
              />
              {status.text}
            </div>
          </div>
          <div className="pointer-events-auto flex shrink-0 items-center gap-2.5">
            <Button onClick={() => setDiscoverOpen(true)} className="text-ui">
              Discover
            </Button>
            <Button
              variant="primary"
              onClick={() => setAddProject({ url: null })}
              className="gap-1.5 text-ui"
            >
              <Icon name="add" size="xs" />
              Add repo
            </Button>
          </div>
        </header>

        <div className="min-h-4 flex-[1]" />

        {!firstRun && (
          <div className="flex flex-col items-center gap-3.5">
            <HeroSearch total={docsQuery.data?.total ?? 0} />
            <StatPills
              documents={docsQuery.data?.total ?? 0}
              links={graph.edges.length}
              projects={projects.length}
              agentsLive={fleet.counts.running}
              weekCount={weekCount}
              hint={
                docs.length > 0
                  ? `The graph shows the ${graph.nodes.filter((n) => n.degree > 0).length} linked of the ${docs.length} most-recent documents`
                  : undefined
              }
            />
            {!docsQuery.isLoading && docs.length === 0 && (
              <p className="max-w-[420px] text-center text-ui text-ink-dim">
                No documents yet. Agents file research, plans, and notes here as they work,
                and they light up as a connected graph.
              </p>
            )}
          </div>
        )}

        <div className="min-h-6 flex-[1.5]" />

        {!firstRun && hasProjects && (
          <div
            className={`pointer-events-auto grid grid-cols-1 gap-4 ${
              fleet.live.length > 0 ? "lg:grid-cols-[3fr_2fr]" : "lg:grid-cols-[2fr_3fr]"
            }`}
          >
            <LiveNowPanel
              items={fleet.live}
              now={now}
              projectName={projectName}
              onOpen={(item) => openWorkstream(item.view, item.sessionId)}
              emptyHint={emptyHint}
              lastShipped={lastShipped}
              onStartAgent={startAgent}
              onOpenLibrary={() => void navigate({ to: "/library", search: {} })}
            />
            <ActivityPanel
              rows={recentRows}
              now={now}
              onOpenActive={(view) => openWorkstream(view)}
              onOpenArchive={openArchivedWorktrees}
            />
          </div>
        )}
      </div>

      {firstRun && <SetupPill onClick={openWizard} />}

      <AddProjectDialog
        open={addProject !== null}
        initialUrl={addProject?.url ?? null}
        onClose={() => setAddProject(null)}
        onAdded={openProject}
      />
      <DiscoverDialog
        open={discoverOpen}
        onClose={() => setDiscoverOpen(false)}
        onPick={(repo) => setAddProject({ url: repo.url })}
      />
    </div>
  );
}
