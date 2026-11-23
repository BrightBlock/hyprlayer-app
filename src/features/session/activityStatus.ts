import { formatElapsed } from "@/lib/format";
import { basename } from "@/lib/path";

import { backgroundTasksFromItems, isBackgroundSpawn } from "./backgroundTask";
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
  Pick<
    SessionEntry,
    "workingSince" | "awaitingSince" | "turnAuthor" | "turnSince" | "lastTurn" | "waitSince" | "recap" | "goal"
  >;

/** Background agents and workflows still running when the turn ended. */
export type PendingWork = { agents: number; workflows: number };

export type LastTurn = { ms: number; endedAt: number; waiting?: PendingWork };

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

// Claude Code's turn-end verbs, one per turn.
const DONE_VERBS = ["Baked", "Brewed", "Churned", "Cogitated", "Cooked", "Crunched", "Sautéed", "Worked"];

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

function pendingWork(items: TranscriptItem[]): PendingWork & { shells: number } {
  const counts = { agents: 0, workflows: 0, shells: 0 };
  for (const task of backgroundTasksFromItems(items)) {
    if (task.kind === "agent") counts.agents++;
    else if (task.kind === "workflow") counts.workflows++;
    else counts.shells++;
  }
  return counts;
}

function waitingLabel({ agents, workflows }: PendingWork): string {
  const parts = [
    agents > 0 && plural(agents, "background agent", "background agents"),
    workflows > 0 && plural(workflows, "dynamic workflow", "dynamic workflows"),
  ].filter(Boolean);
  return `Waiting for ${parts.join(" and ")} to finish`;
}

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
  const newest = entry.items[entry.items.length - 1];
  if (newest?.kind === "relaunched" && newest.reason === "providerSettings") {
    return work(`Reloading ${ctx.harnessLabel} for the new provider settings…`, null);
  }
  if (other) return work(`${other} is working…`, null);
  if (entry.streaming) return streamActivity(entry, ctx.naming);
  const last = entry.lastTurn;
  if (!last) return null;
  // Held until the follow-up turn ends, not until the task does, so the line doesn't flash "done" in between.
  if (last.waiting && entry.waitSince != null) return quiet(waitingLabel(last.waiting), null);
  const at = new Date(last.endedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const { shells } = pendingWork(entry.items);
  const running = shells > 0 ? ` · ${plural(shells, "shell", "shells")} still running` : "";
  const recap = entry.recap?.endedAt === last.endedAt ? entry.recap.text : undefined;
  const verb = DONE_VERBS[last.endedAt % DONE_VERBS.length];
  return {
    label: `${verb} for ${formatElapsed(last.ms)}`,
    detail: `done ${at}${running}`,
    tone: "quiet",
    since: null,
    more: 0,
    recap,
  };
}

type TurnInput = SessionView & Partial<Pick<SessionEntry, "turnSince" | "lastTurn" | "waitSince" | "live">>;

// Harness spawn isn't part of a turn; approval waits are, and so is a reload mid-turn.
const inTurn = (e: TurnInput) => isBusy(e) && e.state !== "starting";

/**
 * Times a turn from its first streamed event to idle, across any approval or question it waits on.
 * A turn that leaves background agents running opens a wait; the turn that closes it is timed from the wait's start.
 * `closed` is a turn that opened and ended within one fold, which never shows this entry busy.
 */
export function nextTurn(
  prev: TurnInput | undefined,
  next: TurnInput,
  now: number,
  closed?: LastTurn | null,
): { turnSince: number | null; lastTurn: LastTurn | null; waitSince: number | null } {
  const prevSince = prev && isBusy(prev) ? (prev.turnSince ?? null) : null;
  const prevWait = prev?.waitSince ?? null;
  if (inTurn(next) || (prevSince !== null && next.state === "starting")) {
    return { turnSince: prevSince ?? now, lastTurn: null, waitSince: prevWait };
  }
  // A respawn or exit drops its background tasks.
  const waitSince = isTerminal(next.state) || next.state === "starting" ? null : prevWait;
  if (prevSince === null) {
    // A reload loses the wait's start; the follow-up is timed from here.
    if (waitSince === null && next.live && !isTerminal(next.state) && !isBusy(next)) {
      const { agents, workflows } = pendingWork(next.items);
      if (agents > 0 || workflows > 0) {
        return { turnSince: null, lastTurn: { ms: 0, endedAt: now, waiting: { agents, workflows } }, waitSince: now };
      }
    }
    return { turnSince: null, lastTurn: closed ?? prev?.lastTurn ?? null, waitSince };
  }
  const { agents, workflows } = pendingWork(next.items);
  if (agents > 0 || workflows > 0) {
    return {
      turnSince: null,
      lastTurn: { ms: now - prevSince, endedAt: now, waiting: { agents, workflows } },
      waitSince: waitSince ?? prevSince,
    };
  }
  return { turnSince: null, lastTurn: { ms: now - (waitSince ?? prevSince), endedAt: now }, waitSince: null };
}

type RecapInput = TurnInput & Pick<SessionEntry, "turnAuthor" | "goal">;

/** A turn that just ended here, on a session that can still show its recap. */
export function recapWanted(prevLast: LastTurn | null | undefined, last: LastTurn | null, next: RecapInput): boolean {
  if (!last || last === prevLast || last.waiting) return false;
  return next.turnAuthor !== "other" && !isTerminal(next.state) && next.goal?.status !== "running";
}
