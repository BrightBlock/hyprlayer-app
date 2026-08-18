/**
 * The home dashboard's fleet fold: every active workstream classified by its
 * live agent activity, with the detail the Live-now strip renders (current
 * tool line, context-meter fill, elapsed anchor, queued follow-ups).
 *
 * Three sources merge per workstream, worst state first:
 *  - the in-memory session store (rich: transcript, pending permission, meter),
 *  - pty agents from multiplexer mode (blocked / working / done),
 *  - the backend session registry (live process states, covers sessions this
 *    webview never attached, e.g. right after a reload; push-refreshed by
 *    SessionLifecycle events).
 */
import { useMemo } from "react";
import { useShallow } from "zustand/shallow";

import type { ActiveSessionView, WorkstreamView } from "@/lib/bindings";
import { useActiveSessions } from "@/lib/queries";
import type { AgentState } from "@/features/multiplexer/engineState";
import { rollupForWorkstream, useMultiplexerStore } from "@/features/multiplexer/multiplexerStore";
import { apiContextWindow } from "@/lib/domain/models";
import { useModelWindowMap } from "@/features/session/modelWindowsStore";
import {
  classifyEntry,
  useSessionStore,
  type SessionEntry,
  type WorkstreamActivity,
} from "@/features/session/sessionStore";
import { baseName, toolSummary } from "@/features/session/toolDisplay";
import type { TranscriptItem } from "@/features/session/transcript";

export type FleetActivity = Exclude<WorkstreamActivity, "idle">;

export type FleetItem = {
  view: WorkstreamView;
  activity: FleetActivity;
  /** The most urgent session (the tab to select on open), when known. */
  sessionId: string | null;
  /** Mono context line, the tool being run or awaited. */
  line: string | null;
  /** Context-meter fill for a working session. */
  meter: { tokens: number; fraction: number } | null;
  /** Epoch ms anchoring the row's elapsed counter (null = don't show one). */
  since: number | null;
  /** Follow-up turns queued behind in-flight turns, across the workstream. */
  queued: number;
};

export type Fleet = {
  /** Workstreams with live agent activity, most urgent first. */
  live: FleetItem[];
  /** Active workstreams with nothing to flag, input order preserved. */
  idle: WorkstreamView[];
  counts: { running: number; needsYou: number; review: number; errors: number; queued: number };
};

// Worst-first, mirroring the sidebar dot's fold (WorkstreamActivityDot).
const RANK: Record<WorkstreamActivity, number> = {
  idle: 0,
  review: 1,
  background: 2,
  error: 3,
  working: 4,
  "needs-you": 5,
};

const PTY_TO_ACTIVITY: Record<AgentState, WorkstreamActivity> = {
  blocked: "needs-you",
  working: "working",
  done: "review",
  idle: "idle",
};

const FILE_VERBS: Record<string, string> = {
  Read: "reading",
  Write: "writing",
  Edit: "editing",
  MultiEdit: "editing",
  NotebookEdit: "editing",
};

/** One tool call as a low-key present-tense phrase ("editing Dashboard.tsx"). */
function toolLine(name: string, input: string): string {
  const summary = toolSummary(name, input);
  const verb = FILE_VERBS[name];
  if (verb) return summary ? `${verb} ${baseName(summary)}` : verb;
  if (name === "Bash" || name === "BashOutput") return summary || "running a command";
  if (name === "Task") return summary ? `subagent: ${summary}` : "running a subagent";
  return summary ? `${name} · ${summary}` : name;
}

/** What a working session is on right now: its most recent tool call, else a
 *  generic "thinking". Scans from the tail, so long transcripts stay cheap. */
export function workingLine(items: readonly TranscriptItem[]): string {
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item.kind === "tool") return toolLine(item.name, item.input);
    if (item.kind === "user") break; // no tool call yet this turn
  }
  return "thinking…";
}

/** What a parked session is waiting on: the pending ask's command/path. */
function pendingLine(entry: SessionEntry): string {
  if (entry.pendingQuestion) return "waiting on an answer";
  const ask =
    entry.pendingPermission ??
    entry.items.find(
      (it): it is Extract<TranscriptItem, { kind: "permission" }> =>
        it.kind === "permission" && it.decision === null,
    );
  if (!ask) return "waiting on a permission";
  const summary = toolSummary(ask.toolName, ask.input);
  return summary ? `${ask.toolName} · ${summary}` : `${ask.toolName} permission`;
}

// The picked-session fields, flattened to one string so `useShallow` can bail
// out on streaming deltas that don't change what the dashboard shows. \0 is
// safe: none of these fields carry NUL bytes, and the line is single-line.
export type Picked = {
  activity: FleetActivity;
  sessionId: string;
  line: string;
  tokens: number | null;
  model: string | null;
  window: number | null;
  since: number | null;
  queued: number;
};

/**
 * Pure core of the store selector: for each workstream, the most urgent
 * session's dashboard-relevant fields as one key string, plus the queued
 * follow-up total across all of its sessions. Ordered by `wsIds`. Exported
 * for tests.
 */
export function selectFleetKeys(
  sessions: Record<string, SessionEntry>,
  wsIds: readonly string[],
): string[] {
  const wanted = new Set(wsIds);
  const best = new Map<string, { rank: number; sessionId: string; entry: SessionEntry }>();
  const queued = new Map<string, number>();
  for (const [sessionId, entry] of Object.entries(sessions)) {
    const wsId = entry.workstreamId;
    if (!wanted.has(wsId)) continue;
    queued.set(wsId, (queued.get(wsId) ?? 0) + entry.queued.length);
    const activity = classifyEntry(entry);
    if (activity === "idle") continue;
    const rank = RANK[activity];
    const cur = best.get(wsId);
    if (!cur || rank > cur.rank) best.set(wsId, { rank, sessionId, entry });
  }

  const out: string[] = [];
  for (const wsId of wsIds) {
    const b = best.get(wsId);
    if (!b) continue;
    const activity = classifyEntry(b.entry);
    const live = activity === "working" || activity === "background";
    const line =
      activity === "needs-you"
        ? pendingLine(b.entry)
        : activity === "working"
          ? workingLine(b.entry.items)
          : activity === "background"
            ? "background tasks running"
            : "";
    out.push(
      [
        wsId,
        activity,
        b.sessionId,
        line,
        live ? (b.entry.contextTokens ?? "") : "",
        live ? (b.entry.model ?? "") : "",
        live ? (b.entry.contextWindow ?? "") : "",
        activity === "working" ? (b.entry.workingSince ?? "") : "",
        queued.get(wsId) ?? 0,
      ].join("\0"),
    );
  }
  return out;
}

function parsePicked(keys: string[]): Map<string, Picked> {
  const out = new Map<string, Picked>();
  for (const key of keys) {
    const [wsId, activity, sessionId, line, tokens, model, window, since, queued] =
      key.split("\0");
    out.set(wsId, {
      activity: activity as FleetActivity,
      sessionId,
      line,
      tokens: tokens ? Number(tokens) : null,
      model: model || null,
      window: window ? Number(window) : null,
      since: since ? Number(since) : null,
      queued: Number(queued) || 0,
    });
  }
  return out;
}

/** Registry states we can classify without a transcript. A bare "running" is
 *  ambiguous (mid-turn vs resting at ready), so it stays unmapped rather than
 *  reading as forever-working. */
function registryActivity(state: string): FleetActivity | null {
  if (state === "awaiting_permission" || state === "awaiting_question") return "needs-you";
  if (state === "starting") return "working";
  return null;
}

function registryFold(
  views: ActiveSessionView[],
): Map<string, { activity: FleetActivity; sessionId: string; since: number | null }> {
  const out = new Map<string, { activity: FleetActivity; sessionId: string; since: number | null }>();
  for (const av of views) {
    const activity = registryActivity(av.session.state);
    if (!activity) continue;
    const wsId = av.session.workstreamId;
    const cur = out.get(wsId);
    if (!cur || RANK[activity] > RANK[cur.activity]) {
      out.set(wsId, { activity, sessionId: av.session.id, since: av.session.startedAt });
    }
  }
  return out;
}

/** Fold one workstream's three sources into its dashboard row. Exported for
 *  tests. */
export function foldFleetItem(
  view: WorkstreamView,
  picked: Picked | undefined,
  pty: WorkstreamActivity,
  reg: { activity: FleetActivity; sessionId: string; since: number | null } | undefined,
  windows: ReadonlyMap<string, number>,
): FleetItem | null {
  let activity: WorkstreamActivity = picked?.activity ?? "idle";
  let source: "store" | "pty" | "reg" = "store";
  if (RANK[pty] > RANK[activity]) {
    activity = pty;
    source = "pty";
  }
  if (reg && RANK[reg.activity] > RANK[activity]) {
    activity = reg.activity;
    source = "reg";
  }
  if (activity === "idle") return null;

  const base: FleetItem = {
    view,
    activity,
    sessionId: null,
    line: null,
    meter: null,
    since: null,
    queued: picked?.queued ?? 0,
  };
  if (source === "reg" && reg) return { ...base, sessionId: reg.sessionId, since: reg.since };
  if (source === "pty" || !picked) return base;

  let meter: FleetItem["meter"] = null;
  if (picked.tokens !== null) {
    const window =
      picked.window ??
      apiContextWindow(picked.model, windows) ??
      (picked.model?.includes("[1m]") ? 1_000_000 : 200_000);
    meter = { tokens: picked.tokens, fraction: Math.min(1, picked.tokens / window) };
  }
  return {
    ...base,
    sessionId: picked.sessionId,
    line: picked.line || null,
    meter,
    since: picked.since,
  };
}

/**
 * Classify `views` into the home dashboard's fleet: live rows (most urgent
 * first, ties keep input order) + quiet ones, with the summary-chip counts.
 */
export function useFleet(views: WorkstreamView[]): Fleet {
  const wsIds = useMemo(() => views.map((v) => v.item.id), [views]);
  const storeKeys = useSessionStore(useShallow((s) => selectFleetKeys(s.sessions, wsIds)));
  const picked = useMemo(() => parsePicked(storeKeys), [storeKeys]);
  // One rollup key per workstream (not the agents map) so pty output churn
  // doesn't re-render the dashboard unless a rollup actually flips.
  const ptyKeys = useMultiplexerStore(
    useShallow((s) => wsIds.map((id) => rollupForWorkstream(s.agents, id) ?? "idle")),
  );
  const registry = useActiveSessions(views.length > 0);
  const windows = useModelWindowMap();

  return useMemo(() => {
    const reg = registryFold(registry.data ?? []);
    const live: FleetItem[] = [];
    const idle: WorkstreamView[] = [];
    let queued = 0;
    views.forEach((view, i) => {
      const pty = PTY_TO_ACTIVITY[(ptyKeys[i] ?? "idle") as AgentState] ?? "idle";
      const item = foldFleetItem(view, picked.get(view.item.id), pty, reg.get(view.item.id), windows);
      queued += item?.queued ?? picked.get(view.item.id)?.queued ?? 0;
      if (item) live.push(item);
      else idle.push(view);
    });
    live.sort((a, b) => RANK[b.activity] - RANK[a.activity]);
    const count = (of: (a: FleetActivity) => boolean) => live.filter((l) => of(l.activity)).length;
    return {
      live,
      idle,
      counts: {
        running: count((a) => a === "working" || a === "background"),
        needsYou: count((a) => a === "needs-you"),
        review: count((a) => a === "review"),
        errors: count((a) => a === "error"),
        queued,
      },
    };
  }, [views, picked, ptyKeys, registry.data, windows]);
}
