import { useEffect, useState, type KeyboardEvent } from "react";

import { Badge, Button, ErrorNote } from "../../components/ui";
import { ModeIcon, SendIcon, StopIcon } from "../../components/icons";
import { errorMessage } from "../../lib/ipc";
import type { WorkstreamView } from "../../lib/bindings";
import { WorkstreamDocChips } from "../workstreams/WorkstreamDocChips";
import type { ShellTab } from "../../routes/ProjectPage";
import { MODE_STYLES, STAGES, asStageKey, stageLabel, type StageKey } from "./stages";
import { formatCost } from "./toolDisplay";
import { TranscriptView, type PermissionHandlers } from "./TranscriptView";
import {
  allowForSession,
  interruptSession,
  killSession,
  respondPermission,
  restoreSessionIfNeeded,
  sendMessage,
  startSession,
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

type Props = {
  view: WorkstreamView;
  shell: ShellTab;
  onDraftChange: (value: string) => void;
  onModeChange: (mode: StageKey) => void;
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
          <div className="mx-auto max-w-3xl">
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
        <div className="mx-auto max-w-3xl">
          {session && (
            <div className="mb-2 flex items-center justify-between gap-2 text-xs text-ink-dim">
              <div className="flex min-w-0 items-center gap-2">
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
              {!terminal && sid && (
                <button
                  type="button"
                  onClick={() => void killSession(sid)}
                  className="shrink-0 text-ink-dim hover:text-bad"
                >
                  End session
                </button>
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
                className={`selectable w-full resize-none rounded-md border bg-panel-2 px-3 py-2 text-sm text-ink placeholder:text-ink-dim/70 focus:outline-none ${MODE_STYLES[mode].composer}`}
              />
              <div className="mt-2 flex items-center justify-between gap-3">
                {!started ? (
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="shrink-0 text-xs text-ink-dim">Mode</span>
                    <div className="flex flex-wrap gap-1">
                      {STAGES.map((s) => (
                        <button
                          key={s.key}
                          type="button"
                          onClick={() => onModeChange(s.key)}
                          className={`inline-flex h-8 w-8 items-center justify-center rounded-md border ${
                            s.key === mode
                              ? MODE_STYLES[s.key].activeButton
                              : "border-edge bg-panel-2 text-ink-dim hover:bg-edge/60 hover:text-ink"
                          }`}
                          title={`${s.label} mode`}
                          aria-label={`${s.label} mode`}
                        >
                          <ModeIcon mode={s.key} />
                        </button>
                      ))}
                    </div>
                    <span className={`rounded px-1.5 py-0.5 text-xs font-medium ${MODE_STYLES[mode].badge}`}>
                      {stageLabel(mode)}
                    </span>
                  </div>
                ) : (
                  <span
                    className={`inline-flex items-center gap-1.5 rounded px-2 py-1 text-xs font-medium ${MODE_STYLES[mode].badge}`}
                  >
                    <ModeIcon mode={mode} size={13} />
                    {stageLabel(mode)}
                  </span>
                )}

                {busy && started ? (
                  <Button
                    variant="danger"
                    onClick={() => sid && void interruptSession(sid)}
                    title="Interrupt the current turn"
                  >
                    <StopIcon />
                    Stop
                  </Button>
                ) : !started ? (
                  <Button variant="primary" busy={starting} disabled={!canSubmit} onClick={() => void handleStart()}>
                    Start
                  </Button>
                ) : (
                  <Button variant="primary" disabled={!canSubmit} onClick={() => void handleSend()}>
                    <SendIcon size={14} />
                    Send
                  </Button>
                )}
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
