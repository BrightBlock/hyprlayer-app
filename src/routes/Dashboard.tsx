import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { Button, PageError } from "@/components/ui";
import { PlusIcon } from "@/components/icons";
import { useAllWorkstreams, useArchivedWorkstreams, useBootstrap, useDocs } from "@/lib/queries";
import type { Project, WorkstreamView } from "@/lib/bindings";
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

/** How many most-recent docs feed the graph. Caps the O(n²) layout cost; the
 *  corpus may be larger (the stat pills reflect the full index). */
const GRAPH_DOC_LIMIT = 200;

/** How many rows the Latest-activity panel shows. */
const RECENT_LIMIT = 5;

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** Stable empty graph for the first-run hero — the KnowledgeGraph then
 *  paints only its nebula backdrop (no stars), identity-stable so its
 *  layout effect never re-runs. */
const EMPTY_GRAPH = buildGraph([]);

/** Bottom band the docked panels sit on — smoothstep easing, painted in the
 *  plain surface tone so the hero's foot reads dark (not nebula-blue) and
 *  blends into the app chrome on every theme. */
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

/** Coarse clock for the dashboard's elapsed/age labels — half-minute beats
 *  keep every "7m"-grade figure fresh without per-second re-renders. */
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

/**
 * The home surface — the Observatory (design 2a): the live knowledge graph
 * fills the pane as a cinematic hero, with the day's greeting and fleet
 * status up top, a centered search bar over corpus stat pills, live agents
 * pinned onto the graph field, and two glass panels docked at the bottom —
 * what's running right now, and what happened lately. "All quiet" is a
 * first-class state, not an empty one.
 */
export function Dashboard() {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const now = useSlowNow();

  const projects = useMemo(() => bootstrap.data?.projects ?? [], [bootstrap.data]);
  const workstreams = useAllWorkstreams(projects.length > 0);
  const archived = useArchivedWorkstreams();
  const fleet = useFleet(workstreams.data ?? []);

  // Most-recent docs feed the hero graph. A generous cap keeps the O(n²)
  // force pass quick; `total` still reflects the full index.
  const docsQuery = useDocs({ sort: "mtime", limit: GRAPH_DOC_LIMIT });
  const docs = useMemo(() => docsQuery.data?.items ?? [], [docsQuery.data]);
  const graph = useMemo(() => buildGraph(docs), [docs]);

  const [addOpen, setAddOpen] = useState(false);
  const [discoverOpen, setDiscoverOpen] = useState(false);

  const projectNames = useMemo(
    () => new Map(projects.map((p) => [p.path, p.mappedName])),
    [projects],
  );
  const projectName = (path: string): string =>
    projectNames.get(path) ?? path.split(/[\\/]/).pop() ?? path;

  // Quiet actives + freshly archived, newest first — the "what was I doing" list.
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
      merged: view.item.prNumber !== null,
    }));
    return [...active, ...gone].sort((a, b) => b.at - a.at).slice(0, RECENT_LIMIT);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fleet.idle, archived.data, projectNames]);

  // The quiet state's "your last agent shipped X" note — the freshest archived
  // workstream that opened a PR (merge status isn't tracked, so the copy only
  // claims the ship).
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

  // Quiet-state stat: workstreams touched (or retired) in the last 7 days.
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
    // Best-effort: focus the tab bound to the live session, when one exists.
    if (sessionId) selectAgentTab(view.item.id, sessionId);
  };

  // Graph pins: every live agent first (stage-colored, pulsing), the freshest
  // quiet workstreams filling the remaining slots.
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

  // "Start an agent": open the most recently touched project's board with the
  // new-workstream dialog popped (falls back to the first project).
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

  // First-run hero (design 4b): the lone first node + "Get set up" pill, shown
  // until the thoughts repo is configured — NOT gated on having a project, so a
  // configured user with no repos yet still gets the real knowledge graph (a
  // cloned thoughts corpus lights it up on its own). Gated on bootstrap.data so
  // the loading frame doesn't flash it.
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
  // First-run shares the full state's status line ("All quiet — no agents
  // running" at rest, live agents shown when working) — hasProjects is
  // forced there so the quiet copy matches instead of the add-a-repo nag.
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
      {/* The hero: the interactive nebula graph fills the pane edge-to-edge.
          Overlay containers are pointer-events-none, so pan/zoom/hover reach
          the graph everywhere except over actual controls. The star field's
          vignette lives inside the svg (an SVG mask on the star layer only —
          masking this whole div would multiply two near-black gradients and
          band into visible rings). */}
      {firstRun ? (
        <>
          {/* The empty graph paints the same dithered nebula backdrop as the
              populated hero — one implementation, no banding rings. */}
          <div className="absolute inset-0">
            <KnowledgeGraph graph={EMPTY_GRAPH} />
          </div>
          <FirstNodeField />
          {/* Dead-center command bar — absolutely positioned so the header's
              height can't skew it off the hero's true middle. */}
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

          {/* Bottom scrim — the band the docked glass panels sit on, deepening
              what the vignette already faded. Hidden until a repo is added,
              since the panels it carries are too. */}
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
            <div className="text-[11px] font-semibold uppercase tracking-[0.16em] text-ink-dim">
              {dateLabel}
            </div>
            <h1 className="mt-1.5 truncate text-[30px] font-semibold leading-tight tracking-[-0.02em] text-ink lg:text-[36px]">
              {greetingFor(now, userName)}
            </h1>
            <div className="mt-2.5 flex items-center gap-2 text-[13px] text-ink-dim">
              <span
                className={`h-[7px] w-[7px] shrink-0 rounded-full ${status.dot} ${
                  status.pulse ? "animate-pulse" : ""
                }`}
              />
              {status.text}
            </div>
          </div>
          <div className="pointer-events-auto flex shrink-0 items-center gap-2.5">
            <Button onClick={() => setDiscoverOpen(true)} className="text-[12.5px]">
              Discover
            </Button>
            <Button
              variant="primary"
              onClick={() => setAddOpen(true)}
              className="gap-1.5 text-[12.5px]"
            >
              <PlusIcon size={12} />
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
              <p className="max-w-[420px] text-center text-xs text-ink-dim">
                No documents yet — agents file research, plans, and notes here as they work,
                and they light up as a connected graph.
              </p>
            )}
          </div>
        )}

        <div className="min-h-6 flex-[1.5]" />

        {/* Live now + Latest activity dock only once a repo exists — with no
            projects there are no workstreams to surface. */}
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
              onOpenArchive={() => void navigate({ to: "/archive" })}
            />
          </div>
        )}
      </div>

      {firstRun && <SetupPill onClick={openWizard} />}

      <AddProjectDialog open={addOpen} onClose={() => setAddOpen(false)} onAdded={openProject} />
      <DiscoverDialog
        open={discoverOpen}
        onClose={() => setDiscoverOpen(false)}
        onAdded={openProject}
      />
    </div>
  );
}
