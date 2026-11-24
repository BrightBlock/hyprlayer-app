import { useMemo } from "react";
import { useShallow } from "zustand/shallow";

import type { ActiveSessionView, Session, WorkstreamView } from "@/lib/bindings";
import type { PtyAgent } from "@/lib/domain/agentControl";
import { goalActivity } from "@/lib/domain/goals";
import { useActiveSessions, usePtyAgents } from "@/lib/queries";
import type { AgentState } from "@/features/multiplexer/engineState";
import { toAgentState } from "@/features/multiplexer/ptyAgentState";
import { rollupForWorkstream, useMultiplexerStore } from "@/features/multiplexer/multiplexerStore";
import {
  classifyEntry,
  useSessionStore,
  type SessionEntry,
  type WorkstreamActivity,
} from "@/features/session/sessionStore";
import {
  baseName,
  isHiddenTool,
  subagentType,
  toolDisplayName,
  toolSummary,
  type ToolNaming,
} from "@/features/session/toolDisplay";
import type { TranscriptItem } from "@/features/session/transcript";

export type FleetActivity = Exclude<WorkstreamActivity, "idle">;

export type FleetItem = {
  view: WorkstreamView;
  activity: FleetActivity;
  sessionId: string | null;
  line: string | null;
  meter: { tokens: number; fraction: number } | null;
  since: number | null;
  queued: number;
};

type Fleet = {
  live: FleetItem[];
  idle: WorkstreamView[];
  counts: { running: number; needsYou: number; review: number; errors: number; queued: number };
};

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

function toolLine(name: string, input: string, naming?: ToolNaming): string {
  const summary = toolSummary(name, input, naming);
  const verb = FILE_VERBS[name];
  if (verb) return summary ? `${verb} ${baseName(summary)}` : verb;
  if (name === "Bash" || name === "BashOutput") return summary || "running a command";
  if (name === "Task" || name === "Agent") {
    const type = subagentType(name, input);
    if (type && summary !== type) return `${type}: ${summary}`;
    return type ? `running ${type}` : summary ? `subagent: ${summary}` : "running a subagent";
  }
  const label = toolDisplayName(name, naming);
  return summary ? `${label} · ${summary}` : label;
}

export function workingLine(items: readonly TranscriptItem[], naming?: ToolNaming): string {
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item.kind === "tool" && !item.parentId && !isHiddenTool(item)) return toolLine(item.name, item.input, naming);
    if (item.kind === "user") break;
  }
  return "thinking…";
}

function pendingLine(entry: SessionEntry): string {
  if (entry.pendingQuestion) return "waiting on an answer";
  const ask =
    entry.pendingPermission ??
    entry.items.find(
      (it): it is Extract<TranscriptItem, { kind: "permission" }> =>
        it.kind === "permission" && it.decision === null,
    );
  if (!ask) return "waiting on a permission";
  const summary = toolSummary(ask.toolName, ask.input, entry);
  const label = toolDisplayName(ask.toolName, entry);
  return summary ? `${label} · ${summary}` : `${label} permission`;
}

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
          ? workingLine(b.entry.items, b.entry)
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

// The goal's brief first, as `classifyEntry` reads it for sessions in the store.
function registryActivity(session: Session): FleetActivity | null {
  const goal = goalActivity(session.goal);
  if (goal && goal !== "idle") return goal;
  if (session.state === "awaiting_permission" || session.state === "awaiting_question") return "needs-you";
  if (session.state === "starting") return "working";
  return null;
}

function registryFold(
  views: ActiveSessionView[],
): Map<string, { activity: FleetActivity; sessionId: string; since: number | null }> {
  const out = new Map<string, { activity: FleetActivity; sessionId: string; since: number | null }>();
  for (const av of views) {
    const activity = registryActivity(av.session);
    if (!activity) continue;
    const wsId = av.session.workstreamId;
    const cur = out.get(wsId);
    if (!cur || RANK[activity] > RANK[cur.activity]) {
      out.set(wsId, { activity, sessionId: av.session.id, since: av.session.startedAt });
    }
  }
  return out;
}

// A background tab never binds, so the store never holds its agent; the server's list does.
function serverPtyFold(agents: readonly PtyAgent[]): Map<string, WorkstreamActivity> {
  const out = new Map<string, WorkstreamActivity>();
  for (const a of agents) {
    if (a.hooked === false) continue;
    const activity = PTY_TO_ACTIVITY[toAgentState(a.state, false)];
    const cur = out.get(a.workstreamId);
    if (!cur || RANK[activity] > RANK[cur]) out.set(a.workstreamId, activity);
  }
  return out;
}

export function foldFleetItem(
  view: WorkstreamView,
  picked: Picked | undefined,
  pty: WorkstreamActivity,
  reg: { activity: FleetActivity; sessionId: string; since: number | null } | undefined,
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
      picked.window ?? (picked.model?.includes("[1m]") ? 1_000_000 : 200_000);
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

export function useFleet(views: WorkstreamView[]): Fleet {
  const wsIds = useMemo(() => views.map((v) => v.item.id), [views]);
  const storeKeys = useSessionStore(useShallow((s) => selectFleetKeys(s.sessions, wsIds)));
  const picked = useMemo(() => parsePicked(storeKeys), [storeKeys]);
  const ptyKeys = useMultiplexerStore(
    useShallow((s) => wsIds.map((id) => rollupForWorkstream(s.agents, id) ?? "idle")),
  );
  const registry = useActiveSessions(views.length > 0);
  const ptyAgents = usePtyAgents(null, views.length > 0);

  return useMemo(() => {
    const reg = registryFold(registry.data ?? []);
    const serverPty = serverPtyFold(ptyAgents.data ?? []);
    const live: FleetItem[] = [];
    const idle: WorkstreamView[] = [];
    let queued = 0;
    views.forEach((view, i) => {
      const local = PTY_TO_ACTIVITY[(ptyKeys[i] ?? "idle") as AgentState] ?? "idle";
      const remote = serverPty.get(view.item.id) ?? "idle";
      const pty = RANK[remote] > RANK[local] ? remote : local;
      const item = foldFleetItem(view, picked.get(view.item.id), pty, reg.get(view.item.id));
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
  }, [views, picked, ptyKeys, registry.data, ptyAgents.data]);
}
