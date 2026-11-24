import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAllWorkstreams, useActiveSessions, useDocs, MIN_DOC_TEXT_QUERY } from "@/lib/queries";
import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { useDebounced } from "@/lib/useDebounced";
import { fuzzyScore, type SearchResult } from "./search";
import type { WorkstreamActivity } from "@/features/session/sessionStore";
import { basename } from "@/lib/path";
import { COMMANDS } from "@/lib/keybindings/commands";
import { useHasCommand } from "@/lib/keybindings/registry";
import { useHasPausedTour } from "@/features/tour/tourStore";

/** Help commands, by name: typed for, never listed for an empty query. */
export function actionResultsFor(query: string, tourPaused: boolean): SearchResult[] {
  if (!query.trim()) return [];
  return COMMANDS.filter((c) => c.category === "Help" && (c.id !== "help.resumeTour" || tourPaused)).flatMap(
    (c): SearchResult[] => {
      const score = fuzzyScore(`${c.label} help`, query);
      return score === null
        ? []
        : [{ kind: "action", key: `action:${c.id}`, title: c.label, subtitle: "Help", score, nav: { type: "command", id: c.id } }];
    },
  );
}

function sessionStateToActivity(state: string): WorkstreamActivity {
  if (state === "awaiting_permission" || state === "awaiting_question") return "needs-you";
  if (state === "starting" || state === "running") return "working";
  if (state === "crashed" || state === "spawn_failed") return "error";
  if (state === "completed" || state === "interrupted" || state === "killed") return "review";
  return "idle";
}

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
        score: score + (query ? 0 : -(v.item.updatedAt ?? 0) / 1e13),
        nav: { type: "workstream", projectPath: v.item.projectPath, workstreamId: v.item.id },
        tag: v.item.stage,
      });
    }
    return out;
  }, [workstreams.data, query]);

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
        score: 1000 - i,
        nav: { type: "doc", id: d.id },
        tag: d.fm.type ?? undefined,
      }),
    );
  }, [docsQuery.data]);

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
        score: 1000 - i,
        nav: { type: "file", workstreamId: activeWorkstreamId, path },
      }),
    );
  }, [filesQuery.data, activeWorkstreamId]);

  const resumable = useHasCommand("help.resumeTour");
  const paused = useHasPausedTour() && resumable;
  const actionResults = useMemo(() => actionResultsFor(query, paused), [query, paused]);

  return useMemo(
    () => [...actionResults, ...wsResults, ...agentResults, ...fileResults, ...docResults],
    [actionResults, wsResults, agentResults, fileResults, docResults],
  );
}
