import { isTerminalBound } from "@/features/multiplexer/multiplexerStore";
import { commands, events, type ClaudeProbe, type Harness } from "@/lib/bindings";
import type { UiRequest } from "@/lib/domain/agentControl";
import { claudeModelKey, claudeModels } from "@/lib/domain/models";
import type { EventBridge } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { findWorkstreamForNotify, notifyFromAgent, notifyNeedsAttention } from "@/lib/notifications";
import { basename } from "@/lib/path";
import { queryClient } from "@/lib/queryClient";

import { isStreamingSession } from "./sessionStore";
import {
  openAgentSessionTab,
  openAgentTerminalTab,
  openSessionTab,
  openViewerTabInBackground,
  type LaunchedAgent,
} from "./workstreamTabs";

async function notify(request: Extract<UiRequest, { kind: "notify" }>): Promise<void> {
  if (request.sessionId && isStreamingSession(request.sessionId)) return;
  if (request.terminalId && isTerminalBound(request.terminalId)) return;
  const ws = findWorkstreamForNotify(request.workstreamId);
  if (!ws) return;
  // A session's ask: worded as the desktop streaming it words its own, so both banners match.
  if (request.sessionId) {
    const sessions = await unwrap(commands.listSessions(request.workstreamId)).catch(() => []);
    const asking = sessions.find((s) => s.id === request.sessionId);
    if (asking) {
      await notifyNeedsAttention({
        workstreamId: request.workstreamId,
        projectPath: ws.projectPath,
        workstreamTitle: ws.title,
        stage: asking.stage,
      });
      return;
    }
  }
  await notifyFromAgent({
    workstreamId: request.workstreamId,
    projectPath: ws.projectPath,
    workstreamTitle: ws.title,
    title: request.title,
    body: request.body,
  });
}

type OpenSessionTab = Extract<UiRequest, { kind: "openSessionTab" }>;

// A claude tab stores a catalog key, where the launch named a flag or an id.
function launchedAgent(harness: Harness, request: OpenSessionTab): LaunchedAgent {
  const model =
    request.model && harness === "claude"
      ? claudeModelKey(request.model, claudeModels(queryClient.getQueryData<ClaudeProbe>(["claude-probe"])?.models))
      : (request.model ?? null);
  return { model, effort: request.effort ?? null };
}

// Opening only adds a tab: SessionChat attaches the stream when the tab is first shown.
async function openSpawned(request: OpenSessionTab): Promise<void> {
  const sessions = await unwrap(commands.listSessions(request.workstreamId));
  const session = sessions.find((s) => s.id === request.sessionId);
  if (!session) return;
  const permissionMode = request.permissionMode ?? "default";
  const launched = launchedAgent(session.harness, request);
  if (request.focus) openSessionTab(request.workstreamId, session, { permissionMode, launched });
  else openAgentSessionTab(request.workstreamId, session, permissionMode, launched);
}

/** Startup and reconnect: a desktop that missed an agent's spawn opens it now, unless it was closed here. */
export async function reconcileAgentTabs(): Promise<void> {
  const active = await unwrap(commands.listActiveSessions());
  for (const { session } of active) {
    if (session.origin !== "agent" || session.archivedAt != null) continue;
    // No event carries the launch mode here; Default never relaunches it into bypass or accept-edits.
    openAgentSessionTab(session.workstreamId, session, "default");
  }
}

function reveal({ workstreamId, target }: Extract<UiRequest, { kind: "reveal" }>): void {
  const title = basename(target.path);
  switch (target.kind) {
    case "doc":
      openViewerTabInBackground(workstreamId, { docPreviewPath: target.path }, title.replace(/\.md$/, ""));
      return;
    case "file":
      openViewerTabInBackground(workstreamId, { filePath: target.path }, title);
      return;
    case "diff":
      openViewerTabInBackground(workstreamId, { diffView: { path: target.path, staged: false } }, title);
  }
}

function onUiRequested(request: UiRequest): void {
  switch (request.kind) {
    case "notify":
      notify(request).catch((e) => log.warn("agent-control", "an agent's notification failed", e));
      return;
    case "openSessionTab":
      openSpawned(request).catch((e) => log.warn("agent-control", "an agent's session did not open", e));
      return;
    case "openTerminalTab":
      openAgentTerminalTab(request.workstreamId, {
        terminalId: request.terminalId,
        engineId: request.engineId,
        sessionId: request.providerSessionId ?? undefined,
        bypass: request.bypass,
      });
      return;
    case "reveal":
      reveal(request);
  }
}

/** Every attached desktop hears an agent's ask; each one shows it. */
export const agentControlBridge: EventBridge = () => [
  events.uiRequested.listen(({ payload }) => onUiRequested(payload.request)),
];
