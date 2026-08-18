import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAllWorkstreams, useActiveSessions, useDocs, MIN_DOC_TEXT_QUERY } from "@/lib/queries";
import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { useDebounced } from "@/lib/useDebounced";
import { fuzzyScore, type SearchResult } from "./search";
import type { WorkstreamActivity } from "@/features/session/sessionStore";
import { basename } from "@/lib/path";

/** Map a backend session.state string to a WorkstreamActivity badge. */
function sessionStateToActivity(state: string): WorkstreamActivity {
  if (state === "awaiting_permission" || state === "awaiting_question") return "needs-you";
  if (state === "starting" || state === "running") return "working";
  if (state === "crashed" || state === "spawn_failed") return "error";
  if (state === "completed" || state === "interrupted" || state === "killed") return "review";
  return "idle";
}

/** Activity urgency rank, higher is more urgent. For empty-query agent sort. */
const ACTIVITY_RANK: Record<WorkstreamActivity, number> = {
  idle: 0,
  review: 1,
  background: 2,
  error: 3,
  working: 4,
  "needs-you": 5,
};

export function useSearchSources(query: string, activeWorkstreamId?: string): SearchResult[] {
  const debounced = useDebounced(query, 120);

  // ── Workstreams ────────────────────────────────────────────────────────────
  // Already cached by the sidebar tree; filter client-side.
  const workstreams = useAllWorkstreams();
  const wsResults = useMemo<SearchResult[]>(() => {
    const views = workstreams.data ?? [];
    const out: SearchResult[] = [];
    for (const v of views) {
      const hay = `${v.item.title} ${v.item.branch} ${v.item.projectPath}`;
      const score = fuzzyScore(hay, query);
      if (score === null) continue;
      out.push({
        kind: "workstream",
        key: `ws:${v.item.id}`,
        title: v.item.title,
        subtitle: v.item.branch,
        // Empty query: recent first (negate updatedAt, larger = newer)
        score: score + (query ? 0 : -(v.item.updatedAt ?? 0) / 1e13),
        nav: { type: "workstream", projectPath: v.item.projectPath, workstreamId: v.item.id },
        tag: v.item.stage,
      });
    }
    return out;
  }, [workstreams.data, query]);

  // ── Docs ───────────────────────────────────────────────────────────────────
  // Reuse the full-text index. Only once the query clears the minimum length,
  // below that, a text query is a full-corpus disk scan (see
  // MIN_DOC_TEXT_QUERY), and this fires on every focus refetch.
  const hasDocQuery = debounced.length >= MIN_DOC_TEXT_QUERY;
  const docsQuery = useDocs(
    { text: hasDocQuery ? debounced : null, sort: hasDocQuery ? "relevance" : "mtime", limit: 12 },
    hasDocQuery,
  );
  const docResults = useMemo<SearchResult[]>(() => {
    const items = docsQuery.data?.items ?? [];
    return items.map(
      (d, i): SearchResult => ({
        kind: "doc",
        key: `doc:${d.id}`,
        title: d.fm.title ?? d.relPath,
        subtitle: [d.project, d.fm.type].filter(Boolean).join(" · ") || undefined,
        score: 1000 - i, // already relevance-ranked by the index
        nav: { type: "doc", id: d.id },
        tag: d.fm.type ?? undefined,
      }),
    );
  }, [docsQuery.data]);

  // ── Agents ─────────────────────────────────────────────────────────────────
  // All live sessions from the backend registry (covers sessions whose tab was
  // never opened this run, which the frontend sessionStore misses).
  const activeSessions = useActiveSessions();
  const agentResults = useMemo<SearchResult[]>(() => {
    const sessions = activeSessions.data ?? [];
    const out: SearchResult[] = [];
    for (const v of sessions) {
      const hay = `${v.workstreamTitle} ${v.session.stage} ${v.session.model ?? ""}`;
      const score = fuzzyScore(hay, query);
      if (score === null) continue;
      const activity = sessionStateToActivity(v.session.state);
      out.push({
        kind: "agent",
        key: `agent:${v.session.id}`,
        title: v.workstreamTitle,
        subtitle: v.session.stage,
        // Empty query: urgency first, then recency
        score:
          score +
          (query ? 0 : ACTIVITY_RANK[activity] * 100 - (v.session.startedAt ?? 0) / 1e13),
        nav: {
          type: "agent",
          projectPath: v.projectPath,
          workstreamId: v.session.workstreamId,
          sessionId: v.session.id,
        },
        activity,
        tag: v.session.model ?? undefined,
      });
    }
    return out;
  }, [activeSessions.data, query]);

  // ── Files ──────────────────────────────────────────────────────────────────
  // Scoped to the active workstream; only present when a query is typed.
  const fileQueryEnabled = !!activeWorkstreamId && debounced.length > 0;
  const filesQuery = useQuery({
    queryKey: ["worktree-search-files", activeWorkstreamId, debounced],
    queryFn: () => unwrap(commands.worktreeSearchFiles(activeWorkstreamId!, debounced, 12)),
    enabled: fileQueryEnabled,
  });
  const fileResults = useMemo<SearchResult[]>(() => {
    if (!activeWorkstreamId) return [];
    const paths = filesQuery.data ?? [];
    return paths.map(
      (path, i): SearchResult => ({
        kind: "file",
        key: `file:${activeWorkstreamId}:${path}`,
        title: basename(path),
        subtitle: path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : undefined,
        score: 1000 - i, // already ranked by the worktree search
        nav: { type: "file", workstreamId: activeWorkstreamId, path },
      }),
    );
  }, [filesQuery.data, activeWorkstreamId]);

  return useMemo(
    () => [...wsResults, ...agentResults, ...fileResults, ...docResults],
    [wsResults, agentResults, fileResults, docResults],
  );
}
