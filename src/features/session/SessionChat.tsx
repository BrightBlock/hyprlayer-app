import { useEffect, useState, type KeyboardEvent, type ReactNode } from "react";

import { Badge, Button, ErrorNote, Spinner } from "../../components/ui";
import {
  ChevronDownIcon,
  EffortIcon,
  ModeIcon,
  PlusIcon,
  SendIcon,
  SparkleIcon,
  StopIcon,
} from "../../components/icons";
import { errorMessage } from "../../lib/ipc";
import type { WorkstreamView } from "../../lib/bindings";
import { WorkstreamDocChips } from "../workstreams/WorkstreamDocChips";
import type { ShellTab } from "../../routes/ProjectPage";
import { MODE_STAGES, MODE_STYLES, asStageKey, stageLabel, type StageKey } from "./stages";
import { EFFORTS, MODELS, effortOption, modelOption } from "./models";
import { formatCost, formatTokens } from "./toolDisplay";
import { TranscriptView, type PermissionHandlers } from "./TranscriptView";
import {
  allowForSession,
  respondPermission,
  restoreSessionIfNeeded,
  sendMessage,
  startSession,
  stopSession,
  useSession,
} from "./sessionStore";
import { describeState, isBusy, isTerminal, type SessionState } from "./transcript";

const STAGE_INTRO: Record<StageKey, string> = {
  general: "Chat with the project agent in this worktree. Pick a stage below to run a spec-driven step.",
  research: "Runs /research_codebase to investigate the code and write a research doc into the library.",
  plan: "Runs /create_plan to turn research into a concrete, reviewable implementation plan.",
  implement: "Runs /implement_plan to make the changes in this worktree, committing as it goes.",
  validate: "Runs /validate_plan to check the work against the plan and the project's tests.",
  ship: "Runs /commit, then helps push the branch and open a pull request.",
};

/** The live session-state chip shown in the workstream header for a tab. */
export function SessionStatusBadge({ sessionId }: { sessionId?: string }) {
  const session = useSession(sessionId);
  if (!session) return <Badge tone="dim">idle</Badge>;
  const { label, tone } = describeState(session.state, session.streaming);
  return <Badge tone={tone}>{label}</Badge>;
}

const DOT_COLOR: Record<ReturnType<typeof describeState>["tone"], string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
  dim: "bg-ink-dim/50",
  accent: "bg-sky-400",
};

/** A tiny live status dot for an agent tab (none until a session is started). */
export function SessionTabDot({ sessionId }: { sessionId?: string }) {
  const session = useSession(sessionId);
  if (!session) return null;
  const { tone } = describeState(session.state, session.streaming);
  const pulse = session.streaming || session.state === "starting";
  return (
    <span
      className={`ml-0.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full ${DOT_COLOR[tone]} ${pulse ? "animate-pulse" : ""}`}
      aria-hidden="true"
    />
  );
}

type MenuItem = { key: string; label: string; hint?: string; icon?: ReactNode };

/** A compact composer-toolbar dropdown (model / effort / mode). Opens upward —
 *  the composer sits at the bottom of the pane. */
function ComposerMenu({
  leading,
  label,
  hint,
  items,
  activeKey,
  onSelect,
  title,
}: {
  leading: ReactNode;
  label: string;
  hint?: string;
  items: MenuItem[];
  activeKey: string;
  onSelect: (key: string) => void;
  title: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={title}
        className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-ink-dim hover:bg-edge/50 hover:text-ink"
      >
        <span className="shrink-0">{leading}</span>
        <span className="font-medium text-ink">{label}</span>
        {hint && <span className="text-ink-dim">{hint}</span>}
        <ChevronDownIcon size={12} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={() => setOpen(false)} aria-hidden="true" />
          <div className="absolute bottom-full left-0 z-30 mb-1.5 min-w-[190px] overflow-hidden rounded-lg border border-edge bg-panel py-1 shadow-xl">
            {items.map((it) => (
              <button
                key={it.key}
                type="button"
                onClick={() => {
                  onSelect(it.key);
                  setOpen(false);
                }}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/50 ${
                  it.key === activeKey ? "text-ink" : "text-ink-dim"
                }`}
              >
                {it.icon && <span className="shrink-0">{it.icon}</span>}
                <span className="font-medium">{it.label}</span>
                {it.hint && <span className="text-ink-dim">{it.hint}</span>}
                {it.key === activeKey && <span className="ml-auto text-[10px] text-ink-dim">●</span>}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

/** A ring whose arc fills with `pct` (0–1), greening → amber → red as it fills. */
function ContextRing({ pct }: { pct: number }) {
  const r = 7;
  const circumference = 2 * Math.PI * r;
  const filled = Math.min(1, Math.max(0, pct));
  const tone = filled > 0.9 ? "text-bad" : filled > 0.75 ? "text-warn" : "text-ok";
  return (
    <svg width={18} height={18} viewBox="0 0 18 18" className="-rotate-90">
      <circle cx="9" cy="9" r={r} fill="none" strokeWidth="2.5" stroke="currentColor" className="text-edge" />
      <circle
        cx="9"
        cy="9"
        r={r}
        fill="none"
        strokeWidth="2.5"
        stroke="currentColor"
        strokeLinecap="round"
        strokeDasharray={circumference}
        strokeDashoffset={circumference * (1 - filled)}
        className={tone}
      />
    </svg>
  );
}

/** Context-window usage: a ring in the toolbar, click for the Context popover. */
function ContextMeter({ tokens, windowSize }: { tokens: number; windowSize: number }) {
  const [open, setOpen] = useState(false);
  const pct = Math.min(1, tokens / windowSize);
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={`Context ${formatTokens(tokens)} / ${formatTokens(windowSize)}`}
        aria-label="Context usage"
        className="flex h-8 items-center gap-1.5 rounded-md px-2 text-ink-dim hover:bg-edge/50 hover:text-ink"
      >
        <ContextRing pct={pct} />
        <span className="font-mono text-[11px]">{formatTokens(tokens)}</span>
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={() => setOpen(false)} aria-hidden="true" />
          <div className="absolute bottom-full right-0 z-30 mb-1.5 w-64 rounded-lg border border-edge bg-panel p-3 shadow-xl">
            <div className="flex items-baseline justify-between">
              <span className="text-sm font-medium text-ink">Context</span>
              <span className="font-mono text-xs text-ink-dim">
                {formatTokens(tokens)} / {formatTokens(windowSize)}
              </span>
            </div>
            <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-edge">
              <div className="h-full rounded-full bg-ink-dim" style={{ width: `${pct * 100}%` }} />
            </div>
            <dl className="mt-2.5 space-y-1 text-xs">
              <div className="flex justify-between">
                <dt className="text-ink-dim">Used</dt>
                <dd className="text-ink">{(pct * 100).toFixed(1)}%</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-ink-dim">Free space</dt>
                <dd className="text-ink">{((1 - pct) * 100).toFixed(1)}%</dd>
              </div>
            </dl>
          </div>
        </>
      )}
    </div>
  );
}

type Props = {
  view: WorkstreamView;
  shell: ShellTab;
  onDraftChange: (value: string) => void;
  onModeChange: (mode: StageKey) => void;
  onModelChange: (key: string) => void;
  onEffortChange: (key: string) => void;
  onSessionStarted: (sessionId: string) => void;
  onResetSession: () => void;
  onOpenDoc: (id: string) => void;
  onDetachDoc: (path: string) => void;
  onBrowseDocs: () => void;
};

export function SessionChat({
  view,
  shell,
  onDraftChange,
  onModeChange,
  onModelChange,
  onEffortChange,
  onSessionStarted,
  onResetSession,
  onOpenDoc,
  onDetachDoc,
  onBrowseDocs,
}: Props) {
  const session = useSession(shell.sessionId);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const started = Boolean(shell.sessionId);
  const hasEntry = Boolean(session);
  const mode = asStageKey(shell.mode);
  const model = modelOption(shell.model);
  const effort = effortOption(shell.effort);

  // A tab bound to a session but with no transcript in memory means we came
  // back after an app restart — rebuild it from the persisted event log.
  useEffect(() => {
    if (shell.sessionId && !hasEntry) {
      void restoreSessionIfNeeded(shell.sessionId, view.item.id, shell.mode);
    }
  }, [shell.sessionId, hasEntry, view.item.id, shell.mode]);

  const busy = session ? isBusy(session) : started;
  const terminal = session ? isTerminal(session.state) : false;
  const draft = shell.draft;
  const canSubmit = draft.trim().length > 0;
  const sid = shell.sessionId;

  const handleStart = async () => {
    const input = draft.trim();
    if (!input || starting) return;
    setStarting(true);
    setError(null);
    try {
      const created = await startSession({
        workstreamId: view.item.id,
        stage: shell.mode,
        input,
        attachedDocs: shell.docs.map((d) => d.path),
        model: model.flag,
        effort: effort.key,
      });
      onSessionStarted(created.id);
      onDraftChange("");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  const handleSend = async () => {
    const text = draft.trim();
    if (!text || !sid || busy) return;
    setError(null);
    try {
      await sendMessage(sid, text);
      onDraftChange("");
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!started) void handleStart();
      else void handleSend();
    }
  };

  const permission: PermissionHandlers = {
    onAllow: (rid) => sid && void respondPermission(sid, rid, true, null),
    onAllowForSession: (rid, tool) => sid && void allowForSession(sid, rid, tool),
    onDeny: (rid) => sid && void respondPermission(sid, rid, false, null),
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {started ? (
        <TranscriptView
          items={session?.items ?? []}
          busy={busy}
          connecting={!session || session.state === "starting"}
          permission={permission}
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          <div className="mx-auto w-4/5">
            <div className="rounded-lg border border-edge bg-panel p-4">
              <div className="mb-2 flex items-center gap-2">
                <span className={`rounded p-0.5 ${MODE_STYLES[mode].badge}`}>
                  <ModeIcon mode={mode} />
                </span>
                <span className="text-sm font-medium text-ink">{stageLabel(mode)} agent</span>
              </div>
              <p className="text-sm leading-relaxed text-ink-dim">{STAGE_INTRO[mode]}</p>
            </div>
          </div>
        </div>
      )}

      <div className="border-t border-edge bg-panel p-4">
        <div className="mx-auto w-4/5">
          {session && (
            <div className="mb-2 flex min-w-0 items-center gap-2 text-xs text-ink-dim">
              {session.model && (
                <span className="truncate font-mono" title={session.model}>
                  {session.model}
                </span>
              )}
              <SessionInlineState state={session.state} streaming={session.streaming} />
              {session.costUsd !== null && <span>· {formatCost(session.costUsd)}</span>}
              {session.numTurns !== null && (
                <span>
                  · {session.numTurns} {session.numTurns === 1 ? "turn" : "turns"}
                </span>
              )}
            </div>
          )}

          {error && (
            <div className="mb-2">
              <ErrorNote error={{ code: "session", message: error, remedy: null }} />
            </div>
          )}

          {terminal ? (
            <div className="flex items-center justify-between gap-3 rounded-md border border-edge bg-panel-2 px-3 py-2.5">
              <span className="text-sm text-ink-dim">
                This session has ended. Start a new one to keep working in this tab.
              </span>
              <Button variant="primary" onClick={onResetSession}>
                New session
              </Button>
            </div>
          ) : (
            <>
              {!started && (
                <WorkstreamDocChips
                  docs={shell.docs}
                  onOpenDoc={onOpenDoc}
                  onDetach={onDetachDoc}
                  onBrowse={onBrowseDocs}
                />
              )}
              <div className="rounded-xl border border-edge bg-panel-2 transition-colors focus-within:border-ink-dim/40">
                <textarea
                  value={draft}
                  onChange={(e) => onDraftChange(e.target.value)}
                  onKeyDown={onKeyDown}
                  rows={3}
                  placeholder={
                    !started
                      ? mode === "general"
                        ? "Ask the project agent…  (Enter to start, Shift+Enter for a newline)"
                        : `Brief the ${stageLabel(mode).toLowerCase()} agent…  (Enter to start)`
                      : "Reply to the agent…  (Enter to send)"
                  }
                  className="selectable block w-full resize-none bg-transparent px-3.5 py-3 text-sm text-ink placeholder:text-ink-dim/60 focus:outline-none"
                />
                <div className="flex items-center gap-0.5 px-2 pb-2">
                  {/* Mode is fixed at spawn → a badge once running; a picker before. */}
                  {started ? (
                    <span
                      className={`inline-flex items-center gap-1.5 rounded px-2 py-1 text-xs font-medium ${MODE_STYLES[mode].badge}`}
                    >
                      <ModeIcon mode={mode} size={13} />
                      {stageLabel(mode)}
                    </span>
                  ) : (
                    <ComposerMenu
                      title="Agent mode"
                      leading={<ModeIcon mode={mode} size={13} />}
                      label={stageLabel(mode)}
                      items={MODE_STAGES.map((s) => ({
                        key: s.key,
                        label: s.label,
                        icon: <ModeIcon mode={s.key} size={13} />,
                      }))}
                      activeKey={mode}
                      onSelect={(k) => onModeChange(k as StageKey)}
                    />
                  )}
                  {/* Model + effort stay visible/selectable; mid-session changes
                      take effect on the next session this tab starts. */}
                  <ComposerMenu
                    title="Model"
                    leading={<SparkleIcon size={14} />}
                    label={model.label}
                    hint={model.hint}
                    items={MODELS.map((m) => ({ key: m.key, label: m.label, hint: m.hint }))}
                    activeKey={model.key}
                    onSelect={onModelChange}
                  />
                  <ComposerMenu
                    title="Reasoning effort"
                    leading={<EffortIcon size={14} />}
                    label={effort.label}
                    items={EFFORTS.map((e) => ({ key: e.key, label: e.label }))}
                    activeKey={effort.key}
                    onSelect={onEffortChange}
                  />

                  <div className="ml-auto flex items-center gap-1">
                    {!started && (
                      <button
                        type="button"
                        onClick={onBrowseDocs}
                        title="Attach context docs"
                        aria-label="Attach context docs"
                        className="flex h-8 w-8 items-center justify-center rounded-md text-ink-dim hover:bg-edge/50 hover:text-ink"
                      >
                        <PlusIcon />
                      </button>
                    )}
                    {session?.contextTokens != null && (
                      <ContextMeter tokens={session.contextTokens} windowSize={model.contextWindow} />
                    )}
                    {busy && started ? (
                      <button
                        type="button"
                        onClick={() => sid && void stopSession(sid)}
                        title="Stop the current turn (interrupt, then force-stop if it doesn't respond)"
                        aria-label="Stop"
                        className="flex h-8 w-8 items-center justify-center rounded-md border border-bad/40 bg-bad/10 text-bad hover:bg-bad/20"
                      >
                        <StopIcon />
                      </button>
                    ) : (
                      <button
                        type="button"
                        disabled={!canSubmit || starting}
                        onClick={() => (started ? void handleSend() : void handleStart())}
                        title={started ? "Send" : "Start session"}
                        aria-label={started ? "Send" : "Start"}
                        className="flex h-8 w-8 items-center justify-center rounded-md border border-edge bg-edge/70 text-ink hover:bg-edge disabled:opacity-40 disabled:hover:bg-edge/70"
                      >
                        {starting ? <Spinner size={14} /> : <SendIcon size={15} />}
                      </button>
                    )}
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function SessionInlineState({ state, streaming }: { state: SessionState; streaming: boolean }) {
  const { label } = describeState(state, streaming);
  return <span>{label}</span>;
}
