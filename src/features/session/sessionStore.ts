
import { useMemo } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";
import { useShallow } from "zustand/shallow";

import {
  commands,
  events,
  type Bootstrap,
  type ClientRef,
  type ClaudeProbe,
  type Harness,
  type LogCursorDto,
  type PermissionMode,
  type Stage,
  type Session,
  type SessionEvent,
  type SessionLogEntryDto,
  type SessionReset,
  type SessionRevised,
  type SessionState,
  type SessionStreamItemDto,
  type SessionTurnsDto,
} from "@/lib/bindings";
import type { WorkstreamActivity } from "@/lib/domain/activity";
import type { EventBridge } from "@/lib/events";
import { isAppError, isStaleTranscript, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";
import { invalidateWorkstreamPr, invalidateWorktreeGit } from "@/lib/queries";
import { findWorkstreamForNotify, notifyNeedsAttention } from "@/lib/notifications";
import { useToast } from "@/lib/toast";
import { currentAppLocation } from "@/lib/appNav";
import type { DocType } from "@/lib/domain/docs";
import { docTitle, sessionDocs } from "./toolDisplay";
import { artifactsFromItems, type SessionArtifact } from "./artifacts";
import { forgetWorkstreamDocs, useWrittenDocs } from "./writtenDocs";
import {
  backgroundTasksFromItems,
  type BackgroundTask,
  type BackgroundTaskStatus,
} from "./backgroundTask";
import { planTasksFromItems, todoPhaseLabel, type PlanTask, type PlanTaskStatus } from "./planTasks";
import { type Attachment } from "./attachments";
import { REVIEW_PROMPT } from "./reviewCardSlot";
import { permissionModeOption } from "@/lib/domain/permissions";
import {
  bodyHasVars,
  effectiveBuiltinBody,
  substitutePromptVars,
} from "@/features/prompts/prompts";
import {
  actionAgentDefaults,
  actionLaunchArgs,
  newChatLaunchArgs,
  type ActionAgentDefaults,
} from "@/lib/domain/harnessing";
import { claudeModels, type ModelOption } from "@/lib/domain/models";
import {
  bindSessionToShell,
  focusedPaneId,
  openCommitTab,
  openImplementTab,
  openPromptTab,
  openSessionTab,
  openShipTab,
  paneShells,
  sessionPermissionMode,
  setSessionPermissionMode,
  useWorkstreamTabsStore,
  visibleShells,
  type ShellTab,
} from "./workstreamTabs";
import {
  addUserMessage,
  cancelPendingPrompts,
  closeOpenAssistant,
  hasPendingPermission,
  initialSessionView,
  isBusy,
  isTerminal,
  isWorking,
  parkedState,
  reduceEvent,
  resolvePermission,
  resolveQuestion,
  restoredView,
  viewToText,
  type SessionView,
  type TranscriptItem,
} from "./transcript";
import {
  canSend,
  heldSummary,
  mergeEntries,
  route,
  turnAuthorAfter,
  type Arrival,
  type RouteContext,
  type Routed,
  type SyncState,
  type TurnAuthor,
} from "./transcriptSync";
import {
  forgetSync,
  rev,
  scheduleWatch,
  selfClientId,
  setDivider,
  setSyncHandlers,
  syncInput,
  syncOf,
  syncSupport,
} from "./syncStore";

export type SessionEntry = SessionView & {
  workstreamId: string;
  stage: string;
  autoAllow: string[];
  live: boolean;
  permissionMode: PermissionMode | null;
  /** The harness's own plan mode, layered over `permissionMode` (codex, OpenCode). */
  planning?: boolean;
  reloading?: boolean;
  drifted?: boolean;
  queued: QueuedMessage[];
  workingSince: number | null;
  /** Optional so setEntry's callers needn't supply it; derived on every write. */
  awaitingSince?: number | null;
  taskStartedAt: Record<string, number>;
  /** Highest log seq in the view; a rewind's reload lowers it, since the rewind reuses seqs. */
  appliedSeq?: number;
  /** Where a catch-up read resumes: the last fold's or page's final raw row. */
  cursor?: LogCursorDto | null;
  turnAuthor?: TurnAuthor;
};

export type QueuedMessage = {
  text: string;
  attachments: Attachment[];
  promptName: string | null;
  /** Runs once the queued text actually reaches the harness. */
  onSent?: () => void;
};

/** "dropped": the session left the store before anything was sent or queued. */
export type SendOutcome = "sent" | "queued" | "dropped";

type State = { sessions: Record<string, SessionEntry> };

export const useSessionStore = create<State>(() => ({ sessions: {} }));

const channels = new Map<string, Channel<SessionStreamItemDto>>();

const restoring = new Set<string>();

// Another device's items, waiting for the user to pull them.
const held = new Map<string, Arrival[]>();

function entryOf(sessionId: string): SessionEntry | undefined {
  return useSessionStore.getState().sessions[sessionId];
}

function setEntry(sessionId: string, next: SessionEntry): void {
  const added = !useSessionStore.getState().sessions[sessionId];
  useSessionStore.setState((s) => {
    const prev = s.sessions[sessionId];
    const workingSince = nextWorkingSince(prev, next);
    const awaitingSince = nextAwaitingSince(prev, next);
    const taskStartedAt = nextTaskStartedAt(prev, next);
    return {
      sessions: { ...s.sessions, [sessionId]: { ...next, workingSince, awaitingSince, taskStartedAt } },
    };
  });
  if (isWorking(next)) lastWorkingAt.set(next.workstreamId, Date.now());
  if (added) scheduleWatch();
}

function forgetSession(sessionId: string): void {
  channels.delete(sessionId);
  held.delete(sessionId);
  forgetSync(sessionId);
}

const lastWorkingAt = new Map<string, number>();

const ATTRIBUTION_GRACE_MS = 30_000;

export function activeWorkstreamIds(): string[] {
  const { sessions } = useSessionStore.getState();
  const cutoff = Date.now() - ATTRIBUTION_GRACE_MS;
  const ids = new Set<string>();
  for (const entry of Object.values(sessions)) {
    if (isWorking(entry)) ids.add(entry.workstreamId);
  }
  for (const [workstreamId, at] of lastWorkingAt) {
    if (at >= cutoff) ids.add(workstreamId);
  }
  return [...ids];
}

export function removeSessionsForWorkstream(workstreamId: string): void {
  forgetWorkstreamDocs(workstreamId);
  lastWorkingAt.delete(workstreamId);

  const { sessions } = useSessionStore.getState();
  const ids = Object.keys(sessions).filter((id) => sessions[id]?.workstreamId === workstreamId);
  if (ids.length === 0) return;
  for (const id of ids) forgetSession(id);
  useSessionStore.setState((s) => {
    const next = { ...s.sessions };
    for (const id of ids) delete next[id];
    return { sessions: next };
  });
  scheduleWatch();
}

export function dropSession(sessionId: string): void {
  forgetSession(sessionId);
  useSessionStore.setState((s) => {
    if (!s.sessions[sessionId]) return s;
    const next = { ...s.sessions };
    delete next[sessionId];
    return { sessions: next };
  });
  scheduleWatch();
}

function nextTaskStartedAt(
  prev: SessionEntry | undefined,
  next: SessionEntry,
): Record<string, number> {
  const live = backgroundTasksFromItems(next.items);
  const out: Record<string, number> = {};
  for (const t of live) {
    out[t.toolUseId] = prev?.taskStartedAt?.[t.toolUseId] ?? Date.now();
  }
  return out;
}

function nextWorkingSince(prev: SessionEntry | undefined, next: SessionEntry): number | null {
  if (!isWorking(next)) return null;
  const wasWorking = prev ? isWorking(prev) : false;
  return wasWorking ? (prev?.workingSince ?? Date.now()) : Date.now();
}

function nextAwaitingSince(prev: SessionEntry | undefined, next: SessionEntry): number | null {
  if (!isAwaiting(next)) return null;
  return prev && isAwaiting(prev) ? (prev.awaitingSince ?? Date.now()) : Date.now();
}

// Events can arrive before the command returns the session id.
function bufferedChannel(): {
  channel: Channel<SessionStreamItemDto>;
  activate: (sessionId: string) => void;
} {
  const buffer: SessionStreamItemDto[] = [];
  let boundId: string | null = null;
  const channel = new Channel<SessionStreamItemDto>();
  channel.onmessage = (item) => {
    if (retired.has(channel)) return;
    if (boundId) dispatch(boundId, item);
    else buffer.push(item);
  };
  return {
    channel,
    activate(sessionId) {
      boundId = sessionId;
      for (const item of buffer) dispatch(sessionId, item);
      buffer.length = 0;
    },
  };
}

// Reused per item: `route` reads it and keeps nothing.
const routeContext: RouteContext = { selfId: null, appliedSeq: -1, heldCount: 0, ownTurnStreaming: false };

function dispatch(sessionId: string, item: SessionStreamItemDto): void {
  // A server without sync sends no rev: everything applies as it arrives.
  if (item.rev === null) {
    applyEvent(sessionId, item.event, null);
    return;
  }
  const current = entryOf(sessionId);
  if (!current) return;
  routeContext.selfId = selfClientId();
  routeContext.appliedSeq = current.appliedSeq ?? -1;
  routeContext.heldCount = held.get(sessionId)?.length ?? 0;
  routeContext.ownTurnStreaming = current.streaming && current.turnAuthor === "self";
  switch (route(item, routeContext)) {
    case "reset":
      if (rev(item.rev) > (syncOf(sessionId)?.resetRev ?? 0)) syncInput(sessionId, { kind: "reset" });
      return;
    case "apply":
      applySynced(sessionId, item);
      return;
    case "hold":
      holdItem(sessionId, { ...item, receivedAt: Date.now() });
      return;
    case "flush":
      applyHeld(sessionId);
      applySynced(sessionId, item);
      return;
    case "skip":
    case "drop":
      return;
  }
}

function applySynced(sessionId: string, item: Routed): void {
  if (!entryOf(sessionId)) return;
  applyEvent(sessionId, item.event, item);
  const applied = rev(item.rev);
  const sync = syncOf(sessionId);
  // Most items repeat the rev already applied; partial text arrives per token.
  if (!sync || applied > sync.localRev) syncInput(sessionId, { kind: "applied", rev: applied });
}

function holdItem(sessionId: string, item: Arrival): void {
  const queue = held.get(sessionId);
  if (queue) queue.push(item);
  else held.set(sessionId, [item]);
  reportHeld(sessionId);
  // A queued message waits on this turn's end; sending it is the user's own action.
  const ended = item.event.kind === "turnDone" || item.event.kind === "exited";
  if (ended && (entryOf(sessionId)?.queued.length ?? 0) > 0) void pullNow(sessionId);
}

function reportHeld(sessionId: string): void {
  const queue = held.get(sessionId);
  if (!queue) return;
  const summary = heldSummary(queue);
  syncInput(sessionId, {
    kind: "remote",
    rev: summary.rev ?? syncOf(sessionId)?.localRev ?? 0,
    turns: summary.turns,
    device: summary.device,
    working: summary.working,
    flushable: summary.flushable,
    attached: false,
    own: false,
  });
}

// In arrival order, through the same path a live item takes.
function applyHeld(sessionId: string): Arrival[] {
  const queue = held.get(sessionId);
  if (!queue) return [];
  held.delete(sessionId);
  for (const item of queue) {
    applySynced(sessionId, item);
    if (item.event.kind !== "userMessage") continue;
    // A live user message has no createdAt; when it arrived is the closest stand-in.
    const entry = entryOf(sessionId);
    const last = entry?.items[entry.items.length - 1];
    if (entry && last?.kind === "user" && last.seq === item.seq && last.createdAt === null) {
      setEntry(sessionId, { ...entry, items: [...entry.items.slice(0, -1), { ...last, createdAt: item.receivedAt }] });
    }
  }
  return queue;
}

function trimHeld(sessionId: string, appliedSeq: number): void {
  const queue = held.get(sessionId);
  if (!queue) return;
  const kept = queue.filter((item) => item.seq === null || item.seq > appliedSeq);
  if (kept.length === 0) held.delete(sessionId);
  else if (kept.length < queue.length) held.set(sessionId, kept);
}

function applyEvent(sessionId: string, event: SessionEvent, synced: Routed | null): void {
  const current = entryOf(sessionId);
  if (!current) return;
  const seq = synced?.seq ?? null;

  // A mode restart must not mark the transcript terminal.
  if (event.kind === "exited" && current.reloading) {
    channels.delete(sessionId);
    const cleared = cancelPendingPrompts(current);
    setEntry(sessionId, {
      ...current,
      ...cleared,
      items: closeOpenAssistant(cleared.items),
      streaming: false,
      live: false,
      reloading: false,
      planning: false,
      state: "idle",
      appliedSeq: Math.max(current.appliedSeq ?? -1, seq ?? -1),
    });
    void queryClient.invalidateQueries({ queryKey: ["workstream", current.workstreamId, "diff"] });
    return;
  }

  const wasNeedsYou = classifyEntry(current) === "needs-you";
  const nextEntry: SessionEntry = { ...current, ...reduceEvent(current, event, seq), ...modeFields(event) };
  if (synced) {
    if (seq !== null && seq > (nextEntry.appliedSeq ?? -1)) nextEntry.appliedSeq = seq;
    nextEntry.turnAuthor = turnAuthorAfter(current.turnAuthor ?? null, synced, selfClientId(), current.streaming);
  }
  setEntry(sessionId, nextEntry);
  if (event.kind === "permissionModeChanged") {
    setSessionPermissionMode(current.workstreamId, sessionId, event.mode);
  }

  const isNeedsYou = classifyEntry(nextEntry) === "needs-you";
  const autoApproved =
    event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName);
  // Another device is driving this turn; its user answers the prompt there.
  if (!wasNeedsYou && isNeedsYou && !autoApproved && nextEntry.turnAuthor !== "other") {
    void maybeNotifyTransition(sessionId, current.workstreamId, current.stage);
  }

  if (event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName)) {
    void respondPermission(sessionId, event.requestId, true, null);
  }
  if (event.kind === "turnDone") {
    invalidateWorktreeGit(current.workstreamId);
    invalidateWorkstreamPr(current.workstreamId);
    flushQueuedMessage(sessionId);
  }
  if (event.kind === "exited") endStream(sessionId, current.workstreamId);
}

// The session exited: its stream is dead, whether the exit came on it or in a pulled page.
function endStream(sessionId: string, workstreamId: string): void {
  invalidateWorktreeGit(workstreamId);
  invalidateWorkstreamPr(workstreamId);
  channels.delete(sessionId);
  const latest = entryOf(sessionId);
  if (latest) setEntry(sessionId, { ...latest, live: false });
  void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
}

// Live-only: the server never persists mode events, so a replayed log has none.
function modeFields(event: SessionEvent): Partial<SessionEntry> {
  switch (event.kind) {
    case "permissionModeChanged":
      return { permissionMode: event.mode };
    case "planModeChanged":
      return { planning: event.active };
    case "relaunched":
    case "exited":
      return { planning: false };
    default:
      return {};
  }
}

async function maybeNotifyTransition(
  sessionId: string,
  workstreamId: string,
  stage: string,
): Promise<void> {
  try {
    if (await isSessionVisible(sessionId, workstreamId)) return;
    const ws = findWorkstreamForNotify(workstreamId);
    if (!ws) return;
    await notifyNeedsAttention({
      workstreamId,
      projectPath: ws.projectPath,
      workstreamTitle: ws.title,
      stage,
    });
  } catch {
  }
}

async function isSessionVisible(sessionId: string, workstreamId: string): Promise<boolean> {
  if (!(await getCurrentWindow().isFocused())) return false;
  const loc = currentAppLocation();
  if (!loc || !loc.pathname.startsWith("/projects/")) return false;
  const search = loc.search as { item?: string };
  if (search.item !== workstreamId) return false;
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  if (!tabs) return false;
  return visibleShells(tabs).some((s) => s.sessionId === sessionId);
}

export async function startSession(args: {
  workstreamId: string;
  stage: Stage;
  input: string;
  attachments?: Attachment[];
  harness?: Harness | null;
  model?: string | null;
  effort?: string | null;
  permissionMode?: PermissionMode | null;
  promptName?: string | null;
}): Promise<Session> {
  const { channel, activate } = bufferedChannel();
  const attachments = args.attachments ?? [];
  const clientMessageId = crypto.randomUUID();

  const { session, prompt } = await unwrap(
    commands.startStageSession(
      args.workstreamId,
      args.stage,
      args.input,
      attachments.map((a) => a.path),
      {
        harness: args.harness ?? null,
        model: args.model ?? null,
        effort: args.effort ?? null,
        permissionMode: args.permissionMode ?? null,
        promptName: args.promptName ?? null,
        clientMessageId,
      },
      channel,
    ),
  );

  channels.set(session.id, channel);

  let view = initialSessionView("starting");
  view = { ...view, model: session.model, claudeSessionId: session.claudeSessionId };
  view = addUserMessage(
    view,
    prompt,
    args.stage,
    attachments,
    Date.now(),
    args.promptName ?? null,
    clientMessageId,
  );
  setEntry(session.id, {
    ...view,
    workstreamId: args.workstreamId,
    stage: args.stage,
    autoAllow: [],
    live: true,
    permissionMode: args.permissionMode ?? null,
    workingSince: null,
    taskStartedAt: {},
    queued: [],
    turnAuthor: "self",
  });

  activate(session.id);
  return session;
}

// "Auto" (null) never disagrees; only an explicit tab mode can be stale.
function adoptRunningMode(sessionId: string, running: PermissionMode | null | undefined): void {
  const entry = entryOf(sessionId);
  if (!running || !entry || entry.permissionMode === null || entry.permissionMode === running) return;
  setEntry(sessionId, { ...entry, permissionMode: running });
  setSessionPermissionMode(entry.workstreamId, sessionId, running);
}

type LogRead = {
  entries: SessionLogEntryDto[];
  cursor: LogCursorDto;
  /** Set when read through `list_session_turns`; they match the rows read. */
  revs: { rev: number; resetRev: number } | null;
};

const FOLD_PAGE = 2000;
const PULL_PAGE = 200;
const FOLD_RESTARTS = 3;

// With sync, a fold pages from the start so it carries the rev it read up to (R6).
async function readLog(sessionId: string): Promise<LogRead> {
  if (!(await syncSupport())) {
    const entries = await unwrap(commands.getSessionLog(sessionId));
    const lastRow = entries[entries.length - 1];
    const cursor: LogCursorDto = lastRow
      ? { kind: "after", seq: lastRow.seq, createdAt: lastRow.createdAt }
      : { kind: "empty" };
    return { entries, cursor, revs: null };
  }
  for (let attempt = 1; attempt <= FOLD_RESTARTS; attempt += 1) {
    try {
      const read = await readTurns(sessionId);
      if (read) return read;
    } catch (e) {
      if (attempt === FOLD_RESTARTS || !isSessionEpochAdvanced(e)) throw e;
    }
  }
  throw new Error(`${sessionId} kept rewinding while its log was read`);
}

// Null when a rewind between pages rewrote rows already read.
async function readTurns(sessionId: string): Promise<LogRead | null> {
  const entries: SessionLogEntryDto[] = [];
  let cursor: LogCursorDto = { kind: "empty" };
  let resetRev: number | null = null;
  for (;;) {
    const page: SessionTurnsDto = await unwrap(commands.listSessionTurns(sessionId, cursor, FOLD_PAGE));
    if (resetRev !== null && rev(page.resetRev) !== resetRev) return null;
    resetRev = rev(page.resetRev);
    for (const entry of page.entries) entries.push(entry);
    cursor = page.nextCursor;
    if (page.remaining === 0) return { entries, cursor, revs: { rev: rev(page.rev), resetRev } };
  }
}

// Who opened the last turn among rows after `afterSeq`, and the last other device among them.
function turnOpeners(
  entries: SessionLogEntryDto[],
  afterSeq: number,
): { turnAuthor: TurnAuthor | undefined; device: string | null } {
  const selfId = selfClientId();
  let turnAuthor: TurnAuthor | undefined;
  let device: string | null = null;
  for (const entry of entries) {
    if (entry.event.kind !== "userMessage" || (entry.seq ?? -1) <= afterSeq) continue;
    const author = entry.event.author ?? null;
    turnAuthor = !author ? null : author.clientId === selfId ? "self" : "other";
    if (author && author.clientId !== selfId) device = author.device;
  }
  return { turnAuthor, device };
}

function foldLog(
  read: LogRead,
  previousItems: TranscriptItem[] = [],
): { view: SessionView; cursor: LogCursorDto; appliedSeq: number; turnAuthor: TurnAuthor } {
  // The log stores no attachments; recover them from the optimistic item this
  // device already rendered, matched by the nonce both share.
  const attachmentsById = new Map(
    previousItems
      .filter((it): it is Extract<TranscriptItem, { kind: "user" }> => it.kind === "user" && !!it.clientMessageId)
      .map((it) => [it.clientMessageId as string, it.attachments]),
  );
  let view = initialSessionView("starting");
  let appliedSeq = -1;
  for (const stored of read.entries) {
    if (stored.seq !== null && stored.seq > appliedSeq) appliedSeq = stored.seq;
    try {
      view = reduceEvent(view, stored.event, stored.seq);
      const last = view.items[view.items.length - 1];
      if (stored.kind === "userMessage" && last?.kind === "user") {
        const createdAt = last.createdAt === null ? stored.createdAt : last.createdAt;
        const attachments = (last.clientMessageId && attachmentsById.get(last.clientMessageId)) || last.attachments;
        if (createdAt !== last.createdAt || attachments !== last.attachments) {
          view = { ...view, items: [...view.items.slice(0, -1), { ...last, createdAt, attachments }] };
        }
      }
    } catch (e) {
      log.warn("session", `folding a ${stored.kind} event`, e);
    }
  }
  const turnAuthor = turnOpeners(read.entries, -1).turnAuthor ?? null;
  return { view, cursor: read.cursor, appliedSeq, turnAuthor };
}

// A full fold replaces everything held: those rows are in it, or the reload discarded them.
function foldLoaded(sessionId: string, read: LogRead): void {
  held.delete(sessionId);
  if (read.revs) syncInput(sessionId, { kind: "loaded", ...read.revs });
}

// Re-reads and re-folds the log into the existing entry, in place (no
// dropSession, so a mounted SessionChat's restore effect does not refire).
// Returns the cursor the fold ended at, or null if the entry disappeared
// while the log was in flight.
async function foldAndWriteEntry(sessionId: string): Promise<LogCursorDto | null> {
  const read = await readLog(sessionId);
  const current = entryOf(sessionId);
  if (!current) return null;
  const { view, ...position } = foldLog(read, current.items);
  setEntry(sessionId, { ...current, ...view, ...position, live: true, state: "starting" });
  foldLoaded(sessionId, read);
  return position.cursor;
}

function isSessionEpochAdvanced(e: unknown): boolean {
  return isAppError(e) && e.code === "session_epoch_advanced";
}

const resuming = new Map<string, Promise<void>>();

// Single-flight: two concurrent resumes would each attach a stream and every
// event would land in the transcript twice.
export function resumeSession(
  sessionId: string,
  opts: { cursor?: LogCursorDto; flush?: boolean } = {},
): Promise<void> {
  const inFlight = resuming.get(sessionId);
  if (inFlight) return inFlight;
  const run = resumeOnce(sessionId, opts).finally(() => resuming.delete(sessionId));
  resuming.set(sessionId, run);
  return run;
}

async function resumeOnce(
  sessionId: string,
  opts: { cursor?: LogCursorDto; flush?: boolean },
): Promise<void> {
  if (!entryOf(sessionId) || channels.has(sessionId)) return;

  // Every channel-less resume folds the log first: the cursor it produces is
  // the only valid one, since live events carry no seq.
  let cursor: LogCursorDto | null;
  if (opts.cursor) {
    cursor = opts.cursor;
    const current = entryOf(sessionId);
    if (current) setEntry(sessionId, { ...current, live: true, state: "starting" });
  } else {
    cursor = await foldAndWriteEntry(sessionId);
    if (!cursor || !entryOf(sessionId)) return;
  }

  let permissionMode: PermissionMode | null = null;
  for (let attempt = 1; ; attempt += 1) {
    const entry = entryOf(sessionId);
    if (!entry) return;
    // A Tauri Channel serves one invoke, so a retry needs a fresh one.
    const { channel, activate } = bufferedChannel();
    channels.set(sessionId, channel);
    try {
      const result = await unwrap(
        commands.resumeSession(sessionId, null, null, entry.permissionMode, cursor, channel),
      );
      permissionMode = result.permissionMode;
      activate(sessionId);
      break;
    } catch (e) {
      channels.delete(sessionId);
      let failure = e;
      if (attempt === 1 && isSessionEpochAdvanced(e)) {
        // A failed re-fold must still reach the rollback, or the tab stays live with no channel.
        try {
          cursor = await foldAndWriteEntry(sessionId);
          if (!cursor || !entryOf(sessionId)) return;
          continue;
        } catch (refold) {
          failure = refold;
        }
      }
      const latest = entryOf(sessionId);
      if (latest) setEntry(sessionId, { ...latest, live: false, state: "idle" });
      throw failure;
    }
  }

  adoptRunningMode(sessionId, permissionMode);
  const resumed = entryOf(sessionId);
  if (resumed?.drifted) setEntry(sessionId, { ...resumed, drifted: false });

  // A quiet Claude resume emits no init event.
  const after = entryOf(sessionId);
  if (after && !isTerminal(after.state) && !after.streaming) {
    setEntry(sessionId, {
      ...after,
      items: closeOpenAssistant(after.items),
      streaming: false,
      state: parkedState(after),
    });
    if (opts.flush !== false) flushQueuedMessage(sessionId);
  }
}

async function restartSession(
  sessionId: string,
  mode: PermissionMode | null,
  model: string | null = null,
  effort: string | null = null,
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;

  channels.delete(sessionId);
  setEntry(sessionId, {
    ...current,
    permissionMode: mode,
    autoAllow: [],
    reloading: true,
    live: false,
    state: "starting",
  });

  const { channel, activate } = bufferedChannel();
  try {
    await unwrap(commands.restartSession(sessionId, model, effort, mode, channel));
  } catch (e) {
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, reloading: false, live: false, state: "idle" });
    throw e;
  }

  channels.set(sessionId, channel);
  activate(sessionId);
  const after = entryOf(sessionId);
  if (after) {
    const cleared = cancelPendingPrompts(after);
    setEntry(sessionId, {
      ...after,
      ...cleared,
      items: closeOpenAssistant(cleared.items),
      live: true,
      reloading: false,
      streaming: false,
      state: "idle",
    });
    flushQueuedMessage(sessionId);
  }
}

function userTurnOrdinal(items: SessionEntry["items"], itemId: string): number {
  let n = 0;
  for (const item of items) {
    if (item.kind !== "user") continue;
    n += 1;
    if (item.id === itemId) return n;
  }
  return 0;
}

export function isFirstUserTurn(sessionId: string, itemId: string): boolean {
  const entry = entryOf(sessionId);
  return entry ? userTurnOrdinal(entry.items, itemId) === 1 : false;
}

export async function rewindToMessage(sessionId: string, itemId: string): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  const ordinal = userTurnOrdinal(current.items, itemId);
  if (ordinal === 0) return;

  channels.delete(sessionId);
  setEntry(sessionId, { ...current, reloading: true, live: false, state: "starting" });

  const { channel, activate } = bufferedChannel();
  let liveRewound: boolean;
  try {
    const result = await unwrap(
      commands.rewindSession(sessionId, ordinal, current.permissionMode, channel),
    );
    liveRewound = result.liveRewound;
  } catch (e) {
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, reloading: false, live: false, state: "idle" });
    throw e;
  }

  if (!liveRewound) {
    useToast.getState().show(
      "Rewound the transcript only",
      "The agent still remembers the removed turns. Its memory couldn't be truncated in place.",
      { tone: "warn" },
    );
  }

  held.delete(sessionId);
  const latest = entryOf(sessionId);
  if (latest) {
    const truncated = truncateAtUserOrdinal(latest, ordinal);
    setEntry(sessionId, {
      ...latest,
      ...truncated,
      appliedSeq: lastSeq(truncated.items),
      autoAllow: liveRewound ? latest.autoAllow : [],
      reloading: false,
      live: true,
      streaming: false,
      state: "running",
      pendingPermission: null,
      pendingQuestion: null,
      queued: [],
    });
  }

  channels.set(sessionId, channel);
  activate(sessionId);
}

function lastSeq(items: TranscriptItem[]): number {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const seq = items[i].seq;
    if (seq !== undefined && seq !== null) return seq;
  }
  return -1;
}

function truncateAtUserOrdinal(view: SessionView, ordinal: number): SessionView {
  let seen = 0;
  for (let i = 0; i < view.items.length; i += 1) {
    if (view.items[i].kind !== "user") continue;
    seen += 1;
    if (seen === ordinal) return { ...view, items: view.items.slice(0, i) };
  }
  return view;
}

export async function applyPermissionMode(
  sessionId: string,
  mode: PermissionMode | null,
  inPlace = false,
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current || current.permissionMode === mode) return;

  setEntry(sessionId, { ...current, permissionMode: mode, autoAllow: [] });
  if (!channels.has(sessionId)) return;
  if (inPlace) await unwrap(commands.setSessionPermissionMode(sessionId, mode));
  else await restartSession(sessionId, mode);
}

export async function applyModel(
  sessionId: string,
  modelFlag: string,
  effort: string | null,
  mode: PermissionMode | null,
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current || !channels.has(sessionId)) return;
  await restartSession(sessionId, mode, modelFlag, effort);
}

export async function sendMessage(
  sessionId: string,
  text: string,
  attachments: Attachment[] = [],
  promptName: string | null = null,
  onSent?: () => void,
): Promise<SendOutcome> {
  const current = entryOf(sessionId);
  if (!current) return "dropped";
  if (channels.has(sessionId) && isBusy(current)) {
    setEntry(sessionId, {
      ...current,
      queued: [...current.queued, { text, attachments, promptName, onSent }],
    });
    return "queued";
  }
  const sync = syncOf(sessionId);
  // Senders that skip the composer's guard (the handoff banner) pull first.
  if (sync && !canSend(sync)) await pullNow(sessionId);
  return deliverMessage(sessionId, text, attachments, promptName, "back", onSent);
}

async function deliverMessage(
  sessionId: string,
  text: string,
  attachments: Attachment[] = [],
  promptName: string | null = null,
  requeue: "front" | "back" = "back",
  onSent?: () => void,
): Promise<SendOutcome> {
  if (!entryOf(sessionId)) return "dropped";
  // Resume before adding the optimistic item: a session another device drove
  // in the meantime replays those turns first, so they land above this
  // prompt instead of after it. `flush: false` keeps the settle from sending
  // a queued message ahead of the one being delivered right now.
  if (!channels.has(sessionId)) await resumeSession(sessionId, { flush: false });
  const current = entryOf(sessionId);
  if (!current) return "dropped";
  // The resume attached mid-turn, or a pull showed one running: it would land inside that turn.
  // Working, not busy: a resumed log can re-show a dead process's ask, which must not block.
  if (channels.has(sessionId) && isWorking(current)) {
    const message: QueuedMessage = { text, attachments, promptName, onSent };
    const queued = requeue === "front" ? [message, ...current.queued] : [...current.queued, message];
    setEntry(sessionId, { ...current, queued });
    return "queued";
  }
  const clientMessageId = crypto.randomUUID();
  const withUser = addUserMessage(current, text, null, attachments, Date.now(), promptName, clientMessageId);
  const optimisticId = withUser.items[withUser.items.length - 1].id;
  // Only a sync server keeps a sync state; an older one ignores the base.
  const baseRev = syncOf(sessionId)?.localRev ?? null;
  setEntry(sessionId, { ...current, ...withUser, turnAuthor: "self" });
  // Busy from here on, so this turn's end may flush the next queued message.
  flushing.delete(sessionId);
  try {
    await unwrap(
      commands.sendUserMessage(
        sessionId,
        text,
        attachments.map((a) => a.path),
        promptName,
        clientMessageId,
        baseRev,
      ),
    );
  } catch (e) {
    const latest = entryOf(sessionId);
    if (latest) {
      const items = latest.items.filter((it) => it.id !== optimisticId);
      setEntry(sessionId, { ...latest, items, ...(isStaleTranscript(e) ? unsentTurn(current, latest, items) : {}) });
    }
    if (isStaleTranscript(e)) {
      syncInput(sessionId, {
        kind: "stale",
        rev: rev(e.staleTranscript.rev),
        committedRev: rev(e.staleTranscript.committedRev),
      });
    }
    throw e;
  }
  return "sent";
}

// A refused send never reached the harness, so the turn it opened locally never ran. But a
// log item applied meanwhile (the winner's message) opened a real one, which keeps running.
function unsentTurn(
  before: SessionEntry,
  latest: SessionEntry,
  items: TranscriptItem[],
): Pick<SessionEntry, "streaming" | "turnAuthor"> {
  if ((latest.appliedSeq ?? -1) === (before.appliedSeq ?? -1)) {
    return { streaming: before.streaming, turnAuthor: before.turnAuthor };
  }
  let author: ClientRef | null = null;
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const item = items[i];
    if (item.kind !== "user") continue;
    author = item.author ?? null;
    break;
  }
  const turnAuthor: TurnAuthor = !author ? null : author.clientId === selfClientId() ? "self" : "other";
  return { streaming: latest.streaming, turnAuthor };
}

const flushing = new Set<string>();

function flushQueuedMessage(sessionId: string): void {
  const entry = entryOf(sessionId);
  // One flush per session: a pull's own trailing flush must not overtake the one waiting on it.
  if (!entry || entry.queued.length === 0 || isBusy(entry) || flushing.has(sessionId)) return;
  const [next, ...rest] = entry.queued;
  setEntry(sessionId, { ...entry, queued: rest });
  flushing.add(sessionId);
  void deliverQueued(sessionId, next).then((requeued) => {
    flushing.delete(sessionId);
    if (!requeued) return;
    const sync = syncOf(sessionId);
    // A pull that closed the gap while this flush held the slot found nothing to send.
    if (!behind(sync)) flushQueuedMessage(sessionId);
    // A refusal's commits are read now, since the user already sent; an in-flight send alone has nothing to read.
    else if (sync && sync.committedRev > sync.localRev) void pullNow(sessionId);
  });
}

const behind = (sync: SyncState | undefined) => !!sync && sync.remoteRev > sync.localRev;

// A refused or still-behind flush keeps its place; whatever closes the gap flushes again.
async function deliverQueued(sessionId: string, next: QueuedMessage): Promise<boolean> {
  const requeue = () => {
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, queued: [next, ...latest.queued] });
    return true;
  };
  try {
    if (behind(syncOf(sessionId))) await pullNow(sessionId);
    // Another client's send is still in flight: sending now would only 409.
    if (behind(syncOf(sessionId))) return requeue();
    const outcome = await deliverMessage(
      sessionId,
      next.text,
      next.attachments,
      next.promptName,
      "front",
      next.onSent,
    );
    if (outcome === "sent") next.onSent?.();
  } catch (e) {
    if (isStaleTranscript(e)) return requeue();
    log.warn("session", "sending a queued message failed", e);
  }
  return false;
}

const pulls = new Map<string, Promise<void>>();
const reloads = new Map<string, Promise<void>>();

/** Pulls held and missed turns now; resolves when the running pull or reload ends. */
export function pullNow(sessionId: string): Promise<void> {
  const running = pulls.get(sessionId) ?? reloads.get(sessionId);
  if (running) return running;
  syncInput(sessionId, { kind: "pull" });
  unstick(sessionId);
  return pulls.get(sessionId) ?? reloads.get(sessionId) ?? Promise.resolve();
}

// `fetching` with no read in flight ignores every `pull` (a reload gated off by a mode
// restart leaves it so); a pull ends it with `fetched`, `reset` or `fetchFailed`.
function unstick(sessionId: string): void {
  if (!syncOf(sessionId)?.fetching || pulls.has(sessionId) || reloads.has(sessionId)) return;
  log.warn("session", `${sessionId} was left syncing with no read in flight; pulling`);
  startPull(sessionId);
}

// Asked for while the slot was taken, e.g. by `fetched` seeing a commit land mid-read.
const repull = new Set<string>();

// Single-flight. No pull is asked for during a reload: `fetching` stays set until its `loaded`.
function startPull(sessionId: string): void {
  if (!entryOf(sessionId)) return;
  if (pulls.has(sessionId)) {
    repull.add(sessionId);
    return;
  }
  // Deferred so the slot is taken before applying held items can ask for another pull.
  const run = (async () => {
    await Promise.resolve();
    do {
      repull.delete(sessionId);
      await pullOnce(sessionId);
    } while (repull.has(sessionId) && entryOf(sessionId));
  })().finally(() => {
    pulls.delete(sessionId);
    repull.delete(sessionId);
  });
  pulls.set(sessionId, run);
}

async function pullOnce(sessionId: string): Promise<void> {
  try {
    const selfId = selfClientId();
    const applied = applyHeld(sessionId);
    let device: string | null = null;
    for (const item of applied) if (item.author && item.author.clientId !== selfId) device = item.author.device;
    let firstSeq = applied.find((item) => item.seq !== null)?.seq ?? null;
    let fetched: { rev: number; resetRev: number; inFlight: number } | null = null;

    const sync = syncOf(sessionId);
    if (sync && (sync.localRev < sync.remoteRev || sync.committedRev > sync.localRev)) {
      let cursor: LogCursorDto = entryOf(sessionId)?.cursor ?? { kind: "empty" };
      for (;;) {
        const page = await unwrap(commands.listSessionTurns(sessionId, cursor, PULL_PAGE));
        const current = entryOf(sessionId);
        if (!current) return;
        const before = current.appliedSeq ?? -1;
        const merged = mergeEntries(current, before, page);
        if (merged.reset) {
          syncInput(sessionId, { kind: "reset" });
          return;
        }
        const opened = turnOpeners(page.entries, before);
        setEntry(sessionId, {
          ...current,
          ...merged.view,
          appliedSeq: merged.appliedSeq,
          cursor: merged.cursor,
          turnAuthor: opened.turnAuthor === undefined ? current.turnAuthor : opened.turnAuthor,
        });
        // The stream's own exit will now be skipped as already applied.
        if (isTerminal(merged.view.state) && channels.has(sessionId)) endStream(sessionId, current.workstreamId);
        trimHeld(sessionId, merged.appliedSeq);
        device = opened.device ?? device;
        firstSeq ??= merged.firstNewSeq;
        cursor = merged.cursor;
        if (page.remaining === 0) {
          fetched = { rev: rev(page.rev), resetRev: rev(page.resetRev), inFlight: page.inFlight };
          break;
        }
      }
    }

    const latest = syncOf(sessionId);
    syncInput(sessionId, {
      kind: "fetched",
      rev: fetched?.rev ?? latest?.localRev ?? 0,
      resetRev: fetched?.resetRev ?? latest?.resetRev ?? 0,
      inFlight: fetched?.inFlight ?? 0,
      remaining: 0,
      device,
    });
    if (firstSeq !== null && device) setDivider(sessionId, { seq: firstSeq, device });
    // Items that arrived during the read are still held; `fetched` cleared their flag.
    reportHeld(sessionId);
  } catch (e) {
    if (isSessionEpochAdvanced(e)) {
      syncInput(sessionId, { kind: "reset" });
    } else if (isAppError(e) && e.code === "session_not_found") {
      syncInput(sessionId, { kind: "gone" });
    } else {
      log.warn("session", `pulling new turns into ${sessionId} failed`, e);
      syncInput(sessionId, { kind: "fetchFailed" });
    }
  }
}

// Absorbs the reload a 15s poll repeats while one is pending.
function startReload(sessionId: string): void {
  if (reloads.has(sessionId) || entryOf(sessionId)?.reloading) return;
  reloads.set(
    sessionId,
    reloadOnce(sessionId).finally(() => reloads.delete(sessionId)),
  );
}

async function reloadOnce(sessionId: string): Promise<void> {
  let folded = true;
  try {
    await refreshSession(sessionId);
    // A resume already in flight folded nothing here, so no `loaded` arrived.
    folded = !syncOf(sessionId)?.fetching || pulls.has(sessionId);
  } catch (e) {
    log.warn("session", `reloading ${sessionId} after a rewind failed`, e);
    folded = false;
  }
  // Without a `loaded`, `fetching` would hold Send back for good.
  if (!folded && entryOf(sessionId)) syncInput(sessionId, { kind: "fetchFailed" });
}

export function cancelQueuedMessage(sessionId: string, index: number): void {
  const current = entryOf(sessionId);
  if (!current) return;
  setEntry(sessionId, { ...current, queued: current.queued.filter((_, i) => i !== index) });
}

export async function respondPermission(
  sessionId: string,
  requestId: string,
  allow: boolean,
  message: string | null,
): Promise<void> {
  const current = entryOf(sessionId);
  if (current) {
    setEntry(sessionId, {
      ...current,
      ...resolvePermission(current, requestId, allow ? "allow" : "deny"),
    });
  }
  if (!channels.has(sessionId)) {
    await resumeSession(sessionId);
    return;
  }
  await unwrap(commands.respondPermission(sessionId, requestId, allow, message));
}

export async function answerQuestion(
  sessionId: string,
  requestId: string,
  answers: Record<string, string>,
): Promise<void> {
  const current = entryOf(sessionId);
  if (current) {
    setEntry(sessionId, { ...current, ...resolveQuestion(current, requestId) });
  }
  if (!channels.has(sessionId)) {
    await resumeSession(sessionId);
    return;
  }
  await unwrap(commands.answerQuestion(sessionId, requestId, answers));
}

export async function allowForSession(
  sessionId: string,
  requestId: string,
  toolName: string,
): Promise<void> {
  const current = entryOf(sessionId);
  if (current && !current.autoAllow.includes(toolName)) {
    setEntry(sessionId, { ...current, autoAllow: [...current.autoAllow, toolName] });
  }
  await respondPermission(sessionId, requestId, true, null);
}

export async function interruptSession(sessionId: string): Promise<void> {
  await unwrap(commands.interruptSession(sessionId));
}

const STOP_ESCALATION_MS = 3000;
export async function stopSession(sessionId: string): Promise<void> {
  if (!channels.has(sessionId)) return;

  const parked = entryOf(sessionId);
  if (parked && isBusy(parked) && !isWorking(parked)) {
    const cleared = cancelPendingPrompts(parked);
    setEntry(sessionId, { ...parked, ...cleared, streaming: false, state: parkedState(cleared) });
    try {
      if (parked.pendingQuestion) {
        await unwrap(commands.answerQuestion(sessionId, parked.pendingQuestion.requestId, {}));
      } else {
        const requestId = openPermissionRequestId(parked);
        if (requestId) await unwrap(commands.respondPermission(sessionId, requestId, false, null));
      }
    } catch (e) {
      log.warn("session", `declining ${sessionId}'s open prompt failed`, e);
    }
  }

  try {
    await interruptSession(sessionId);
  } catch (e) {
    log.warn("session", `interrupting ${sessionId} failed`, e);
  }
  const settled = await waitForView(sessionId, (v) => !isWorking(v), STOP_ESCALATION_MS);
  if (settled) return;
  log.info("session", `${sessionId} did not stop after an interrupt; killing it`);
  try {
    await killSession(sessionId);
  } catch (e) {
    log.warn("session", `killing ${sessionId} failed`, e);
  }
}

function openPermissionRequestId(entry: SessionEntry): string | null {
  if (entry.pendingPermission) return entry.pendingPermission.requestId;
  for (const it of entry.items) {
    if (it.kind === "permission" && it.decision === null) return it.requestId;
  }
  return null;
}

function waitForView(
  sessionId: string,
  predicate: (view: SessionEntry) => boolean,
  timeoutMs: number,
): Promise<boolean> {
  const initial = entryOf(sessionId);
  if (!initial || predicate(initial)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const finish = (result: boolean) => {
      clearTimeout(timer);
      unsub();
      resolve(result);
    };
    const timer = setTimeout(() => finish(false), timeoutMs);
    const unsub = useSessionStore.subscribe((state) => {
      const entry = state.sessions[sessionId];
      if (!entry || predicate(entry)) finish(true);
    });
  });
}

export async function killSession(sessionId: string): Promise<void> {
  if (!channels.has(sessionId)) {
    const current = entryOf(sessionId);
    if (current) {
      setEntry(sessionId, {
        ...current,
        ...reduceEvent(current, { kind: "exited", outcome: "killed", code: null, stderrTail: "" }),
        live: false,
      });
    }
    return;
  }
  await unwrap(commands.killSession(sessionId));
}

// Archiving or closing kills the harness, so running work would be lost; an agent parked on an ask is not working.
export function agentStillWorking(entry: SessionEntry): boolean {
  return isWorking(entry) || (entry.live && backgroundTasksFromItems(entry.items).length > 0);
}

// Ends an archived or closed agent. Its persisted exit expires the asks it left open.
export async function endSession(sessionId: string): Promise<void> {
  try {
    if (channels.has(sessionId)) {
      await killSession(sessionId);
      return;
    }
    // After a restart the server can still run a tab this window never opened.
    const active = await unwrap(commands.listActiveSessions());
    if (active.some((a) => a.session.id === sessionId)) {
      await unwrap(commands.killSession(sessionId));
    }
  } catch (e) {
    log.warn("session", `ending ${sessionId} failed`, e);
  }
}

function commitShipDefaults(): ActionAgentDefaults {
  return actionAgentDefaults(queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings);
}

function claudeCatalogSnapshot(): ModelOption[] {
  return claudeModels(queryClient.getQueryData<ClaudeProbe>(["claude-probe"])?.models);
}

async function effectiveCommitText(workstreamId: string): Promise<string> {
  const boot = queryClient.getQueryData<Bootstrap>(["bootstrap"]);
  const raw = effectiveBuiltinBody(boot?.settings, boot?.promptDefaults, "commit");
  const body = raw.trim() ? raw : (boot?.promptDefaults?.commit ?? "/commit");
  if (!bodyHasVars(body)) return body;
  const ctx = await unwrap(commands.workstreamPromptContext(workstreamId));
  return substitutePromptVars(body, ctx);
}

const promptStartsInFlight = new Set<string>();

export async function runPromptTextInWorkstream(
  workstreamId: string,
  text: string,
  title: string,
  targetShellId?: number,
): Promise<void> {
  if (!text.trim()) return;
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const pane = tabs ? focusedPaneId(tabs) : undefined;
  const active =
    targetShellId !== undefined
      ? tabs?.shells.find((s) => s.id === targetShellId)
      : tabs && pane
        ? paneShells(tabs, pane).find((s) => s.id === tabs.panes[pane]?.activeId)
        : undefined;

  if (active?.sessionId) {
    if (!entryOf(active.sessionId)) {
      await restoreSessionIfNeeded(
        active.sessionId,
        workstreamId,
        active.sessionStage ?? active.mode,
        permissionModeOption(active.permissionMode).flag,
      );
    }
    const entry = entryOf(active.sessionId);
    if (entry && !isTerminal(entry.state)) {
      await sendMessage(active.sessionId, text, [], title);
      return;
    }
  }

  if (promptStartsInFlight.has(workstreamId)) return;
  promptStartsInFlight.add(workstreamId);
  try {
    const settings = queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings;
    const reuseShell =
      active &&
      !active.sessionId &&
      active.mode === "general" &&
      !active.docPreviewPath &&
      !active.diffView &&
      !active.filePath &&
      !active.terminalView &&
      !active.ptyAgent;
    const session = await startSession({
      workstreamId,
      stage: "general",
      input: text,
      ...newChatLaunchArgs(settings, reuseShell ? active : null, claudeCatalogSnapshot()),
      promptName: title,
    });
    if (reuseShell) {
      bindSessionToShell(workstreamId, active.id, session.id);
    } else {
      openPromptTab(workstreamId, session.id, title);
    }
  } finally {
    promptStartsInFlight.delete(workstreamId);
  }
}

// Loads, then attaches, the implement session; false when only a fresh session can take the review.
async function attachForReview(workstreamId: string, implement: Session): Promise<boolean> {
  if (implement.state === "spawn_failed") return false;
  await restoreSessionIfNeeded(
    implement.id,
    workstreamId,
    "implement",
    permissionModeOption(sessionPermissionMode(workstreamId, implement.id)).flag,
  );
  const entry = entryOf(implement.id);
  if (!entry || entry.state === "spawn_failed") return false;
  if (channels.has(implement.id)) return true;
  try {
    await resumeSession(implement.id, { flush: false });
  } catch (e) {
    log.warn("session", `resuming ${implement.id} for a review failed; starting a new session`, e);
    return false;
  }
  return entryOf(implement.id) !== undefined;
}

/**
 * Sends a review to the implement session, resuming a finished one so it keeps its
 * context. `onQueuedSent` runs if the text was queued behind a turn, once it goes out.
 */
export async function deliverReview(
  workstreamId: string,
  implement: Session | null,
  text: string,
  onQueuedSent?: () => void,
): Promise<"sent" | "queued"> {
  if (implement && (await attachForReview(workstreamId, implement))) {
    const outcome = await sendMessage(implement.id, text, [], REVIEW_PROMPT, onQueuedSent);
    if (outcome !== "dropped") {
      openSessionTab(workstreamId, implement);
      return outcome;
    }
  }
  const settings = queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings;
  const session = await startSession({
    workstreamId,
    stage: "general",
    input: text,
    promptName: REVIEW_PROMPT,
    ...newChatLaunchArgs(settings, null, claudeCatalogSnapshot()),
  });
  openPromptTab(workstreamId, session.id, "Review");
  return "sent";
}

export async function commitWorkstream(workstreamId: string): Promise<void> {
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const commitTab = tabs?.shells.find((s) => s.mode === "commit");
  if (commitTab?.sessionId) {
    const entry = entryOf(commitTab.sessionId);
    if (entry && !isTerminal(entry.state)) {
      openCommitTab(workstreamId, commitTab.sessionId);
      await sendMessage(commitTab.sessionId, await effectiveCommitText(workstreamId));
      return;
    }
  }
  const defaults = commitShipDefaults();
  const session = await startSession({
    workstreamId,
    stage: "commit",
    input: "",
    ...actionLaunchArgs(defaults, claudeCatalogSnapshot()),
  });
  openCommitTab(workstreamId, session.id, defaults);
}

export async function approvePlanAndBuild(
  workstreamId: string,
  planPath: string,
  from: ShellTab,
): Promise<void> {
  const settings = queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings;
  const session = await startSession({
    workstreamId,
    stage: "implement",
    input: "",
    attachments: [{ path: planPath, name: docTitle(planPath), kind: "file" }],
    ...newChatLaunchArgs(settings, from, claudeCatalogSnapshot()),
  });
  openImplementTab(workstreamId, session.id, planPath, from);
}

export async function shipWorkstream(workstreamId: string): Promise<void> {
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const shipTab = tabs?.shells.find((s) => s.mode === "ship");
  if (shipTab?.sessionId) {
    const entry = entryOf(shipTab.sessionId);
    if (entry && !isTerminal(entry.state)) {
      openShipTab(workstreamId, shipTab.sessionId);
      return;
    }
  }
  const defaults = commitShipDefaults();
  const session = await startSession({
    workstreamId,
    stage: "ship",
    input: "",
    ...actionLaunchArgs(defaults, claudeCatalogSnapshot()),
  });
  openShipTab(workstreamId, session.id, defaults);
}

export async function addressPrFeedback(
  workstreamId: string,
  kind: "ci" | "comments",
  prNumber: number,
): Promise<void> {
  const instruction =
    kind === "ci"
      ? `CI checks are failing on PR #${prNumber}. Investigate the failures ` +
        "(`gh pr checks`, `gh run view --log-failed`), fix them, then commit and " +
        "push to this branch."
      : `Address the review feedback on PR #${prNumber}. Read it with ` +
        "`gh pr view --comments`, make the requested changes, then commit and push " +
        "to this branch.";

  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const shipTab = tabs?.shells.find((s) => s.mode === "ship");
  if (shipTab?.sessionId) {
    const entry = entryOf(shipTab.sessionId);
    if (entry && !isTerminal(entry.state)) {
      openShipTab(workstreamId, shipTab.sessionId);
      await sendMessage(shipTab.sessionId, instruction);
      return;
    }
  }
  const defaults = commitShipDefaults();
  const session = await startSession({
    workstreamId,
    stage: "revise",
    input: instruction,
    ...actionLaunchArgs(defaults, claudeCatalogSnapshot()),
  });
  openShipTab(workstreamId, session.id, defaults);
}

export async function restoreSessionIfNeeded(
  sessionId: string,
  workstreamId: string,
  stage: string,
  permissionMode: PermissionMode | null = null,
): Promise<void> {
  if (entryOf(sessionId) || channels.has(sessionId) || restoring.has(sessionId)) return;
  restoring.add(sessionId);
  try {
    const [read, active] = await Promise.all([
      readLog(sessionId),
      // The server outlives the app (ADR-0009): a session it still reports
      // live is attached to below rather than shown as a frozen transcript.
      unwrap(commands.listActiveSessions()).catch((e) => {
        log.warn("session", "listing live sessions failed; restoring as if none were live", e);
        return [];
      }),
    ]);
    if (entryOf(sessionId) || channels.has(sessionId)) return;

    const { view: folded, ...position } = foldLog(read);
    const live = active.some((a) => a.session.id === sessionId);
    const view = restoredView(folded, live);
    setEntry(sessionId, {
      ...view,
      ...position,
      workstreamId,
      stage,
      autoAllow: [],
      live: false,
      permissionMode,
      workingSince: null,
      taskStartedAt: {},
      queued: [],
    });
    foldLoaded(sessionId, read);
    if (live) await resumeSession(sessionId, { cursor: position.cursor });
  } catch (error) {
    log.error("session", `restoring ${sessionId} failed`, error);
  } finally {
    restoring.delete(sessionId);
  }
}

async function liveSessionIds(): Promise<Set<string>> {
  const active = await unwrap(commands.listActiveSessions());
  return new Set(active.map((a) => a.session.id));
}

// A retired channel may still be delivering (a live stream this window
// replaced); its events would land twice beside the fresh attach's replay.
// ponytail: the retired stream keeps running server-side until the session
// exits; expose cancel_stream to the frontend if that ever shows up in load.
const retired = new WeakSet<Channel<SessionStreamItemDto>>();

async function resync(sessionId: string, live: boolean): Promise<void> {
  const inFlight = resuming.get(sessionId);
  if (inFlight) return inFlight;
  if (entryOf(sessionId)?.reloading) return;
  const old = channels.get(sessionId);
  if (old) retired.add(old);
  channels.delete(sessionId);
  if (live) return resumeSession(sessionId);
  const read = await readLog(sessionId);
  const current = entryOf(sessionId);
  if (!current || channels.has(sessionId)) return;
  const { view, ...position } = foldLog(read, current.items);
  setEntry(sessionId, { ...current, ...restoredView(view, false), ...position, live: false, drifted: false });
  foldLoaded(sessionId, read);
}

/** Hides the no-sync "new turns" notice until the next lifecycle event marks the view again. */
export function dismissDrift(sessionId: string): void {
  const entry = entryOf(sessionId);
  if (entry?.drifted) setEntry(sessionId, { ...entry, drifted: false });
}

export async function refreshSession(sessionId: string): Promise<void> {
  await resync(sessionId, (await liveSessionIds()).has(sessionId));
}

// A dropped connection ends every stream without an `Exited`, so the channels
// still registered here are dead and would block any resume.
export async function reattachSessions(): Promise<void> {
  const live = await liveSessionIds();
  await Promise.all(
    Object.keys(useSessionStore.getState().sessions).map((id) =>
      resync(id, live.has(id)).catch((e) =>
        log.warn("session", `reattaching ${id} after a reconnect failed`, e),
      ),
    ),
  );
}

function markDrifted(sessionId: string, state: SessionState): void {
  const entry = entryOf(sessionId);
  if (!entry || entry.drifted || entry.reloading) return;
  if (channels.has(sessionId) || resuming.has(sessionId)) return;
  // This window's own stop already shows the exit the event announces.
  if (isTerminal(state) && isTerminal(entry.state)) return;
  setEntry(sessionId, { ...entry, drifted: true });
}

export const sessionBridge: EventBridge = () => [
  events.sessionLifecycle.listen(({ payload }) => {
    if (!entryOf(payload.sessionId)) return;
    // A sync server pulls other devices' turns itself.
    void syncSupport().then((synced) => {
      if (!synced) markDrifted(payload.sessionId, payload.state);
    });
  }),
];

function onRevised(event: SessionRevised): void {
  const { sessionId } = event;
  if (!entryOf(sessionId)) return;
  const queue = held.get(sessionId);
  const selfId = selfClientId();
  const at = rev(event.rev);
  syncInput(sessionId, {
    kind: "remote",
    rev: at,
    turns: event.turns.length > 0 ? [{ rev: at, kinds: [...new Set(event.turns.map((t) => t.kind))] }] : [],
    device: event.author?.device ?? null,
    working: event.working,
    // A turnless revision must not clear what the held buffer reported.
    flushable: queue ? heldSummary(queue).flushable : false,
    attached: channels.has(sessionId),
    own: selfId !== null && event.author?.clientId === selfId,
  });
}

function onReset(event: SessionReset): void {
  const entry = entryOf(event.sessionId);
  if (!entry || entry.reloading) return;
  if (rev(event.rev) > (syncOf(event.sessionId)?.resetRev ?? 0)) syncInput(event.sessionId, { kind: "reset" });
}

setSyncHandlers({
  pull: startPull,
  reload: startReload,
  flush: (sessionId) => {
    unstick(sessionId);
    flushQueuedMessage(sessionId);
  },
  watched: () => Object.keys(useSessionStore.getState().sessions),
  revised: onRevised,
  reset: onReset,
});

export function transcriptToText(sessionId: string): string {
  const entry = useSessionStore.getState().sessions[sessionId];
  return entry ? viewToText(entry) : "(no transcript)";
}

export function useSession(sessionId: string | undefined): SessionEntry | undefined {
  return useSessionStore((s) => (sessionId ? s.sessions[sessionId] : undefined));
}

export type CreatedDoc = { path: string; kind: DocType; title: string };

export function useWorkstreamDocs(workstreamId: string | null): CreatedDoc[] {
  const written = useWrittenDocs(workstreamId);
  const keys = useSessionStore(
    useShallow((s) => {
      if (!workstreamId) return [] as string[];
      const seen = new Set<string>();
      const out: string[] = [];
      for (const entry of Object.values(s.sessions)) {
        if (entry.workstreamId !== workstreamId) continue;
        for (const doc of sessionDocs(entry.items, written)) {
          if (seen.has(doc.path)) continue;
          seen.add(doc.path);
          out.push(`${doc.kind}\t${doc.path}`);
        }
      }
      return out;
    }),
  );
  return useMemo(
    () =>
      keys.map((k) => {
        const tab = k.indexOf("\t");
        const kind = k.slice(0, tab) as DocType;
        const path = k.slice(tab + 1);
        return { kind, path, title: docTitle(path) };
      }),
    [keys],
  );
}

export function useMostRecentDoc(
  workstreamId: string | null,
  kind: DocType | null,
): CreatedDoc | null {
  const written = useWrittenDocs(workstreamId);
  const key = useSessionStore(
    useShallow((s) => {
      if (!workstreamId || !kind) return null;
      const docs = Object.values(s.sessions)
        .filter((entry) => entry.workstreamId === workstreamId)
        .flatMap((entry) => sessionDocs(entry.items, written))
        .filter((doc) => doc.kind === kind);
      return docs[docs.length - 1]?.path ?? null;
    }),
  );
  return useMemo(
    () => (key ? { kind: kind as DocType, path: key, title: docTitle(key) } : null),
    [key, kind],
  );
}

export function useWorkstreamArtifacts(workstreamId: string | null): SessionArtifact[] {
  const keys = useSessionStore(
    useShallow((s) => {
      if (!workstreamId) return [] as string[];
      const byUrl = new Map<string, string>();
      for (const entry of Object.values(s.sessions)) {
        if (entry.workstreamId !== workstreamId) continue;
        for (const a of artifactsFromItems(entry.items)) {
          byUrl.set(a.url, [a.url, a.title, a.description ?? "", a.filePath].join("\0"));
        }
      }
      return [...byUrl.values()];
    }),
  );
  return useMemo(
    () =>
      keys.map((k) => {
        const [url, title, description, filePath] = k.split("\0");
        return { url, title, description: description || null, filePath } satisfies SessionArtifact;
      }),
    [keys],
  );
}

export type LiveBackgroundTask = BackgroundTask & { startedAt: number | null };

export function useSessionBackgroundTasks(sessionId: string | undefined): LiveBackgroundTask[] {
  const keys = useSessionStore(
    useShallow((s) => {
      if (!sessionId) return [] as string[];
      const entry = s.sessions[sessionId];
      if (!entry) return [] as string[];
      if (isTerminal(entry.state) || !entry.live) return [] as string[];
      const tasks = backgroundTasksFromItems(entry.items);
      return tasks.map((t) => {
        const startedAt = entry.taskStartedAt[t.toolUseId] ?? 0;
        return [t.toolUseId, t.taskId ?? "", t.status, startedAt, t.label, t.command].join("\0");
      });
    }),
  );
  return useMemo(
    () =>
      keys.map((k) => {
        const [toolUseId, taskIdRaw, status, startedAtStr, label, ...cmdParts] = k.split("\0");
        return {
          toolUseId,
          taskId: taskIdRaw || null,
          status: status as BackgroundTaskStatus,
          startedAt: Number(startedAtStr) || null,
          label,
          command: cmdParts.join("\0"),
        } satisfies LiveBackgroundTask;
      }),
    [keys],
  );
}

export function useSessionPlanTasks(sessionId: string | undefined): PlanTask[] {
  const keys = useSessionStore(
    useShallow((s) => {
      if (!sessionId) return [] as string[];
      const entry = s.sessions[sessionId];
      if (!entry) return [] as string[];
      return planTasksFromItems(entry.items).map((t) => [t.id ?? "", t.subject, t.status].join("\0"));
    }),
  );
  return useMemo(
    () =>
      keys.map((k) => {
        const [id, subject, status] = k.split("\0");
        return { id: id || null, subject, status: status as PlanTaskStatus } satisfies PlanTask;
      }),
    [keys],
  );
}

// The phase label a live implement session in this workstream is building, from its phase todo.
export function useWorkstreamBuildPhase(workstreamId: string): string | null {
  return useSessionStore((s) => {
    for (const entry of Object.values(s.sessions)) {
      if (entry.workstreamId !== workstreamId || entry.stage !== "implement") continue;
      if (!entry.live || !isWorking(entry)) continue;
      for (const task of planTasksFromItems(entry.items)) {
        if (task.status !== "in_progress") continue;
        const label = todoPhaseLabel(task.subject);
        if (label) return label;
      }
    }
    return null;
  });
}

export type { WorkstreamActivity } from "@/lib/domain/activity";

const ACTIVITY_RANK: Record<WorkstreamActivity, number> = {
  idle: 0,
  review: 1,
  background: 2,
  error: 3,
  working: 4,
  "needs-you": 5,
};

export function isAwaiting(entry: SessionEntry): boolean {
  return (
    entry.state === "awaiting_permission" ||
    entry.state === "awaiting_question" ||
    hasPendingPermission(entry) ||
    entry.pendingQuestion !== null
  );
}

export function classifyEntry(entry: SessionEntry): WorkstreamActivity {
  if (isAwaiting(entry)) return "needs-you";
  if (isWorking(entry)) return "working";
  if (entry.state === "crashed" || entry.state === "spawn_failed") return "error";
  if (entry.state === "completed" || (entry.state === "running" && !entry.streaming)) {
    if (entry.live && !isTerminal(entry.state) && backgroundTasksFromItems(entry.items).length > 0) {
      return "background";
    }
    return "review";
  }
  return "idle";
}

export function useWorkstreamActivity(workstreamId: string | null): WorkstreamActivity {
  return useSessionStore((s) => {
    if (!workstreamId) return "idle";
    let best: WorkstreamActivity = "idle";
    for (const entry of Object.values(s.sessions)) {
      if (entry.workstreamId !== workstreamId) continue;
      const cur = classifyEntry(entry);
      if (ACTIVITY_RANK[cur] > ACTIVITY_RANK[best]) best = cur;
    }
    return best;
  });
}

export function workstreamShipBusy(
  sessions: Record<string, SessionEntry>,
  workstreamId: string | null,
): boolean {
  if (!workstreamId) return false;
  for (const entry of Object.values(sessions)) {
    if (entry.workstreamId !== workstreamId || entry.stage !== "ship") continue;
    const activity = classifyEntry(entry);
    if (activity === "working" || activity === "needs-you") return true;
  }
  return false;
}

export function useWorkstreamShipBusy(workstreamId: string | null): boolean {
  return useSessionStore((s) => workstreamShipBusy(s.sessions, workstreamId));
}
