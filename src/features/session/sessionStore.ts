
import { useMemo } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";
import { useShallow } from "zustand/shallow";

import {
  commands,
  type Bootstrap,
  type ClaudeProbe,
  type Harness,
  type PermissionMode,
  type Stage,
  type Session,
  type SessionEvent,
} from "@/lib/bindings";
import type { WorkstreamActivity } from "@/lib/domain/activity";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";
import { invalidateWorkstreamPr, invalidateWorktreeGit } from "@/lib/queries";
import { findWorkstreamForNotify, notifyNeedsAttention } from "@/lib/notifications";
import { useToast } from "@/lib/toast";
import { currentAppLocation } from "@/lib/appNav";
import type { DocKind } from "@/lib/domain/docs";
import { docTitle, sessionDocs } from "./toolDisplay";
import { artifactsFromItems, type SessionArtifact } from "./artifacts";
import { forgetWorkstreamDocs, useWrittenDocs } from "./writtenDocs";
import {
  backgroundTasksFromItems,
  type BackgroundTask,
  type BackgroundTaskStatus,
} from "./backgroundTask";
import { planTasksFromItems, type PlanTask, type PlanTaskStatus } from "./planTasks";
import { type Attachment } from "./attachments";
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
  openPromptTab,
  openShipTab,
  paneShells,
  setSessionPermissionMode,
  useWorkstreamTabsStore,
  visibleShells,
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
} from "./transcript";

export type SessionEntry = SessionView & {
  workstreamId: string;
  stage: string;
  autoAllow: string[];
  live: boolean;
  permissionMode: PermissionMode | null;
  /** The harness's own plan mode, layered over `permissionMode` (codex, OpenCode). */
  planning?: boolean;
  reloading?: boolean;
  queued: QueuedMessage[];
  workingSince: number | null;
  taskStartedAt: Record<string, number>;
};

export type QueuedMessage = {
  text: string;
  attachments: Attachment[];
  promptName: string | null;
};

type State = { sessions: Record<string, SessionEntry> };

export const useSessionStore = create<State>(() => ({ sessions: {} }));

const channels = new Map<string, Channel<SessionEvent>>();

const restoring = new Set<string>();

function entryOf(sessionId: string): SessionEntry | undefined {
  return useSessionStore.getState().sessions[sessionId];
}

function setEntry(sessionId: string, next: SessionEntry): void {
  useSessionStore.setState((s) => {
    const prev = s.sessions[sessionId];
    const workingSince = nextWorkingSince(prev, next);
    const taskStartedAt = nextTaskStartedAt(prev, next);
    return { sessions: { ...s.sessions, [sessionId]: { ...next, workingSince, taskStartedAt } } };
  });
  if (isWorking(next)) lastWorkingAt.set(next.workstreamId, Date.now());
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
  for (const id of ids) channels.delete(id);
  useSessionStore.setState((s) => {
    const next = { ...s.sessions };
    for (const id of ids) delete next[id];
    return { sessions: next };
  });
}

export function dropSession(sessionId: string): void {
  channels.delete(sessionId);
  useSessionStore.setState((s) => {
    if (!s.sessions[sessionId]) return s;
    const next = { ...s.sessions };
    delete next[sessionId];
    return { sessions: next };
  });
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

// Events can arrive before the command returns the session id.
function bufferedChannel(): {
  channel: Channel<SessionEvent>;
  activate: (sessionId: string) => void;
} {
  const buffer: SessionEvent[] = [];
  let boundId: string | null = null;
  const channel = new Channel<SessionEvent>();
  channel.onmessage = (event) => {
    if (boundId) dispatch(boundId, event);
    else buffer.push(event);
  };
  return {
    channel,
    activate(sessionId) {
      boundId = sessionId;
      for (const event of buffer) dispatch(sessionId, event);
      buffer.length = 0;
    },
  };
}

function dispatch(sessionId: string, event: SessionEvent): void {
  const current = entryOf(sessionId);
  if (!current) return;

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
    });
    void queryClient.invalidateQueries({ queryKey: ["workstream", current.workstreamId, "diff"] });
    return;
  }

  const wasNeedsYou = classifyEntry(current) === "needs-you";
  const nextEntry: SessionEntry = { ...current, ...reduceEvent(current, event), ...modeFields(event) };
  setEntry(sessionId, nextEntry);
  if (event.kind === "permissionModeChanged") {
    setSessionPermissionMode(current.workstreamId, sessionId, event.mode);
  }

  const isNeedsYou = classifyEntry(nextEntry) === "needs-you";
  const autoApproved =
    event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName);
  if (!wasNeedsYou && isNeedsYou && !autoApproved) {
    void maybeNotifyTransition(sessionId, current.workstreamId, current.stage);
  }

  if (event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName)) {
    void respondPermission(sessionId, event.requestId, true, null);
  }
  if (event.kind === "turnDone" || event.kind === "exited") {
    invalidateWorktreeGit(current.workstreamId);
    invalidateWorkstreamPr(current.workstreamId);
  }
  if (event.kind === "turnDone") {
    flushQueuedMessage(sessionId);
  }
  if (event.kind === "exited") {
    channels.delete(sessionId);
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, live: false });
    void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
  }
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
      },
      channel,
    ),
  );

  channels.set(session.id, channel);

  let view = initialSessionView("starting");
  view = { ...view, model: session.model, claudeSessionId: session.claudeSessionId };
  view = addUserMessage(view, prompt, args.stage, attachments, Date.now(), args.promptName ?? null);
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
  });

  activate(session.id);
  return session;
}

export async function resumeSession(sessionId: string): Promise<void> {
  const current = entryOf(sessionId);
  if (!current || channels.has(sessionId)) return;

  const { channel, activate } = bufferedChannel();
  channels.set(sessionId, channel);
  setEntry(sessionId, { ...current, live: true, state: "starting" });

  try {
    await unwrap(commands.resumeSession(sessionId, null, null, current.permissionMode, channel));
  } catch (e) {
    channels.delete(sessionId);
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, live: false, state: "idle" });
    throw e;
  }

  activate(sessionId);

  // A quiet Claude resume emits no init event.
  const after = entryOf(sessionId);
  if (after && !isTerminal(after.state) && !after.streaming) {
    setEntry(sessionId, {
      ...after,
      items: closeOpenAssistant(after.items),
      streaming: false,
      state: parkedState(after),
    });
    flushQueuedMessage(sessionId);
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

  const latest = entryOf(sessionId);
  if (latest) {
    const truncated = truncateAtUserOrdinal(latest, ordinal);
    setEntry(sessionId, {
      ...latest,
      ...truncated,
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
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current || current.permissionMode === mode) return;

  setEntry(sessionId, { ...current, permissionMode: mode, autoAllow: [] });
  if (!channels.has(sessionId)) return;
  await restartSession(sessionId, mode);
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
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  if (channels.has(sessionId) && isBusy(current)) {
    setEntry(sessionId, {
      ...current,
      queued: [...current.queued, { text, attachments, promptName }],
    });
    return;
  }
  await deliverMessage(sessionId, text, attachments, promptName);
}

async function deliverMessage(
  sessionId: string,
  text: string,
  attachments: Attachment[] = [],
  promptName: string | null = null,
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  const withUser = addUserMessage(current, text, null, attachments, Date.now(), promptName);
  const optimisticId = withUser.items[withUser.items.length - 1].id;
  setEntry(sessionId, { ...current, ...withUser });
  if (!channels.has(sessionId)) await resumeSession(sessionId);
  try {
    await unwrap(
      commands.sendUserMessage(
        sessionId,
        text,
        attachments.map((a) => a.path),
        promptName,
      ),
    );
  } catch (e) {
    const latest = entryOf(sessionId);
    if (latest) {
      setEntry(sessionId, { ...latest, items: latest.items.filter((it) => it.id !== optimisticId) });
    }
    throw e;
  }
}

function flushQueuedMessage(sessionId: string): void {
  const entry = entryOf(sessionId);
  if (!entry || entry.queued.length === 0 || isBusy(entry)) return;
  const [next, ...rest] = entry.queued;
  setEntry(sessionId, { ...entry, queued: rest });
  void deliverMessage(sessionId, next.text, next.attachments, next.promptName);
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
        active.mode ?? "general",
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
    const [entries, active] = await Promise.all([
      unwrap(commands.getSessionLog(sessionId)),
      // The server outlives the app (ADR-0009): a session it still reports
      // live is attached to below rather than shown as a frozen transcript.
      unwrap(commands.listActiveSessions()).catch((e) => {
        log.warn("session", "listing live sessions failed; restoring as if none were live", e);
        return [];
      }),
    ]);
    if (entryOf(sessionId) || channels.has(sessionId)) return;

    let view = initialSessionView("starting");
    for (const stored of entries) {
      try {
        view = reduceEvent(view, stored.event);
        const last = view.items[view.items.length - 1];
        if (stored.kind === "userMessage" && last?.kind === "user" && last.createdAt === null) {
          view = { ...view, items: [...view.items.slice(0, -1), { ...last, createdAt: stored.createdAt }] };
        }
      } catch (e) {
        log.warn("session", `restoring ${sessionId} skipped a ${stored.kind} event`, e);
      }
    }
    view = restoredView(view);
    setEntry(sessionId, {
      ...view,
      workstreamId,
      stage,
      autoAllow: [],
      live: false,
      permissionMode,
      workingSince: null,
      taskStartedAt: {},
      queued: [],
    });
    if (active.some((a) => a.session.id === sessionId)) await resumeSession(sessionId);
  } catch (error) {
    log.error("session", `restoring ${sessionId} failed`, error);
  } finally {
    restoring.delete(sessionId);
  }
}

export function transcriptToText(sessionId: string): string {
  const entry = useSessionStore.getState().sessions[sessionId];
  return entry ? viewToText(entry) : "(no transcript)";
}

export function useSession(sessionId: string | undefined): SessionEntry | undefined {
  return useSessionStore((s) => (sessionId ? s.sessions[sessionId] : undefined));
}

export type CreatedDoc = { path: string; kind: DocKind; title: string };

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
        const kind = k.slice(0, tab) as DocKind;
        const path = k.slice(tab + 1);
        return { kind, path, title: docTitle(path) };
      }),
    [keys],
  );
}

export function useMostRecentDoc(
  workstreamId: string | null,
  kind: DocKind | null,
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
    () => (key ? { kind: kind as DocKind, path: key, title: docTitle(key) } : null),
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

export type { WorkstreamActivity } from "@/lib/domain/activity";

const ACTIVITY_RANK: Record<WorkstreamActivity, number> = {
  idle: 0,
  review: 1,
  background: 2,
  error: 3,
  working: 4,
  "needs-you": 5,
};

export function classifyEntry(entry: SessionEntry): WorkstreamActivity {
  if (
    entry.state === "awaiting_permission" ||
    entry.state === "awaiting_question" ||
    hasPendingPermission(entry) ||
    entry.pendingQuestion !== null
  ) {
    return "needs-you";
  }
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
