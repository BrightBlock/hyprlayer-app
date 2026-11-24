import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";

import { Button } from "@/components/ui";
import type { BinaryStatus, Project, WorkstreamView } from "@/lib/bindings";
import { useEscape, runCommand } from "@/lib/keybindings/registry";
import { openSettings } from "@/lib/appNav";
import { useArchivedWorkstreams, useBootstrap, useConnectionStatus, useWorkstreams } from "@/lib/queries";
import { useUiStore } from "@/lib/uiStore";
import { offerLeadDraft, setLeadDockExpanded, useProjectLead } from "@/features/lead/lead";
import { focusShell, updateWorkstreamTabs, useWorkstreamTabsStore } from "@/features/session/workstreamTabs";
import { useSessionStore } from "@/features/session/sessionStore";
import { chordText, CoachMark } from "./CoachMark";
import { taskWorkstream, useSignal } from "./signals";
import type { Need, Reveal } from "./steps";
import {
  activeTour,
  endTour,
  nextStep,
  pauseTour,
  prevStep,
  setTaskSentAt,
  setTourProject,
  setTourWorkstream,
  skipStep,
  useTourStore,
  type ActiveTour,
} from "./tourStore";
import { useAnchor } from "./useAnchor";

const ENGINES = new Set(["claude", "codex", "opencode"]);

export function hasEngine(binaries: readonly BinaryStatus[] | undefined): boolean {
  return binaries === undefined || binaries.some((b) => ENGINES.has(b.kind) && b.state.state === "ok");
}

export function routeProject(pathname: string): string | null {
  const m = pathname.match(/^\/projects\/([^/?#]+)/);
  return m ? decodeURIComponent(m[1]) : null;
}

/** The project the tour points at: the one on screen, else the one the tour opened, else the first. */
export function tourProject(
  onScreen: string | null,
  remembered: string | null,
  projects: readonly Project[] | undefined,
): string | null {
  const known = (p: string | null) => (p && projects?.some((x) => x.path === p) ? p : null);
  return known(onScreen) ?? known(remembered) ?? projects?.[0]?.path ?? null;
}

export type Unmet = { need: Need; notice: string };

export function firstUnmet(
  needs: readonly Need[],
  facts: { server: boolean; project: boolean; lead: boolean; engine: boolean; leadPanel: boolean },
): Unmet | null {
  const NOTICE: Record<Need, string> = {
    server: "The server isn’t answering. The tour waits here and picks up when it’s back.",
    project: "This step needs a project, and none is open yet.",
    lead: "This project has no Lead Engineer yet. Turn it on under Settings → Agent control.",
    engine: "The Lead spawns agents on your default engine, and none is installed on this server yet.",
    leadPanel: "This step points at the Lead panel, which you’ve hidden.",
  };
  const need = needs.find((n) => !facts[n]);
  return need ? { need, notice: NOTICE[need] } : null;
}

/** Mounted once in the app shell: renders the active step's coach mark, if a tour is running. */
export function TourHost() {
  // A string, so the selector is stable; the tour itself is rebuilt from it.
  const key = useTourStore((s) => {
    const t = activeTour(s);
    return t ? `${t.path}:${t.step.id}` : null;
  });
  const tour = key ? activeTour(useTourStore.getState()) : null;
  if (!tour) return null;
  return <ActiveStep key={key} tour={tour} />;
}

function ActiveStep({ tour }: { tour: ActiveTour }) {
  const { step, path, index, steps } = tour;
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const openItem = useRouterState({ select: (s) => (s.location.search as { item?: unknown }).item ?? null });
  const bootstrap = useBootstrap().data;
  const connection = useConnectionStatus().data;
  const remembered = useTourStore((s) => s.projectPath);
  const tourWorkstreamId = useTourStore((s) => s.tourWorkstreamId);
  const projectPath = tourProject(routeProject(pathname), remembered, bootstrap?.projects);
  const lead = useProjectLead(projectPath ?? "");
  const workstreams = useWorkstreams(projectPath ?? "").data;
  const leadPanelHidden = useUiStore((s) => s.leadPanelHidden);
  const leadAsking = useSessionStore((s) =>
    lead?.view.sessionId ? s.sessions[lead.view.sessionId]?.pendingQuestion != null : false,
  );
  const tabs = useWorkstreamTabsStore((s) => (tourWorkstreamId ? s.byWs[tourWorkstreamId] : undefined));
  const overrides = bootstrap?.settings.keybindings?.overrides ?? {};
  const [attempt, setAttempt] = useState(0);

  const unmet = firstUnmet(step.needs, {
    server: connection === undefined || connection.state === "connected",
    project: projectPath !== null,
    lead: lead !== null,
    engine: hasEngine(bootstrap?.binaries),
    leadPanel: !leadPanelHidden,
  });

  const signal = useSignal(step.tryIt?.signal ?? null, {
    projectPath,
    projects: bootstrap?.projects,
    lead: lead?.view ?? null,
    workstreams: workstreams?.filter((w) => w.item.id !== lead?.view.workstreamId),
    leadAsking,
    tourWorkstreamId,
  });

  useEffect(() => {
    if (signal.project) setTourProject(signal.project);
    if (signal.sentAt !== undefined) setTaskSentAt(signal.sentAt);
    else if (signal.workstream) setTourWorkstream(signal.workstream);
  }, [signal.project, signal.workstream, signal.sentAt]);

  useTaskWorkstream(path === "firstRun", projectPath, lead?.view.workstreamId ?? null, workstreams);
  const taskGone =
    tourWorkstreamId !== null && workstreams !== undefined && !workstreams.some((w) => w.item.id === tourWorkstreamId);
  const agentFinished = step.reveal === "tourAgentTab" && taskGone;

  const reveal = useRef<(r: Reveal) => void>(() => {});
  reveal.current = (r: Reveal) => {
    if (!projectPath || r === "none") return;
    if (r === "tourAgentTab") {
      if (!tourWorkstreamId) return;
      void navigate({ to: "/projects/$projectPath", params: { projectPath }, search: { item: tourWorkstreamId } });
      const shell = tabs?.shells.find((s) => s.ptyAgent || s.sessionId);
      if (shell) {
        updateWorkstreamTabs((all) => {
          const ws = all[tourWorkstreamId];
          return ws ? { ...all, [tourWorkstreamId]: focusShell(ws, shell.id) } : all;
        });
      }
      return;
    }
    if (routeProject(pathname) !== projectPath || openItem !== null) {
      void navigate({ to: "/projects/$projectPath", params: { projectPath }, search: {} });
    }
    setLeadDockExpanded(projectPath, r === "leadExpanded");
  };

  const blocked = unmet !== null || agentFinished;
  useEffect(() => {
    if (!blocked) reveal.current(step.reveal);
  }, [blocked, step.reveal, attempt]);

  const anchor = useAnchor(blocked ? null : step.anchor, attempt);
  useEscape(pauseTour);
  useF6BackToCoach();

  const done = signal.done;
  const canAdvance = !step.tryIt || done;

  const fix = fixFor(unmet, {
    retry: () => setAttempt((a) => a + 1),
    showPanel: () => useUiStore.getState().toggleLeadPanel(),
  });
  const shortcut = chordText(overrides, "view.toggleLeadPanel");

  let notice: ReactNode = unmet?.notice;
  let anchored = anchor.status === "found" ? anchor.rect : null;
  if (agentFinished) {
    notice = "The agent has already finished, and the Lead archived its workstream, so its tab is closed.";
  } else if (!unmet && anchor.status === "missing" && !step.anchorOptional) {
    notice = "What this step points at isn’t on screen.";
    anchored = null;
  }
  if (unmet?.need === "leadPanel" && shortcut) notice = `${unmet.notice} Press ${shortcut} to show it.`;
  if (anchor.status === "searching" && !unmet) return null;

  return (
    <CoachMark
      step={step}
      path={path}
      index={index}
      total={steps.length}
      anchor={anchored}
      notice={notice}
      fix={
        fix ??
        (!blocked && anchor.status === "missing" && !step.anchorOptional ? (
          <Button size="sm" onClick={() => setAttempt((a) => a + 1)}>
            Take me there
          </Button>
        ) : undefined)
      }
      tryState={step.tryIt ? { kind: done ? "done" : "waiting" } : undefined}
      canAdvance={canAdvance}
      overrides={overrides}
      onNext={nextStep}
      onBack={prevStep}
      onSkipStep={skipStep}
      onPause={pauseTour}
      onEndTour={() => endTour(path)}
      onUsePrompt={
        step.tryIt?.prompt && projectPath ? () => offerLeadDraft(projectPath, step.tryIt!.prompt!) : undefined
      }
      onFocusAnchor={
        anchor.status === "found" ? () => focusInside(anchor.element) : undefined
      }
    />
  );
}

/**
 * Adopts the workstream the Lead made for the first-run task, whichever step is showing: a fast Lead can
 * spawn, finish and archive it while the user is still reading the step that sent it.
 */
function useTaskWorkstream(
  firstRun: boolean,
  projectPath: string | null,
  leadWorkstreamId: string | null,
  live: readonly WorkstreamView[] | undefined,
) {
  const sentAt = useTourStore((s) => s.taskSentAt);
  const known = useTourStore((s) => s.tourWorkstreamId);
  const looking = firstRun && sentAt !== null && known === null;
  const archived = useArchivedWorkstreams(looking).data;
  useEffect(() => {
    if (!looking || sentAt === null) return;
    const candidates = [
      ...(live ?? []).map((w) => w.item),
      ...(archived ?? []).map((w) => w.item).filter((w) => w.projectPath === projectPath),
    ];
    const id = taskWorkstream(sentAt, leadWorkstreamId, candidates);
    if (id) setTourWorkstream(id);
  }, [looking, sentAt, leadWorkstreamId, live, archived, projectPath]);
}

/** F6 from anywhere in the app returns to the coach mark; from the coach mark it goes to the anchor. */
function useF6BackToCoach() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "F6") return;
      const coach = document.querySelector<HTMLElement>("[data-tour-coach]");
      if (!coach || coach.contains(document.activeElement)) return;
      e.preventDefault();
      coach.querySelector<HTMLElement>("h2")?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

function focusInside(el: HTMLElement) {
  const target = el.matches("button, input, textarea, [tabindex]")
    ? el
    : el.querySelector<HTMLElement>("button, input, textarea, a[href], [tabindex]:not([tabindex='-1'])");
  target?.focus();
}

function fixFor(unmet: Unmet | null, actions: { retry: () => void; showPanel: () => void }): ReactNode {
  switch (unmet?.need) {
    case "project":
      return (
        <Button size="sm" onClick={() => runCommand("project.open")}>
          Open a project…
        </Button>
      );
    case "engine":
      return (
        <Button size="sm" onClick={() => openSettings("binaries")}>
          Open Settings → Binaries
        </Button>
      );
    case "lead":
      return (
        <Button size="sm" onClick={() => openSettings("agentControl")}>
          Open Settings → Agent control
        </Button>
      );
    case "leadPanel":
      return (
        <Button size="sm" onClick={actions.showPanel}>
          Show the Lead panel
        </Button>
      );
    case "server":
      return (
        <Button size="sm" onClick={actions.retry}>
          Try again
        </Button>
      );
    default:
      return null;
  }
}
