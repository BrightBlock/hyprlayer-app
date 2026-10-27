import { formatElapsed } from "@/lib/format";
import { basename } from "@/lib/path";

import { isBackgroundSpawn } from "./backgroundTask";
import type { SessionEntry } from "./sessionStore";
import { runningSubagent } from "./subagent";
import {
  isHiddenTool,
  subagentType,
  toolDisplayName,
  toolFilePath,
  toolSummary,
  type ToolNaming,
} from "./toolDisplay";
import { hasPendingPermission, isBusy, isTerminal, type SessionView, type TranscriptItem } from "./transcript";

export type ActivityInput = SessionView &
  Pick<SessionEntry, "workingSince" | "awaitingSince" | "turnAuthor" | "turnSince" | "lastTurn" | "recap" | "goal">;

export type LastTurn = { ms: number; endedAt: number };

/** A model-written recap of the turn that ended at `endedAt`. */
export type TurnRecap = { text: string; endedAt: number };

export type Activity = {
  label: string;
  detail: string | null;
  tone: "work" | "wait" | "quiet";
  since: number | null;
  /** Other foreground calls still open alongside the one named. */
  more: number;
  /** The label is a verb and the detail its object, so no separator between them. */
  inline?: boolean;
  recap?: string;
};

export type ActivityContext = {
  harnessLabel: string;
  /** Set while another device owns the running turn, including its waits. */
  otherDevice: string | null;
  naming?: ToolNaming;
  started?: boolean;
};

type Tool = Extract<TranscriptItem, { kind: "tool" }>;

const VERBS: Record<string, string> = {
  Read: "Reading",
  Write: "Writing",
  Edit: "Editing",
  MultiEdit: "Editing",
  NotebookEdit: "Editing",
  Bash: "Running",
  BashOutput: "Reading output",
  Grep: "Searching",
  Glob: "Searching",
  WebFetch: "Fetching",
  WebSearch: "Searching the web",
  Skill: "Using skill",
  Workflow: "Running workflow",
};

const work = (label: string, since: number | null, detail: string | null = null, more = 0): Activity => ({
  label,
  detail,
  tone: "work",
  since,
  more,
});

const quiet = (label: string, since: number | null): Activity => ({ label, detail: null, tone: "quiet", since, more: 0 });

function toolActivity(tool: Tool, items: TranscriptItem[], since: number | null, more: number, naming?: ToolNaming) {
  if (tool.name === "Agent" || tool.name === "Task") {
    const sub = runningSubagent(items);
    if (sub?.toolUseId === tool.toolUseId) return work(sub.agent, since, sub.lastTool ?? sub.steps?.current ?? null, more);
    // A later spawn is the latest running subagent; this one still has its own type.
    const type = subagentType(tool.name, tool.input);
    if (type) return work(type, since, toolSummary(tool.name, tool.input, naming) || null, more);
  }
  const file = toolFilePath(tool.name, tool.input);
  const detail = (file ? basename(file) : toolSummary(tool.name, tool.input, naming)) || null;
  const verb = VERBS[tool.name] ?? `Running ${toolDisplayName(tool.name, naming)}`;
  return { ...work(verb, since, detail, more), inline: true };
}

// Reads only the current turn: everything after the last user message, exit or relaunch.
function streamActivity(entry: ActivityInput, naming?: ToolNaming): Activity {
  const { items } = entry;
  const since = entry.turnSince ?? entry.workingSince;
  const open: Tool[] = [];
  let last: TranscriptItem | null = null;
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item.kind === "user" || item.kind === "exit" || item.kind === "relaunched") break;
    if ("parentId" in item && item.parentId) continue;
    last ??= item;
    if (item.kind === "tool" && item.result === null && !isHiddenTool(item) && !isBackgroundSpawn(item)) {
      open.push(item);
    }
  }
  if (open.length > 0) return toolActivity(open[0], items, since, open.length - 1, naming);
  if (last?.kind === "thinking") return work("Thinking…", since);
  if (last?.kind === "assistant" && last.streaming) return work("Writing…", since);
  return work("Working…", since);
}

/** One line on what the agent is doing right now, for any harness; null when idle or ended. */
export function activityStatus(entry: ActivityInput | undefined, ctx: ActivityContext): Activity | null {
  if (!entry) return ctx.started ? work(`Connecting to ${ctx.harnessLabel}…`, null) : null;
  if (isTerminal(entry.state)) return null;
  if (entry.state === "starting") return work(`Connecting to ${ctx.harnessLabel}…`, null);
  const waitSince = entry.awaitingSince ?? null;
  const other = ctx.otherDevice;
  if (entry.pendingQuestion) {
    if (other) return quiet(`${other} is waiting for an answer`, waitSince);
    return { label: "Waiting for your answer", detail: null, tone: "wait", since: waitSince, more: 0 };
  }
  if (entry.state === "awaiting_permission" || hasPendingPermission(entry)) {
    if (other) return quiet(`${other} is waiting for approval`, waitSince);
    const pending = [...entry.items].reverse().find((it) => it.kind === "permission" && it.decision === null);
    const tool = pending?.kind === "permission" ? pending.toolName : (entry.pendingPermission?.toolName ?? null);
    return {
      label: "Waiting for approval",
      detail: tool && toolDisplayName(tool, ctx.naming),
      tone: "wait",
      since: waitSince,
      more: 0,
    };
  }
  if (other) return work(`${other} is working…`, null);
  if (entry.streaming) return streamActivity(entry, ctx.naming);
  const last = entry.lastTurn;
  if (!last) return null;
  const at = new Date(last.endedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const recap = entry.recap?.endedAt === last.endedAt ? entry.recap.text : undefined;
  return { label: `Worked for ${formatElapsed(last.ms)}`, detail: `done ${at}`, tone: "quiet", since: null, more: 0, recap };
}

type TurnInput = SessionView & Partial<Pick<SessionEntry, "turnSince" | "lastTurn">>;

// Harness spawn isn't part of a turn; approval waits are, and so is a reload mid-turn.
const inTurn = (e: TurnInput) => isBusy(e) && e.state !== "starting";

/** Times a turn from its first streamed event to idle, across any approval or question it waits on. */
export function nextTurn(
  prev: TurnInput | undefined,
  next: TurnInput,
  now: number,
): { turnSince: number | null; lastTurn: LastTurn | null } {
  const prevSince = prev && isBusy(prev) ? (prev.turnSince ?? null) : null;
  if (inTurn(next) || (prevSince !== null && next.state === "starting")) {
    return { turnSince: prevSince ?? now, lastTurn: null };
  }
  if (prevSince !== null) return { turnSince: null, lastTurn: { ms: now - prevSince, endedAt: now } };
  return { turnSince: null, lastTurn: prev?.lastTurn ?? null };
}

type RecapInput = TurnInput & Pick<SessionEntry, "turnAuthor" | "goal">;

/** A turn that just ended here, on a session that can still show its recap. */
export function recapWanted(prevLast: LastTurn | null | undefined, last: LastTurn | null, next: RecapInput): boolean {
  if (!last || last === prevLast) return false;
  return next.turnAuthor !== "other" && !isTerminal(next.state) && next.goal?.status !== "running";
}
