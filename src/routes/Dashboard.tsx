import { useMemo, useState, type ReactNode } from "react";
import { Link, useNavigate } from "@tanstack/react-router";

import { HyprlayerLogo } from "../components/Logo";
import { Card, PageError, Spinner } from "../components/ui";
import { PlusIcon } from "../components/icons";
import { useBootstrap, useDocs } from "../lib/queries";
import type { Project } from "../lib/bindings";
import { AddProjectDialog } from "../features/projects/AddProjectDialog";
import { DiscoverDialog } from "../features/dashboard/DiscoverDialog";
import { KnowledgeGraph } from "../features/dashboard/KnowledgeGraph";
import { buildGraph } from "../features/dashboard/graph";

/** How many most-recent docs feed the graph. Caps the O(n²) layout cost; the
 *  corpus may be larger (we surface that as a caption when it is). */
const GRAPH_DOC_LIMIT = 200;

/** The home surface: brand, the three primary actions, and a live view of the
 *  knowledge graph built from the thoughts index. */
export function Dashboard() {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();

  // Most-recent docs feed the graph. A generous cap keeps the layout legible
  // and the O(n²) force pass quick; `total` still reflects the full index.
  const docsQuery = useDocs({ sort: "mtime", limit: GRAPH_DOC_LIMIT });

  const [addOpen, setAddOpen] = useState(false);
  const [discoverOpen, setDiscoverOpen] = useState(false);

  const docs = useMemo(() => docsQuery.data?.items ?? [], [docsQuery.data]);
  const graph = useMemo(() => buildGraph(docs), [docs]);

  const openProject = (project: Project) =>
    void navigate({
      to: "/projects/$projectPath",
      params: { projectPath: project.path },
      search: {},
    });

  if (bootstrap.error) return <PageError error={bootstrap.error} />;

  const configMissing = bootstrap.data?.config.state === "missing";
  const projectCount = bootstrap.data?.projects.length ?? 0;
  const docTotal = docsQuery.data?.total ?? 0;
  const typeCount = new Set(graph.nodes.map((n) => n.type)).size;
  // One node per fetched doc; if the corpus is bigger, the graph is a slice.
  const graphed = graph.nodes.length;
  const capped = docTotal > graphed;

  return (
    <div className="mx-auto flex min-h-full max-w-5xl flex-col gap-8 p-6 lg:p-8">
      <header className="mb-2 mt-4 flex flex-col items-center gap-4 text-center">
        <HyprlayerLogo height={109} />
        <p className="text-[11.5px] font-medium uppercase tracking-[0.3em] text-ink-dim">
          Your repos, workstreams &amp; the knowledge between them
        </p>
      </header>

      {configMissing ? (
        /* First-run CTA: replace the three tiles with a single prominent prompt */
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
        <section className="grid grid-cols-1 gap-4 sm:grid-cols-3">
          <ActionTile
            title="Add"
            subtitle="Clone a repo or open one from disk"
            tone="brand"
            icon={<PlusIcon size={20} />}
            onClick={() => setAddOpen(true)}
          />
          <ActionTile
            title="Discover"
            subtitle="Browse your GitHub repos to clone"
            tone="violet"
            icon={<CompassIcon />}
            onClick={() => setDiscoverOpen(true)}
          />
          <ActionTile
            title="Settings"
            subtitle="Binaries, roots, and preferences"
            tone="neutral"
            icon={<GearIcon />}
            onClick={() => void navigate({ to: "/settings" })}
          />
        </section>
      )}

      <Card className="flex flex-col p-0">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-edge px-5 py-3">
          <div>
            <h2 className="text-sm font-semibold tracking-wide text-ink">Knowledge graph</h2>
            {capped && (
              <p className="mt-0.5 text-xs text-ink-dim">
                most-recent {graphed} of {docTotal} documents
              </p>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Stat label="documents" value={docTotal} />
            <Stat label="links" value={graph.edges.length} />
            <Stat label="types" value={typeCount} />
            <Stat label="projects" value={projectCount} />
          </div>
        </div>
        <div className="h-[560px] p-4">
          {docsQuery.isLoading ? (
            <div className="flex h-full items-center justify-center">
              <Spinner />
            </div>
          ) : docsQuery.error ? (
            <PageError error={docsQuery.error} />
          ) : (
            <KnowledgeGraph graph={graph} />
          )}
        </div>
      </Card>

      <AddProjectDialog open={addOpen} onClose={() => setAddOpen(false)} onAdded={openProject} />
      <DiscoverDialog
        open={discoverOpen}
        onClose={() => setDiscoverOpen(false)}
        onAdded={openProject}
      />
    </div>
  );
}

type Tone = "brand" | "violet" | "neutral";

function ActionTile({
  title,
  subtitle,
  icon,
  tone,
  onClick,
}: {
  title: string;
  subtitle: string;
  icon: ReactNode;
  tone: Tone;
  onClick: () => void;
}) {
  const tones: Record<Tone, { icon: string; hover: string }> = {
    brand: { icon: "text-brand", hover: "hover:border-brand/50" },
    violet: { icon: "text-[#c084fc]", hover: "hover:border-[#c084fc]/50" },
    neutral: { icon: "text-ink-dim", hover: "hover:border-ink-dim/50" },
  };
  return (
    <button
      type="button"
      onClick={onClick}
      className={`group flex flex-col items-start gap-4 rounded-xl border border-edge bg-panel p-6 text-left transition-colors hover:bg-panel-2 ${tones[tone].hover}`}
    >
      <span
        className={`flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border border-edge bg-panel-2 ${tones[tone].icon}`}
      >
        {icon}
      </span>
      <span className="min-w-0">
        <span className="block text-lg font-semibold text-ink">{title}</span>
        <span className="mt-0.5 block text-sm text-ink-dim">{subtitle}</span>
      </span>
    </button>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <span className="inline-flex items-baseline gap-1.5 rounded-md border border-edge bg-panel-2 px-2.5 py-1">
      <span className="text-sm font-semibold tabular-nums text-ink">{value}</span>
      <span className="text-xs text-ink-dim">{label}</span>
    </span>
  );
}

function CompassIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="m15.5 8.5-2 5-5 2 2-5 5-2Z"
        fill="currentColor"
        fillOpacity="0.25"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function GearIcon() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}
