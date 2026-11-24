import { useEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent, type ReactNode } from "react";

import { Icon } from "@/components/Icon";
import { Button, ErrorNote, IconButton, Spinner } from "@/components/ui";
import type { LeadView, WorkstreamView } from "@/lib/bindings";
import { openExternalUrl } from "@/lib/browser";
import { formatCompactAge } from "@/lib/format";
import { errorMessage } from "@/lib/ipc";
import { useMessageLead, useSetLeadRemoteControl, useWorkstreamSessions } from "@/lib/queries";
import { showFailure, useToast } from "@/lib/toast";
import { useReducedMotion } from "@/lib/useReducedMotion";
import { useSlowNow } from "@/lib/useSlowNow";
import { QuestionPrompt } from "@/features/session/QuestionPrompt";
import { answerQuestion, useSessionStore } from "@/features/session/sessionStore";
import { isWorking, type TranscriptItem } from "@/features/session/transcript";
import { nextDockPhase, type DockEvent, type DockPhase } from "./dockPhase";
import { LeadMark } from "./LeadChip";
import { LeadDockSplit } from "./LeadSplit";
import { clearLeadNote, useFollowLead, useLeadNote, type LeadAsk, type LeadNote, type ProjectLead } from "./lead";

function latestReply(items: readonly TranscriptItem[]): string | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item.kind === "assistant" && !item.parentId && item.text.trim()) return item.text.replace(/\s+/g, " ");
  }
  return null;
}

function tokenMs(name: string, fallback: number): number {
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || fallback;
}

const SHRINK_SHARE = 0.7;

export function LeadDock({
  lead,
  ask,
  expanded,
  chat,
  views,
  onExpand,
  onMinimize,
}: {
  lead: ProjectLead;
  ask: LeadAsk | null;
  expanded: boolean;
  chat: ReactNode;
  views: WorkstreamView[];
  onExpand: () => void;
  onMinimize: () => void;
}) {
  const { projectPath, sessionId, workstreamId } = lead.view;
  useFollowLead(lead.view);
  const now = useSlowNow();
  const reduce = useReducedMotion();
  const items = useSessionStore((s) => (sessionId ? s.sessions[sessionId]?.items : undefined));
  const endedAt = useSessionStore((s) => (sessionId ? (s.sessions[sessionId]?.lastTurn?.endedAt ?? null) : null));
  const lastActiveAt = useWorkstreamSessions(workstreamId ?? "", !!workstreamId).data?.find((s) => s.id === sessionId)
    ?.lastActiveAt;
  const working = useSessionStore((s) => {
    const entry = sessionId ? s.sessions[sessionId] : undefined;
    return entry ? isWorking(entry) : false;
  });
  const note = useLeadNote(workstreamId);
  const send = useMessageLead();
  const [draft, setDraft] = useState("");
  const [phase, setPhase] = useState<DockPhase>(expanded ? "open" : "closed");
  const [primed, setPrimed] = useState(true);
  const [heights, setHeights] = useState({ docked: 0, full: 0 });
  const docked = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const expandControl = useRef<HTMLButtonElement>(null);
  const focusChat = useRef(false);
  const returnFocus = useRef(false);

  const reply = useMemo(() => latestReply(items ?? []), [items]);
  const age = reply && !working ? formatCompactAge(endedAt ?? lastActiveAt ?? null, now) : "";

  const measure = () =>
    setHeights({
      docked: docked.current?.offsetHeight ?? 0,
      full: (docked.current?.offsetParent as HTMLElement | null)?.clientHeight ?? 0,
    });
  const go = (event: DockEvent) => {
    const next = nextDockPhase(phase, event);
    if (next === phase) return;
    if (next === "growing" || next === "shrinking") measure();
    if (next === "growing") {
      focusChat.current = true;
      onExpand();
    }
    if (phase === "shrinking" && next === "closed") {
      returnFocus.current = true;
      onMinimize();
    }
    setPrimed(false);
    setPhase(next);
  };
  const goRef = useRef(go);
  goRef.current = go;

  useEffect(() => {
    if (primed) return;
    let inner = 0;
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(() => setPrimed(true));
    });
    return () => {
      cancelAnimationFrame(outer);
      cancelAnimationFrame(inner);
    };
  }, [primed]);

  useEffect(() => {
    if (!primed) return;
    const gentle = tokenMs("--duration-gentle", 280);
    const wait = {
      growing: reduce ? 0 : gentle,
      fading: tokenMs(reduce ? "--duration-instant" : "--duration-quick", reduce ? 80 : 140),
      shrinking: reduce ? 0 : Math.round(gentle * SHRINK_SHARE),
      open: null,
      closed: null,
    }[phase];
    if (wait === null) return;
    const timer = window.setTimeout(() => goRef.current("settled"), wait);
    return () => window.clearTimeout(timer);
  }, [phase, primed, reduce]);

  useEffect(() => {
    goRef.current(expanded ? "restored" : "dropped");
  }, [expanded]);

  // The chat mounts once ProjectPage binds main, which can land after the grow ends.
  useEffect(() => {
    if (phase !== "open" || !focusChat.current) return;
    const composer = panel.current?.querySelector("textarea");
    if (!composer) return;
    composer.focus();
    focusChat.current = false;
  }, [phase, chat]);

  useEffect(() => {
    if (phase !== "closed" || !returnFocus.current) return;
    returnFocus.current = false;
    expandControl.current?.focus();
  }, [phase]);

  // Menus and pickers render in portals, so an Escape aimed at one never targets this panel.
  useEffect(() => {
    if (phase !== "open" && phase !== "growing") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const target = e.target as Node | null;
      if (target && target !== document.body && !panel.current?.contains(target)) return;
      goRef.current("minimize");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase]);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text || send.isPending) return;
    send.mutate(
      { projectPath, text },
      {
        onSuccess: () => {
          setDraft("");
          if (workstreamId) clearLeadNote(workstreamId);
        },
      },
    );
  };

  const atDockHeight = phase === "closed" || (phase === "growing" && !primed) || (phase === "shrinking" && primed);
  const heightCurve =
    reduce || phase === "open" || phase === "fading"
      ? "none"
      : phase === "shrinking"
        ? `height calc(var(--duration-gentle) * ${SHRINK_SHARE}) var(--ease-in)`
        : "height var(--duration-gentle) var(--ease-std)";
  const panelStyle: CSSProperties = {
    height: phase === "open" || phase === "fading" ? "100%" : `${atDockHeight ? heights.docked : heights.full}px`,
    transition: heightCurve,
  };
  const chevronStyle: CSSProperties = {
    transform: atDockHeight ? "rotate(180deg)" : "none",
    transition: heightCurve === "none" ? "none" : heightCurve.replace("height", "transform"),
  };
  const contentShown = phase === "open";
  const contentStyle: CSSProperties = reduce
    ? { opacity: contentShown ? 1 : 0, transition: "opacity var(--duration-instant) linear" }
    : {
        opacity: contentShown ? 1 : 0,
        transform: contentShown ? "none" : "translateY(4px)",
        transition: contentShown
          ? "opacity var(--duration-quick) var(--ease-out), transform var(--duration-quick) var(--ease-out)"
          : "opacity var(--duration-quick) var(--ease-in)",
      };

  const header = (expandedHeader: boolean) => (
    <div className="flex h-[34px] shrink-0 items-center gap-2.5 pr-1.5 pl-6">
      <button
        type="button"
        onClick={() => go(expandedHeader ? "minimize" : "expand")}
        disabled={!sessionId}
        className="flex min-w-0 flex-1 items-center gap-2.5 text-left disabled:cursor-default"
      >
        <LeadMark />
        <span className="text-ui font-semibold text-ink">Lead Engineer</span>
        <span className="min-w-0 flex-1 truncate text-meta text-ink-dim">
          {reply ? (
            <>
              {lead.view.summary ?? (
                <>
                  <span className="text-ink-4">Lead:</span> {reply}
                </>
              )}
              {age && <span className="text-ink-4">{` · ${age}`}</span>}
            </>
          ) : (
            "No replies yet"
          )}
        </span>
        {working && <Spinner size="xs" />}
      </button>
      <RemoteControl view={lead.view} />
      {sessionId && (
        <button
          ref={expandedHeader ? undefined : expandControl}
          type="button"
          aria-label={expandedHeader ? "Minimize the Lead" : "Expand the Lead"}
          aria-expanded={expandedHeader}
          title={expandedHeader ? "Minimize (Esc)" : "Expand"}
          onClick={() => go(expandedHeader ? "minimize" : "expand")}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded-control text-ink-dim hover:bg-edge/50 hover:text-ink"
        >
          <span className="flex" style={expandedHeader ? chevronStyle : { transform: "rotate(180deg)" }}>
            <Icon name="chevronDown" size="xs" />
          </span>
        </button>
      )}
    </div>
  );

  return (
    <>
      <div ref={docked} inert={phase !== "closed"} className="shrink-0 border-t border-edge bg-panel">
        {header(false)}
        <div className="px-6 pt-1 pb-3.5">
          {ask && (
            <QuestionPrompt
              key={ask.question.requestId}
              pending={ask.question}
              onSubmit={(answers) =>
                void answerQuestion(ask.sessionId, ask.question.requestId, answers).catch(
                  showFailure("Your answer didn't reach the Lead"),
                )
              }
            />
          )}
          <div className="rounded-card border border-edge bg-surface focus-within:border-ink-dim/70">
            {note && workstreamId && <NoteStrip note={note} workstreamId={workstreamId} />}
            <form onSubmit={submit} className="flex items-center gap-2.5 py-1.5 pr-2.5 pl-3.5">
              <input
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                aria-label="Message the Lead Engineer"
                placeholder="Ask the Lead to start, check or land work…"
                className="h-7 min-w-0 flex-1 bg-transparent text-body text-ink placeholder:text-ink-dim focus:outline-none"
              />
              <Button type="submit" variant="primary" size="sm" busy={send.isPending} disabled={!draft.trim()}>
                Send
              </Button>
            </form>
          </div>
          {send.error && (
            <div className="mt-2">
              <ErrorNote error={send.error} onDismiss={() => send.reset()} />
            </div>
          )}
        </div>
      </div>
      {phase !== "closed" && (
        <div
          ref={panel}
          style={panelStyle}
          className="absolute inset-x-0 bottom-0 z-20 flex flex-col overflow-hidden border-t border-edge bg-panel"
        >
          {header(true)}
          <div style={contentStyle} className="flex min-h-0 flex-1 flex-col">
            {note && workstreamId && <NoteStrip note={note} workstreamId={workstreamId} />}
            {/* SessionChat's transcript slides 36px up under its own header; this clip keeps it off the dock's. */}
            <LeadDockSplit projectPath={projectPath} views={views}>
              <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
                {chat ?? (
                  <div className="flex flex-1 items-center justify-center">
                    <Spinner />
                  </div>
                )}
              </div>
            </LeadDockSplit>
          </div>
        </div>
      )}
    </>
  );
}

function RemoteControl({ view }: { view: LeadView }) {
  const set = useSetLeadRemoteControl();
  const { projectPath, remoteControl, remoteControlUrl: url } = view;
  const toggle = (enabled: boolean) =>
    set.mutate(
      { projectPath, enabled },
      {
        onError: (e) =>
          useToast
            .getState()
            .show(`Couldn't turn Remote Control ${enabled ? "on" : "off"}`, errorMessage(e), { tone: "bad" }),
      },
    );
  const label = !remoteControl
    ? "Turn on Remote Control to reach this Lead from the Claude app"
    : url
      ? "Remote Control is on. Click to turn it off."
      : "Remote Control is on. The Claude app link appears when the Lead is running. Click to turn it off.";
  const copy = (link: string) =>
    void navigator.clipboard.writeText(link).then(() => useToast.getState().show("Remote Control link copied", link));

  return (
    <span className="flex shrink-0 items-center gap-1.5">
      {remoteControl && url && (
        <>
          <button
            type="button"
            title={url}
            onClick={() => void openExternalUrl(url)}
            className="text-meta text-ink-dim hover:text-ink hover:underline"
          >
            Open in Claude
          </button>
          <IconButton aria-label="Copy the Remote Control link" title="Copy link" onClick={() => copy(url)} className="h-6 w-6">
            <Icon name="copy" size="xs" />
          </IconButton>
        </>
      )}
      <IconButton
        active={remoteControl}
        aria-pressed={remoteControl}
        aria-label={label}
        title={label}
        disabled={set.isPending}
        onClick={() => toggle(!remoteControl)}
        className="h-6 w-6"
      >
        {set.isPending ? <Spinner size="xs" /> : <Icon name={remoteControl ? "leadRemote" : "leadLocal"} size="xs" />}
      </IconButton>
    </span>
  );
}

function NoteStrip({ note, workstreamId }: { note: LeadNote; workstreamId: string }) {
  return (
    <div className="flex items-center gap-2.5 border-b border-edge-2 py-2 pr-2 pl-3.5">
      <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-warn" />
      <span className="min-w-0 flex-1 text-body text-ink-2">
        <span className="font-semibold text-ink">{note.title}</span> {note.body}
      </span>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={() => clearLeadNote(workstreamId)}
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-control text-ink-dim hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus-visible:text-ink"
      >
        <Icon name="close" size="xs" />
      </button>
    </div>
  );
}
