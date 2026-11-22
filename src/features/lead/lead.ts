import { useEffect } from "react";
import { create } from "zustand";

import { events, type LeadView } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";
import { log } from "@/lib/log";
import { useUiStore } from "@/lib/uiStore";
import { useActiveSessions, useBootstrap, useLeads } from "@/lib/queries";
import { isStreamingSession, restoreSessionIfNeeded, resumeSession } from "@/features/session/sessionStore";
import { leadSettings, type Lead } from "@/features/settings/settingsPatches";

export type ProjectLead = { view: LeadView; settings: Lead };

export function useProjectLead(projectPath: string): ProjectLead | null {
  const appSettings = useBootstrap().data?.settings;
  const settings = appSettings ? leadSettings(appSettings) : null;
  const enabled = settings?.enabled === true;
  const view = useLeads(enabled).data?.find((l) => l.projectPath === projectPath);
  return enabled && settings && view ? { view, settings } : null;
}

// Resuming a session that isn't live relaunches it, so the stream attaches only once the server reports it live.
export function useFollowLead({ sessionId, workstreamId }: LeadView): void {
  const live = useActiveSessions().data?.some((a) => a.session.id === sessionId) ?? false;
  useEffect(() => {
    if (!sessionId || !workstreamId) return;
    restoreSessionIfNeeded(sessionId, workstreamId, "general")
      .then(() => (live && !isStreamingSession(sessionId) ? resumeSession(sessionId, { flush: false }) : undefined))
      .catch((e) => log.warn("lead", "following the Lead's session failed", e));
  }, [sessionId, workstreamId, live]);
}

export type LeadNote = { title: string; body: string };

const useLeadNotes = create<Record<string, LeadNote | undefined>>(() => ({}));

export function useLeadNote(workstreamId: string | null): LeadNote | null {
  return useLeadNotes((s) => (workstreamId ? (s[workstreamId] ?? null) : null));
}

export function clearLeadNote(workstreamId: string): void {
  useLeadNotes.setState({ [workstreamId]: undefined });
}

// notify_user and request_attention carry no session or terminal; an ask's notification names one.
export const leadNoteBridge: EventBridge = () => [
  events.uiRequested.listen(({ payload: { request } }) => {
    if (request.kind !== "notify" || request.sessionId || request.terminalId) return;
    useLeadNotes.setState({ [request.workstreamId]: { title: request.title, body: request.body } });
  }),
];

export function useLeadDockExpanded(projectPath: string): boolean {
  return useUiStore((s) => s.leadDockExpanded.includes(projectPath));
}

export function setLeadDockExpanded(projectPath: string, expanded: boolean): void {
  useUiStore.getState().setLeadDockExpanded(projectPath, expanded);
}
