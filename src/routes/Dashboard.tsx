import { useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { Link, useNavigate } from "@tanstack/react-router";

import { HyprlayerMark } from "../components/Logo";
import { PageError } from "../components/ui";
import { PlusIcon } from "../components/icons";
import { useAllWorkstreams, useArchivedWorkstreams, useBootstrap, useDocs } from "../lib/queries";
import type { Project, WorkstreamView } from "../lib/bindings";
import { useElementWidth } from "../lib/useElementWidth";
import { AddProjectDialog } from "../features/projects/AddProjectDialog";
import { DiscoverDialog } from "../features/dashboard/DiscoverDialog";
import { buildGraph } from "../features/dashboard/graph";
import { useFleet } from "../features/dashboard/fleet";
import {
  LiveNowCard,
  NeedsYouBanner,
  RecentWorkstreams,
  type RecentRow,
} from "../features/dashboard/HomeFleet";
import { KnowledgeCard } from "../features/dashboard/KnowledgeCard";
import { selectAgentTab } from "../features/search/selectAgentTab";

/** How many most-recent docs feed the graph. Caps the O(n²) layout cost; the
 *  corpus may be larger (the graph dialog surfaces that as a caption). */
const GRAPH_DOC_LIMIT = 200;

/** How many rows the Recent-workstreams card shows. */
const RECENT_LIMIT = 6;

/** Dashboard width (px) from which the wide layout — Live now on top, the
 *  Knowledge card permanently in its expanded form, Recent workstreams below —
 *  fits comfortably (a display bigger than a laptop panel). Measured on the
 *  dashboard's own pane, not the window, so the resizable/collapsible project
 *  sidebar folds in automatically. Below it the stacked layout stays as-is.
 *  Set above a maximized 14" MacBook Pro's pane (~1376px at the "More Space"
 *  1800-wide scaling) so the built-in panel keeps the stacked view and only
 *  larger external displays go wide. */
const WIDE_MIN = 1440;

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

/**
 * The home surface — an operational fleet overview: what needs you, what's
 * running right now, what you touched recently, and the knowledge the agents
 * are building up, with the primary actions in the header.
 */
export function Dashboard() {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const now = useSlowNow();

  // Wide-layout gate: the pane's own width (0 until measured → stacked).
  const rootRef = useRef<HTMLDivElement>(null);
  const wide = useElementWidth(rootRef) >= WIDE_MIN;

  const projects = useMemo(() => bootstrap.data?.projects ?? [], [bootstrap.data]);
  const workstreams = useAllWorkstreams(projects.length > 0);
  const archived = useArchivedWorkstreams();
  const fleet = useFleet(workstreams.data ?? []);

  // Most-recent docs feed the Knowledge column and the graph. A generous cap
  // keeps the O(n²) force pass quick; `total` still reflects the full index.
  const docsQuery = useDocs({ sort: "mtime", limit: GRAPH_DOC_LIMIT });
  const docs = useMemo(() => docsQuery.data?.items ?? [], [docsQuery.data]);
  const graph = useMemo(() => buildGraph(docs), [docs]);

  const [addOpen, setAddOpen] = useState(false);
  const [discoverOpen, setDiscoverOpen] = useState(false);
  // Full-width knowledge-graph mode: the graph takes over the top of the page
  // (docs rail on its right) and the fleet cards slide below. Session-local —
  // minimizing restores the standard layout exactly. Only reachable below
  // WIDE_MIN: the wide layout carries the expanded card permanently.
  const [graphExpanded, setGraphExpanded] = useState(false);
  // Morph the swap: each `view-transition-name`d card (knowledge, graph stage,
  // live strip, recent list) glides between its two layouts. Falls back to an
  // instant swap without the API or under reduced motion.
  const toggleGraph = (next: boolean) => {
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const doc = document as Document & { startViewTransition?: (cb: () => void) => void };
    if (reduceMotion || !doc.startViewTransition) {
      setGraphExpanded(next);
      return;
    }
    // flushSync so the DOM reaches the new layout inside the transition
    // callback — the API snapshots before/after around it.
    doc.startViewTransition(() => flushSync(() => setGraphExpanded(next)));
  };

  const projectNames = useMemo(
    () => new Map(projects.map((p) => [p.path, p.mappedName])),
    [projects],
  );
  const projectName = (path: string): string =>
    projectNames.get(path) ?? path.split(/[\\/]/).pop() ?? path;

  // The single most urgent blocked agent gets the banner; everything else
  // lives in the strip, so nothing renders twice.
  const banner = fleet.live.find((l) => l.activity === "needs-you") ?? null;
  const strip = banner ? fleet.live.filter((l) => l !== banner) : fleet.live;

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

  if (bootstrap.error) return <PageError error={bootstrap.error} />;

  const configMissing = bootstrap.data?.config.state === "missing";
  const dateLabel = new Date(now).toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
  const emptyHint =
    projects.length === 0
      ? "Add a repo to start dispatching agents."
      : workstreams.isLoading
        ? "Loading workstreams…"
        : banner
          ? "Nothing else is running."
          : "All quiet — no agents running right now.";

  return (
    // The outer div is the width probe — no max-width, so it reads the pane.
    <div ref={rootRef}>
      {/* Wide cap: sized so the expanded graph (content − rail, aspect-locked)
          stays a component, not a takeover, on a maximized big display. */}
      <div
        className={`mx-auto flex w-full flex-col ${
          wide ? "max-w-[1400px] gap-5 px-12 py-9" : "max-w-[1160px] gap-[18px] px-6 py-7 lg:px-8"
        }`}
      >
        <header className="flex items-center gap-4">
          <HyprlayerMark size={48} />
          <div className="flex min-w-0 flex-col gap-1.5">
            <h1 className="text-[21px] font-bold leading-none tracking-[-0.01em] text-ink">
              {dateLabel}
            </h1>
            <FleetChips counts={fleet.counts} />
          </div>
          <span className="min-w-0 flex-1" />
          {/* Monolith action cluster (design pass 6b): the three header actions
              fuse into one hairline-bordered segment control — accent segment
              leads, dividers instead of per-button borders. */}
          <div className="flex h-[34px] shrink-0 items-stretch overflow-hidden rounded-[10px] border border-edge bg-panel shadow-[0_1px_3px_rgba(0,0,0,0.12)]">
            <button
              type="button"
              onClick={() => setAddOpen(true)}
              className="inline-flex items-center gap-1.5 bg-accent px-[15px] text-[12.5px] font-semibold text-accent-ink hover:opacity-90"
            >
              <PlusIcon size={12} />
              Add repo
            </button>
            <button
              type="button"
              onClick={() => setDiscoverOpen(true)}
              className="inline-flex items-center border-l border-edge px-3.5 text-[12.5px] font-semibold text-ink hover:bg-panel-2"
            >
              Discover
            </button>
            <Link
              to="/settings"
              className="flex items-center border-l border-edge px-[11px] text-ink-dim hover:bg-panel-2 hover:text-ink"
              title="Settings"
              aria-label="Settings"
            >
              <GearIcon />
            </Link>
          </div>
        </header>

        {configMissing ? (
          /* First-run CTA: replace the fleet with a single prominent prompt. */
          <section className="flex flex-col items-center gap-4 rounded-xl border border-brand/30 bg-brand/5 p-8 text-center">
            <p className="text-lg font-semibold text-ink">Finish setting up Hyprlayer</p>
            <p className="text-sm text-ink-dim">
              Configure your AI assistant and connect your thoughts repository to get started.
            </p>
            <Link
              to="/onboarding"
              className="inline-flex items-center gap-2 rounded-lg bg-brand px-5 py-2.5 text-sm font-medium text-white hover:opacity-90"
            >
              Finish setup →
            </Link>
          </section>
        ) : (
          <>
            {banner && (
              <NeedsYouBanner item={banner} onOpen={() => openWorkstream(banner.view, banner.sessionId)} />
            )}

            {wide ? (
              /* Wide layout — a maximized window on a big display spends its
                 height, not columns: Live now leads, the Knowledge card rides
                 permanently in its expanded form (interactive graph + docs
                 rail), Recent workstreams closes. No expand/minimize toggle —
                 this already is the expanded form. */
              <>
                <span className="-mb-2 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-dim [view-transition-name:live-label]">
                  Live now
                </span>
                <LiveNowCard
                  items={strip}
                  now={now}
                  onOpen={(item) => openWorkstream(item.view, item.sessionId)}
                  emptyHint={emptyHint}
                />
                <KnowledgeCard
                  docs={docs}
                  total={docsQuery.data?.total ?? 0}
                  graph={graph}
                  projectCount={projects.length}
                  loading={docsQuery.isLoading}
                  now={now}
                  expanded
                />
                <RecentWorkstreams
                  rows={recentRows}
                  now={now}
                  onOpenActive={(view) => openWorkstream(view)}
                  onOpenArchive={() => void navigate({ to: "/archive" })}
                />
              </>
            ) : graphExpanded ? (
              /* Graph takes the stage: full-width knowledge card up top, the
                 fleet cards pushed below. Minimize restores the grid exactly. */
              <>
                <KnowledgeCard
                  docs={docs}
                  total={docsQuery.data?.total ?? 0}
                  graph={graph}
                  projectCount={projects.length}
                  loading={docsQuery.isLoading}
                  now={now}
                  expanded
                  onToggleExpanded={toggleGraph}
                />
                <span className="-mb-2 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-dim [view-transition-name:live-label]">
                  Live now
                </span>
                <LiveNowCard
                  items={strip}
                  now={now}
                  onOpen={(item) => openWorkstream(item.view, item.sessionId)}
                  emptyHint={emptyHint}
                />
                <RecentWorkstreams
                  rows={recentRows}
                  now={now}
                  onOpenActive={(view) => openWorkstream(view)}
                  onOpenArchive={() => void navigate({ to: "/archive" })}
                />
              </>
            ) : (
              <>
                <span className="-mb-2 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-dim [view-transition-name:live-label]">
                  Live now
                </span>
                {/* No flex-1: the pair ends at its content (equal bottoms via the
                    grid row) instead of stretching to the app's bottom edge. */}
                <div className="grid grid-cols-1 gap-3.5 lg:grid-cols-[minmax(0,1fr)_400px]">
                  <div className="flex min-h-0 flex-col gap-4">
                    <LiveNowCard
                      items={strip}
                      now={now}
                      onOpen={(item) => openWorkstream(item.view, item.sessionId)}
                      emptyHint={emptyHint}
                    />
                    <RecentWorkstreams
                      rows={recentRows}
                      now={now}
                      onOpenActive={(view) => openWorkstream(view)}
                      onOpenArchive={() => void navigate({ to: "/archive" })}
                    />
                  </div>
                  <KnowledgeCard
                    docs={docs}
                    total={docsQuery.data?.total ?? 0}
                    graph={graph}
                    projectCount={projects.length}
                    loading={docsQuery.isLoading}
                    now={now}
                    expanded={false}
                    onToggleExpanded={toggleGraph}
                  />
                </div>
              </>
            )}
          </>
        )}

        <AddProjectDialog open={addOpen} onClose={() => setAddOpen(false)} onAdded={openProject} />
        <DiscoverDialog
          open={discoverOpen}
          onClose={() => setDiscoverOpen(false)}
          onAdded={openProject}
        />
      </div>
    </div>
  );
}

/** The header's dot-coded fleet summary — one chip per live state, or a quiet
 *  "all clear" when nothing is happening. */
function FleetChips({
  counts,
}: {
  counts: { running: number; needsYou: number; review: number; errors: number; queued: number };
}) {
  const chips: Array<{ key: string; dot: string; text: string; label: string }> = [];
  if (counts.running > 0) {
    chips.push({
      key: "running",
      dot: "bg-accent animate-pulse",
      text: "text-accent",
      label: `${counts.running} running`,
    });
  }
  if (counts.needsYou > 0) {
    chips.push({
      key: "needs-you",
      dot: "bg-attention",
      text: "text-attention",
      label: `${counts.needsYou} needs you`,
    });
  }
  if (counts.review > 0) {
    chips.push({
      key: "review",
      dot: "bg-ok",
      text: "text-ok",
      label: `${counts.review} review`,
    });
  }
  if (counts.errors > 0) {
    chips.push({ key: "errors", dot: "bg-bad", text: "text-bad", label: `${counts.errors} errored` });
  }
  if (counts.queued > 0) {
    chips.push({
      key: "queued",
      dot: "bg-edge",
      text: "text-ink-dim",
      label: `${counts.queued} queued`,
    });
  }
  if (chips.length === 0) {
    chips.push({ key: "quiet", dot: "bg-edge", text: "text-ink-dim", label: "all quiet" });
  }
  return (
    <div className="flex flex-wrap items-center gap-3 text-xs font-semibold">
      {chips.map((chip) => (
        <span key={chip.key} className={`inline-flex items-center gap-1.5 ${chip.text}`}>
          <span className={`h-1.5 w-1.5 rounded-full ${chip.dot}`} />
          {chip.label}
        </span>
      ))}
    </div>
  );
}

function GearIcon() {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="2.6" />
      <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M12.6 3.4l-1.4 1.4M4.8 11.2l-1.4 1.4" />
    </svg>
  );
}
