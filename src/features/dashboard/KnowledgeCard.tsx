/**
 * The home dashboard's Knowledge surface. Two shapes, one card:
 *  - collapsed (the right column): search affordance, nebula preview, corpus
 *    stats, and the freshest docs as library-style rows;
 *  - expanded (full width): the interactive nebula graph fills the left, and
 *    the same search/stats/doc rows move into a rail on the right.
 * The parent owns the `expanded` flag, so minimizing restores the collapsed
 * dashboard exactly as it was.
 */
import { useNavigate } from "@tanstack/react-router";

import { Spinner } from "../../components/ui";
import type { DocMeta } from "../../lib/bindings";
import { effectiveChord } from "../../lib/keybindings/resolve";
import { formatChord } from "../../lib/keybindings/chord";
import { IS_MACOS } from "../../lib/platform";
import { useBootstrap } from "../../lib/queries";
import { openSearch } from "../search/searchStore";
import { docTitle } from "../library/text";
import { formatRelativeTime } from "../session/toolDisplay";
import { KnowledgeGraph } from "./KnowledgeGraph";
import { MiniGraph } from "./MiniGraph";
import type { Graph } from "./graph";

/** How many doc rows the card shows. */
const RECENT_DOCS = 6;

// Doc-type chips reuse the stage/doc accent tokens (same mapping as the
// Changes panel's Documents section); off-schema types fall to neutral.
const TYPE_CHIP: Record<string, string> = {
  plan: "bg-plan/15 text-plan",
  research: "bg-research/15 text-research",
  handoff: "bg-handoff/15 text-handoff",
};

const STATUS_CHIP: Record<string, string> = {
  draft: "bg-panel-2 text-ink-dim",
  active: "bg-brand/15 text-brand",
  implemented: "bg-ok/15 text-ok",
  superseded: "bg-warn/15 text-warn",
  archived: "bg-panel-2 text-ink-dim",
};

type Props = {
  /** Most-recent docs (mtime desc) — rows come from the head of this list. */
  docs: DocMeta[];
  /** Full corpus size (may exceed `docs.length`). */
  total: number;
  graph: Graph;
  projectCount: number;
  loading: boolean;
  now: number;
  /** Full-width graph mode; the parent restacks the dashboard around it. */
  expanded: boolean;
  /** Absent = no expand/minimize affordance (the wide dashboard renders the
   *  card permanently expanded, so there's nothing to toggle). */
  onToggleExpanded?: (expanded: boolean) => void;
};

export function KnowledgeCard({
  docs,
  total,
  graph,
  projectCount,
  loading,
  now,
  expanded,
  onToggleExpanded,
}: Props) {
  const header = (
    <div className="flex items-center">
      <span className="text-[12.5px] font-bold text-ink">Knowledge</span>
      {expanded && total > docs.length && (
        <span className="ml-3 text-[11px] text-ink-dim">
          most-recent {docs.length} of {total} documents
        </span>
      )}
      <span className="flex-1" />
      {onToggleExpanded && (
        <button
          type="button"
          onClick={() => onToggleExpanded(!expanded)}
          className="text-[11.5px] font-semibold text-accent hover:opacity-80"
        >
          {expanded ? "Minimize ↙" : "Open graph →"}
        </button>
      )}
    </div>
  );

  if (expanded) {
    return (
      <div className="flex min-h-0 flex-col gap-3 overflow-hidden rounded-xl border border-edge bg-panel p-4 [view-transition-name:kg-card]">
        {header}
        <div className="flex min-h-0 gap-4">
          {/* Aspect-locked to the graph's 960×560 stage so the nebula fills
              the box edge-to-edge — no letterbox bands from `meet` scaling. */}
          <div className="min-w-0 flex-1 self-start [aspect-ratio:960/610] [view-transition-name:kg-stage]">
            <KnowledgeGraph graph={graph} />
          </div>
          <div className="flex w-[400px] shrink-0 flex-col gap-3 overflow-hidden">
            <SearchAffordance total={total} />
            <CorpusStats total={total} links={graph.edges.length} projects={projectCount} />
            <RecentDocs docs={docs} loading={loading} now={now} />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-col gap-3 overflow-hidden rounded-xl border border-edge bg-panel p-4 [view-transition-name:kg-card]">
      {header}
      <SearchAffordance total={total} />
      <button
        type="button"
        onClick={() => onToggleExpanded?.(true)}
        className="block w-full shrink-0 [view-transition-name:kg-stage]"
        title="Expand knowledge graph"
        aria-label="Expand knowledge graph"
      >
        <MiniGraph graph={graph} />
      </button>
      <CorpusStats total={total} links={graph.edges.length} projects={projectCount} />
      <RecentDocs docs={docs} loading={loading} now={now} />
    </div>
  );
}

function SearchAffordance({ total }: { total: number }) {
  const bootstrap = useBootstrap();
  const chord = effectiveChord(
    bootstrap.data?.settings.keybindings?.overrides ?? {},
    "search.open",
    IS_MACOS,
  );
  return (
    <button
      type="button"
      onClick={openSearch}
      className="flex items-center gap-2 rounded-[9px] border border-edge bg-surface px-3 py-2 text-xs text-ink-dim hover:border-ink-dim/50"
    >
      <SearchGlyph />
      <span className="truncate">
        Search {total > 0 ? `${total} ${total === 1 ? "document" : "documents"}` : "documents"}…
      </span>
      <span className="min-w-0 flex-1" />
      {chord && (
        <span className="shrink-0 rounded-[5px] bg-panel-2 px-1.5 py-px font-mono text-[10px]">
          {formatChord(chord, IS_MACOS)}
        </span>
      )}
    </button>
  );
}

function CorpusStats({ total, links, projects }: { total: number; links: number; projects: number }) {
  return (
    <div className="flex gap-4 text-[11.5px] text-ink-dim">
      <span>
        <b className="font-semibold text-ink">{total}</b> documents
      </span>
      <span>
        <b className="font-semibold text-ink">{links}</b> links
      </span>
      <span>
        <b className="font-semibold text-ink">{projects}</b>{" "}
        {projects === 1 ? "project" : "projects"}
      </span>
    </div>
  );
}

function RecentDocs({ docs, loading, now }: { docs: DocMeta[]; loading: boolean; now: number }) {
  const navigate = useNavigate();
  return (
    <>
      <span className="text-[10.5px] font-bold uppercase tracking-[0.08em] text-ink-dim">
        Recent
      </span>
      <div className="-mx-2 flex min-h-0 flex-1 flex-col overflow-hidden">
        {loading && (
          <div className="flex justify-center py-6">
            <Spinner />
          </div>
        )}
        {!loading && docs.length === 0 && (
          <p className="px-2 text-xs text-ink-dim">
            No documents yet — agents file research and plans here as they work.
          </p>
        )}
        {docs.slice(0, RECENT_DOCS).map((doc) => {
          const type = doc.fm.type?.toLowerCase();
          const status = doc.fm.status?.toLowerCase();
          return (
            <button
              key={doc.id}
              type="button"
              onClick={() => void navigate({ to: "/library", search: { doc: doc.id } })}
              className="flex flex-col gap-1 rounded-[9px] px-2.5 py-2 text-left hover:bg-panel-2/60"
            >
              <span className="truncate text-xs font-semibold leading-[1.35] text-ink">
                {docTitle(doc)}
              </span>
              <span className="flex items-center gap-1.5 overflow-hidden">
                {type && <DocChip label={type} tone={TYPE_CHIP[type] ?? "bg-panel-2 text-ink-dim"} />}
                {status && (
                  <DocChip label={status} tone={STATUS_CHIP[status] ?? "bg-panel-2 text-ink-dim"} />
                )}
                <span className="truncate text-[10.5px] text-ink-dim">
                  {doc.fm.author ?? doc.scope} ·{" "}
                  {formatRelativeTime(doc.mtimeMs, now).replace(" ago", "")}
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </>
  );
}

function DocChip({ label, tone }: { label: string; tone: string }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center rounded-full px-2 py-px text-[9.5px] font-bold ${tone}`}
    >
      {label}
    </span>
  );
}

function SearchGlyph() {
  return (
    <svg
      aria-hidden="true"
      width="12"
      height="12"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
    >
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5l3 3" />
    </svg>
  );
}
