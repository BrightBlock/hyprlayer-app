import { useEffect, useMemo } from "react";
import { create } from "zustand";

import { events, type LeadView, type WorkstreamView } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";
import { log } from "@/lib/log";
import { useUiStore } from "@/lib/uiStore";
import { useActiveSessions, useBootstrap, useLeads } from "@/lib/queries";
import { useFleet } from "@/features/dashboard/fleet";
import {
  isStreamingSession,
  restoreSessionIfNeeded,
  resumeSession,
  useSessionStore,
} from "@/features/session/sessionStore";
import type { PendingQuestion } from "@/features/session/transcript";
import { leadSettings, type Lead } from "@/features/settings/settingsPatches";

export type ProjectLead = { view: LeadView; settings: Lead };

function useEnabledLeadSettings(): Lead | null {
  const appSettings = useBootstrap().data?.settings;
  const settings = appSettings ? leadSettings(appSettings) : null;
  return settings?.enabled ? settings : null;
}

export function useProjectLead(projectPath: string): ProjectLead | null {
  const settings = useEnabledLeadSettings();
  const view = useLeads(settings !== null).data?.find((l) => l.projectPath === projectPath);
  return settings && view ? { view, settings } : null;
}

export function useLeadViews(): ReadonlyMap<string, LeadView> {
  const enabled = useEnabledLeadSettings() !== null;
  const leads = useLeads(enabled).data;
  return useMemo(
    () => new Map<string, LeadView>(enabled && leads ? leads.map((l) => [l.projectPath, l]) : []),
    [enabled, leads],
  );
}

export function mainHiddenByLead(view: LeadView | undefined): string | null {
  return view?.sessionId ? view.workstreamId : null;
}

export function useLeadNeedsYou(lead: LeadView, views: WorkstreamView[]): number {
  const fleet = useFleet(views);
  const mainId = mainHiddenByLead(lead);
  return fleet.live.filter(
    (l) => l.activity === "needs-you" && (l.view.item.id !== mainId || l.sessionId === lead.sessionId),
  ).length;
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

export type LeadAsk = {
  sessionId: string;
  question: PendingQuestion;
  subject: { view: WorkstreamView; archive: boolean } | null;
};

export function useLeadAsk(lead: ProjectLead | null, views: readonly WorkstreamView[]): LeadAsk | null {
  const sessionId = lead?.view.sessionId ?? null;
  const question = useSessionStore((s) => (sessionId ? (s.sessions[sessionId]?.pendingQuestion ?? null) : null));
  if (!lead || !sessionId || !question) return null;
  const named = question.subject;
  const view = named
    ? views.find((v) => v.item.id === named.workstreamId && v.item.id !== lead.view.workstreamId)
    : undefined;
  return { sessionId, question, subject: named && view ? { view, archive: named.archive } : null };
}

export type LeadNote = { title: string; body: string };

const useLeadNotes = create<Record<string, LeadNote | undefined>>(() => ({}));

export function useLeadNote(workstreamId: string | null): LeadNote | null {
  return useLeadNotes((s) => (workstreamId ? (s[workstreamId] ?? null) : null));
}

export function clearLeadNote(workstreamId: string): void {
  useLeadNotes.setState({ [workstreamId]: undefined });
}

export const leadNoteBridge: EventBridge = () => [
  events.uiRequested.listen(({ payload: { request } }) => {
    if (request.kind !== "notify" || !request.fromLead) return;
    useLeadNotes.setState({ [request.workstreamId]: { title: request.title, body: request.body } });
  }),
];

export function useLeadDockExpanded(projectPath: string): boolean {
  return useUiStore((s) => s.leadDockExpanded.includes(projectPath));
}

export function setLeadDockExpanded(projectPath: string, expanded: boolean): void {
  useUiStore.getState().setLeadDockExpanded(projectPath, expanded);
}
