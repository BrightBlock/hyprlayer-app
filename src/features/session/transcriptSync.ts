import type { ClientRef, LogCursorDto, SessionEvent, SessionLogEntryDto, TurnKind } from "@/lib/bindings";
import { isAgentAuthor } from "@/lib/domain/agentControl";
import type { SyncPhase } from "@/lib/domain/sync";
import { reduceEvent, type SessionView } from "./transcript";

export type PendingTurn = { rev: number; kinds: TurnKind[] };
export type TurnAuthor = "self" | "other" | null;

export type SyncState = {
  localRev: number;
  /** Highest committed rev any source reported. */
  committedRev: number;
  /** What this view must reach: committed revs plus other clients' sends in flight. */
  remoteRev: number;
  /** `reset_rev` of the last full load; a higher one means a missed rewind. */
  resetRev: number;
  phase: SyncPhase;
  pendingTurns: PendingTurn[];
  authorDevice: string | null;
  workingDevice: string | null;
  /** Held items can apply: they include a commit or end where the turn waits. */
  flushable: boolean;
  draft: boolean;
  fetching: boolean;
  retry: { attempt: number; delayMs: number } | null;
  syncedWith: string | null;
  gone: boolean;
};

/** When the newest held turn arrived, and the remote rev the user dismissed the strip at. */
export type HeldView = { at: number | null; dismissedRev: number | null };

export type SyncInput =
  | { kind: "loaded"; rev: number; resetRev: number }
  | { kind: "applied"; rev: number }
  | {
      kind: "remote";
      rev: number;
      turns: PendingTurn[];
      device: string | null;
      working: boolean;
      flushable: boolean;
      attached: boolean;
      own: boolean;
    }
  | { kind: "polled"; rev: number; resetRev: number; inFlight: number; working: boolean }
  | { kind: "composer"; draft: boolean }
  | { kind: "pull" }
  | { kind: "fetched"; rev: number; resetRev: number; inFlight: number; remaining: number; device: string | null }
  | { kind: "fetchFailed" }
  | { kind: "stale"; rev: number; committedRev: number }
  | { kind: "reset" }
  | { kind: "gone" };

export type SyncEffect =
  | { kind: "pull" }
  | { kind: "reload" }
  | { kind: "recheck" }
  | { kind: "retryIn"; ms: number }
  | { kind: "flashSynced" };

export const RETRY_DELAYS_MS = [1_000, 2_000, 5_000, 15_000, 30_000] as const;

type Step = { state: SyncState; effects: SyncEffect[] };

export function transition(state: SyncState, input: SyncInput): Step {
  if (state.gone && input.kind !== "loaded") return { state, effects: [] };
  const effects: SyncEffect[] = [];
  switch (input.kind) {
    case "loaded": {
      const committedRev = Math.max(state.committedRev, input.rev);
      const next: SyncState = {
        ...state,
        localRev: input.rev,
        committedRev,
        remoteRev: Math.max(input.rev, committedRev),
        resetRev: input.resetRev,
        pendingTurns: state.pendingTurns.filter((t) => t.rev > input.rev),
        flushable: false,
        fetching: false,
        retry: null,
        gone: false,
      };
      return settle(next, effects);
    }

    case "applied": {
      const localRev = Math.max(state.localRev, input.rev);
      const next: SyncState = {
        ...state,
        localRev,
        committedRev: Math.max(state.committedRev, input.rev),
        remoteRev: Math.max(state.remoteRev, localRev),
        pendingTurns: state.pendingTurns.filter((t) => t.rev > localRev),
      };
      return settle(next, effects);
    }

    case "remote": {
      let next: SyncState = { ...state, committedRev: Math.max(state.committedRev, input.rev) };
      if (input.own) {
        next.workingDevice = null;
        // The view already shows its own turns; a detached one reads them without any cue.
        if (!input.attached && !state.fetching) effects.push({ kind: "pull" });
        recheckIfReleased(next, input.turns, effects);
        return { state: next, effects };
      }
      next.workingDevice = input.working ? (input.device ?? state.workingDevice) : null;
      next.flushable = input.flushable;
      if (!input.attached) {
        next.remoteRev = Math.max(state.remoteRev, input.rev);
        next.pendingTurns = mergeTurns(state.pendingTurns, input.turns, state.localRev);
        if (input.device && (input.flushable || input.turns.some((t) => t.rev > state.localRev))) {
          next.authorDevice = input.device;
        }
      }
      recheckIfReleased(next, input.turns, effects);
      next = settle(next, effects).state;
      return { state: next, effects };
    }

    case "polled": {
      const next = authoritative(state, input.rev, input.inFlight);
      if (!input.working) next.workingDevice = null;
      reloadIfReset(next, input.resetRev, effects);
      return settle(next, effects);
    }

    case "composer":
      return settle({ ...state, draft: input.draft }, effects);

    case "pull":
      if (state.fetching) return { state, effects };
      effects.push({ kind: "pull" });
      return settle({ ...state, fetching: true }, effects);

    case "fetched": {
      let next: SyncState = { ...state, fetching: false, retry: null, flushable: false };
      // Only a final page's rev matches the rows read.
      if (input.remaining === 0) {
        next.localRev = Math.max(state.localRev, input.rev);
        next = authoritative(next, input.rev, input.inFlight);
        next.pendingTurns = next.pendingTurns.filter((t) => t.rev > next.localRev);
        reloadIfReset(next, input.resetRev, effects);
      }
      if (!next.fetching && (input.remaining > 0 || next.committedRev > next.localRev)) {
        effects.push({ kind: "pull" });
        next.fetching = true;
      } else if (!next.fetching && next.localRev >= next.remoteRev) {
        const device = input.device ?? (state.pendingTurns.length > 0 ? state.authorDevice : null);
        if (device) {
          next.syncedWith = device;
          effects.push({ kind: "flashSynced" });
        }
      }
      return settle(next, effects);
    }

    case "fetchFailed": {
      const attempt = (state.retry?.attempt ?? 0) + 1;
      const delayMs = RETRY_DELAYS_MS[Math.min(attempt, RETRY_DELAYS_MS.length) - 1];
      effects.push({ kind: "retryIn", ms: delayMs });
      return settle({ ...state, fetching: false, retry: { attempt, delayMs } }, effects);
    }

    case "stale":
      return settle(
        {
          ...state,
          committedRev: Math.max(state.committedRev, input.committedRev),
          remoteRev: Math.max(state.remoteRev, input.rev),
        },
        effects,
      );

    case "reset":
      effects.push({ kind: "reload" });
      return settle({ ...state, fetching: true }, effects);

    case "gone":
      return {
        state: {
          ...state,
          gone: true,
          phase: "inSync",
          remoteRev: state.localRev,
          pendingTurns: [],
          flushable: false,
          fetching: false,
          retry: null,
          workingDevice: null,
        },
        effects,
      };
  }
}

// Another device's turns only ever apply on a pull the user asks for.
function settle(state: SyncState, effects: SyncEffect[]): Step {
  // Held items with no new revision (a turn end, a prompt) still wait on a pull.
  const pending = state.remoteRev > state.localRev || state.flushable;
  let phase: SyncPhase;
  if (state.fetching) phase = "syncing";
  else if (state.draft && pending) phase = "guard";
  else if (pending) phase = "behind";
  else phase = "inSync";
  return { state: { ...state, phase }, effects };
}

function authoritative(state: SyncState, rev: number, inFlight: number): SyncState {
  const committedRev = Math.max(state.committedRev, rev);
  return { ...state, committedRev, remoteRev: Math.max(state.localRev, committedRev, rev + inFlight) };
}

function reloadIfReset(state: SyncState, resetRev: number, effects: SyncEffect[]): void {
  if (resetRev <= state.resetRev) return;
  effects.push({ kind: "reload" });
  state.fetching = true;
}

// A turnless revision while a send is counted means that send ended without a commit.
function recheckIfReleased(state: SyncState, turns: PendingTurn[], effects: SyncEffect[]): void {
  if (turns.length === 0 && state.remoteRev > state.committedRev) effects.push({ kind: "recheck" });
}

// Held summaries re-report the whole buffer, so a rev seen again replaces its entry.
function mergeTurns(current: PendingTurn[], incoming: PendingTurn[], localRev: number): PendingTurn[] {
  const byRev = new Map(current.map((t) => [t.rev, t]));
  for (const turn of incoming) if (turn.rev > localRev) byRev.set(turn.rev, turn);
  return [...byRev.values()].sort((a, b) => a.rev - b.rev);
}

export function canSend(state: SyncState): boolean {
  return state.phase === "inSync";
}

export function deviceLabel(state: SyncState): string {
  return state.authorDevice || state.workingDevice || "another device";
}

export type Arrival = {
  seq: number | null;
  /** With `seq`, the row's resume cursor; a server before 0.8.21 sends none. */
  createdAt?: number | null;
  rev: number | null;
  author: ClientRef | null;
  event: SessionEvent;
  receivedAt: number;
};

/** A stream item as routed: `receivedAt` is stamped only on the items that get held. */
export type Routed = Omit<Arrival, "receivedAt">;

export type RouteContext = { selfId: string | null; appliedSeq: number; heldCount: number; ownTurnStreaming: boolean };

const COMMIT_KINDS: ReadonlySet<SessionEvent["kind"]> = new Set(["userMessage", "assistantText", "toolResult"]);
const WAIT_KINDS: ReadonlySet<SessionEvent["kind"]> = new Set(["turnDone", "exited", "permissionAsk", "questionAsk"]);
const TURN_END_KINDS: ReadonlySet<SessionEvent["kind"]> = new Set(["turnDone", "exited"]);

// An agent's turn routes like the app's own unattributed ones: applied live, never held behind a pull.
const isForeign = (author: ClientRef | null, selfId: string | null) =>
  !!author && !isAgentAuthor(author) && author.clientId !== selfId;
const isOwn = (author: ClientRef | null, selfId: string | null) =>
  !!author && !isAgentAuthor(author) && author.clientId === selfId;

export function route(item: Routed, ctx: RouteContext): "apply" | "hold" | "drop" | "skip" | "flush" | "reset" {
  const { event } = item;
  if (event.kind === "rewound") return "reset";
  if (item.seq !== null && item.seq <= ctx.appliedSeq) return "skip";
  if (ctx.selfId === null) return "apply";
  const foreign = isForeign(item.author, ctx.selfId);
  if (foreign && event.kind === "partialText") return "drop";
  if (ctx.heldCount > 0) return isOwn(item.author, ctx.selfId) ? "flush" : "hold";
  if (foreign && event.kind === "userMessage" && ctx.ownTurnStreaming) return "apply";
  if (foreign) return "hold";
  return "apply";
}

export function turnAuthorAfter(
  current: TurnAuthor,
  item: Routed,
  selfId: string | null,
  wasStreaming: boolean,
): TurnAuthor {
  const author: TurnAuthor =
    item.author && !isAgentAuthor(item.author) ? (isOwn(item.author, selfId) ? "self" : "other") : null;
  // A user message opens a turn only when none is running; mid-turn it waits for the next one.
  if (item.event.kind === "userMessage") return wasStreaming ? current : author;
  return author ?? current;
}

const turnKind = (kind: SessionEvent["kind"]): TurnKind | null => {
  switch (kind) {
    case "userMessage":
      return "user";
    case "assistantText":
    case "thinking":
      return "assistant";
    case "toolUse":
    case "toolResult":
      return "tool";
    default:
      return null;
  }
};

export function heldSummary(held: Arrival[]): {
  rev: number | null;
  turns: PendingTurn[];
  device: string | null;
  working: boolean;
  flushable: boolean;
} {
  const turns: PendingTurn[] = [];
  // Thinking and tool-use rows carry the previous rev and ride with the next commit.
  let riders: TurnKind[] = [];
  let rev: number | null = null;
  let device: string | null = null;
  for (const item of held) {
    if (item.author) device = item.author.device;
    const kind = turnKind(item.event.kind);
    if (kind === null) continue;
    riders.push(kind);
    if (COMMIT_KINDS.has(item.event.kind) && item.rev !== null) {
      turns.push({ rev: item.rev, kinds: [...new Set(riders)] });
      rev = item.rev;
      riders = [];
    }
  }
  const last = held[held.length - 1]?.event.kind;
  return {
    rev,
    turns,
    device,
    working: last !== undefined && !TURN_END_KINDS.has(last),
    flushable: turns.length > 0 || (last !== undefined && WAIT_KINDS.has(last)),
  };
}

export type Merged = {
  view: SessionView;
  appliedSeq: number;
  cursor: LogCursorDto;
  firstNewSeq: number | null;
  revisions: number;
  reset: boolean;
};

export function mergeEntries(
  view: SessionView,
  appliedSeq: number,
  page: { entries: SessionLogEntryDto[]; nextCursor: LogCursorDto },
): Merged {
  const unchanged = { view, appliedSeq, cursor: page.nextCursor, firstNewSeq: null, revisions: 0 };
  // A rewind reuses seqs, so its boundary is found whatever its seq.
  if (page.entries.some((e) => e.event.kind === "rewound")) return { ...unchanged, reset: true };
  const fresh = page.entries
    .map((entry) => ({ entry, seq: entry.seq ?? -1 }))
    .filter(({ seq }) => seq > appliedSeq)
    .sort((a, b) => a.seq - b.seq);
  if (fresh.length === 0) return { ...unchanged, reset: false };

  let next = view;
  let revisions = 0;
  for (const { entry, seq } of fresh) {
    const before = next;
    next = reduceEvent(next, entry.event, seq);
    if (next === before) continue;
    if (COMMIT_KINDS.has(entry.event.kind)) revisions += 1;
    const last = next.items[next.items.length - 1];
    if (entry.event.kind === "userMessage" && last?.kind === "user" && last.seq === seq && last.createdAt === null) {
      next = { ...next, items: [...next.items.slice(0, -1), { ...last, createdAt: entry.createdAt }] };
    }
  }
  return {
    view: next,
    appliedSeq: fresh[fresh.length - 1].seq,
    cursor: page.nextCursor,
    firstNewSeq: fresh[0].seq,
    revisions,
    reset: false,
  };
}
