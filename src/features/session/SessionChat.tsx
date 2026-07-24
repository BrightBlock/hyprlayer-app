import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { confirm } from "@tauri-apps/plugin-dialog";

import { Badge, Button, ErrorNote, Spinner } from "@/components/ui";
import {
  BookIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  ClaudeMarkIcon,
  CodeIcon,
  DocIcon,
  EffortIcon,
  EraserIcon,
  HandoffIcon,
  ImageIcon,
  ModeIcon,
  OpenAiIcon,
  PlusIcon,
  QuestionIcon,
  SendIcon,
  ShieldIcon,
  StopIcon,
  TerminalIcon,
  XIcon,
} from "@/components/icons";
import { errorMessage } from "@/lib/ipc";
import { eventToChord, hasPrimaryModifier } from "@/lib/keybindings/chord";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { basename } from "@/lib/path";
import { IS_MACOS } from "@/lib/platform";
import { useBootstrap, useCodexProbe, useOpencodeCatalog, useWorkstreamSessions } from "@/lib/queries";
import type { WorkstreamView } from "@/lib/bindings";
import { chipLabel, WorkstreamDocChips } from "@/features/workstreams/WorkstreamDocChips";
import { PromptChips, usePinnedPrompts } from "@/features/prompts/PromptChips";
import { AttachmentCards } from "./AttachmentCard";
import { importDroppedPaths, saveClipboardImage, type Attachment } from "./attachments";
import { consumeComposerFocus, useComposerFocusStore } from "./workstreamTabs";
import { generateTabTitle } from "./tabTitles";
import type { ShellTab } from "./workstreamTabs";
import { MODE_STAGES, MODE_STYLES, asStageKey, nextModeStage, stageLabel, suggestedDocKind, type StageKey } from "@/lib/domain/stages";
import {
  EFFORTS,
  apiContextWindow,
  codexEfforts,
  effectiveEffort,
  modelOption,
  modelSupportsUltracode,
  resolveCodexEffort,
  resolveCodexModelIds,
  resumeModelFlag,
  visibleModels,
} from "@/lib/domain/models";
import { useModelWindowMap } from "./modelWindowsStore";
import {
  effectiveHarnessKey,
  harnessOption,
  lockedSessionProvider,
  modelProvider,
  opencodeModelLabel,
  opencodeModelNames,
} from "@/lib/domain/harnessing";
import { PERMISSION_MODES, permissionModeOption } from "@/lib/domain/permissions";
import { docFromTool, formatTokens } from "./toolDisplay";
import { TranscriptView, type PermissionHandlers, type PromptHandlers, type TranscriptHandle } from "./TranscriptView";
import {
  allowForSession,
  answerQuestion,
  applyModel,
  applyPermissionMode,
  cancelQueuedMessage,
  isFirstUserTurn,
  killSession,
  respondPermission,
  restoreSessionIfNeeded,
  resumeSession,
  rewindToMessage,
  sendMessage,
  startSession,
  stopSession,
  useSession,
  useMostRecentDoc,
  useSessionBackgroundTasks,
  type QueuedMessage,
} from "./sessionStore";
import { BackgroundTasksIndicator } from "./BackgroundTasksIndicator";
import {
  buildQuestionAnswers,
  describeState,
  isBusy,
  isTerminal,
  isWorking,
  type PendingQuestion,
} from "./transcript";
import {
  AutocompleteDropdown,
  HighlightBackdrop,
  useComposerAutocomplete,
} from "./ComposerAutocomplete";
import { useCommand } from "@/lib/keybindings/registry";
import { useUiStore } from "@/lib/uiStore";
import { PromptHistoryStack } from "./PromptHistoryStack";
import {
  IDLE_NAV,
  recallAt,
  recallNext,
  recallPrev,
  userPrompts,
  type HistoryNav,
  type NavResult,
} from "./promptHistory";

const STAGE_INTRO: Record<StageKey, string> = {
  general: "Chat with the project agent in this worktree. Pick a stage below to run a spec-driven step.",
  research: "Runs /research_codebase to investigate the code and write a research doc into the library.",
  plan: "Runs /create_plan to turn research into a concrete, reviewable implementation plan.",
  implement: "Runs /implement_plan to make the changes in this worktree, committing as it goes.",
  validate: "Runs /validate_plan to check the work against the plan and the project's tests.",
  commit: "Runs /commit to commit this worktree's changes, then asks whether to push them — never a pull request. Use Ship when you're ready to open a PR.",
  ship: "Commits any remaining changes via /commit, then pushes the branch and opens a pull request.",
  revise: "Addresses PR feedback — failing CI or requested changes — then pushes fixes to the branch.",
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
  accent: "bg-working",
};

/** A tiny status dot for an agent tab encoding the session's tone (none until a
 *  session starts). Carries the color only — the tab icon's glow signals "live". */
export function SessionTabDot({ sessionId }: { sessionId?: string }) {
  const session = useSession(sessionId);
  if (!session) return null;
  const { tone } = describeState(session.state, session.streaming);
  return (
    <span
      className={`ml-0.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full ${DOT_COLOR[tone]}`}
      aria-hidden="true"
    />
  );
}

/** The mode badge leading an agent tab. Glows while the session is working (same
 *  `isWorking` signal as the elapsed counter) — a session paused for a permission
 *  shows the warn dot, not a pulse. Subscribes per-session. */
export function SessionTabIcon({ mode, sessionId }: { mode: StageKey; sessionId?: string }) {
  const session = useSession(sessionId);
  const working = session ? isWorking(session) : false;
  return (
    <span className={`rounded p-0.5 ${MODE_STYLES[mode].badge} ${working ? "animate-tab-glow" : ""}`}>
      <ModeIcon mode={mode} />
    </span>
  );
}

type MenuItem = {
  key: string;
  label: string;
  hint?: string;
  icon?: ReactNode;
  /** Render in red (e.g. "Bypass permissions") to flag a risky choice. */
  danger?: boolean;
  /** Greyed and non-selectable — the model picker locks a started session to its
   *  provider (see `lockedProvider` in SessionChat). Per-row, unlike a group's `disabled`. */
  disabled?: boolean;
  /** Accent-tint this row (and the trigger when active) so a distinct choice like Ultracode stands out. */
  accent?: boolean;
};

/** A labelled section in a grouped menu (the model picker groups models by
 *  harness). A `disabled` group is greyed/non-selectable — for "soon" harnesses like Codex. */
type MenuGroup = {
  key: string;
  label: string;
  icon?: ReactNode;
  items: MenuItem[];
  disabled?: boolean;
};

/**
 * The composer's mark for a harness, used by both the model trigger and every
 * model-menu group so the two can't drift apart.
 *
 * Deliberately NOT `harnessIcon` from the design system: the composer shows
 * OpenCode as a generic `CodeIcon` (it fronts many providers, so its own logo
 * would misrepresent which one a model comes from), while Settings shows the
 * OpenCode mark because there it names the harness itself.
 */
function composerMark(key: string, size = 13) {
  if (key === "codex") return <OpenAiIcon size={size} />;
  if (key === "opencode") return <CodeIcon size={size} />;
  return <ClaudeMarkIcon size={size} />;
}

/** A compact composer-toolbar dropdown (model / effort / mode). Opens upward —
 *  the composer sits at the bottom of the pane. */
function ComposerMenu({
  leading,
  label,
  hint,
  items = [],
  groups,
  footer,
  activeKey,
  onSelect,
  title,
  custom,
  compact = false,
}: {
  leading: ReactNode;
  label: string;
  hint?: string;
  items?: MenuItem[];
  /** When set, render a grouped menu instead of the flat `items` list (the model
   *  picker groups models by harness). `items`/`custom` are ignored in grouped mode. */
  groups?: MenuGroup[];
  /** An action row pinned to the bottom of the menu (e.g. "Harnesses →"). */
  footer?: { label: string; icon?: ReactNode; onClick: () => void };
  activeKey: string;
  onSelect: (key: string) => void;
  title: string;
  /** Appends a free-text row for a value not in `items` — the model picker uses it
   *  to accept ANY `--model` the CLI takes. Submitting calls `onSelect` with the text. */
  custom?: { label: string; placeholder?: string };
  /** Icon-only trigger (label/hint/chevron hidden) when the composer is too narrow. */
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  // Every selectable row across flat items and groups (disabled group/row excluded).
  const allItems = groups
    ? groups.flatMap((g) => (g.disabled ? [] : g.items.filter((it) => !it.disabled)))
    : items;
  // The active selection is "custom" when it matches none of the preset items.
  const customActive = custom != null && !allItems.some((it) => it.key === activeKey);
  // A risky active choice (e.g. "Bypass permissions") tints the whole trigger red.
  const activeDanger = allItems.some((it) => it.key === activeKey && it.danger);
  // An accent active choice (e.g. "Ultracode") tints the whole trigger accent.
  const activeAccent = allItems.some((it) => it.key === activeKey && it.accent);

  const close = () => {
    setOpen(false);
    setEditing(false);
    setText("");
  };
  const beginCustom = () => {
    setText(customActive ? activeKey : "");
    setEditing(true);
  };
  const commitCustom = () => {
    const value = text.trim();
    if (value) onSelect(value);
    close();
  };
  const onCustomKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitCustom();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setEditing(false);
    }
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => (open ? close() : setOpen(true))}
        title={compact ? `${title}: ${label}` : title}
        className={`flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-2 py-1 text-xs hover:bg-edge/50 ${
          activeDanger
            ? "text-bad hover:text-bad"
            : activeAccent
              ? "text-accent hover:text-accent"
              : "text-ink-dim hover:text-ink"
        }`}
      >
        <span className="shrink-0">{leading}</span>
        {!compact && (
          <>
            <span
              className={`max-w-[12rem] truncate font-medium ${
                activeDanger ? "text-bad" : activeAccent ? "text-accent" : "text-ink"
              }`}
            >
              {label}
            </span>
            {hint && <span className={activeDanger ? "text-bad/80" : "text-ink-dim"}>{hint}</span>}
            <ChevronDownIcon size={12} />
          </>
        )}
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={close} aria-hidden="true" />
          <div className="absolute bottom-full left-0 z-30 mb-1.5 max-h-[60vh] min-w-[190px] max-w-[22rem] overflow-y-auto overflow-x-hidden rounded-lg border border-edge bg-panel py-1 shadow-xl">
            {groups ? (
              <>
                {groups.map((g, gi) => (
                  <div key={g.key}>
                    {gi > 0 && <div className="my-1 border-t border-edge" />}
                    <div className="flex items-center gap-2 px-3 py-1 text-[11px] font-medium text-ink-dim/70">
                      {g.icon && <span className="shrink-0">{g.icon}</span>}
                      <span>{g.label}</span>
                    </div>
                    {g.items.map((it) =>
                      g.disabled || it.disabled ? (
                        <div
                          key={it.key}
                          title={
                            it.disabled && !g.disabled
                              ? "Locked to this session's provider — start a new agent to use a different provider"
                              : undefined
                          }
                          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink-dim/40"
                        >
                          {it.icon && <span className="shrink-0">{it.icon}</span>}
                          <span className="min-w-0 truncate font-medium">{it.label}</span>
                          {g.disabled && (
                            <span className="ml-auto shrink-0 text-[10px] uppercase tracking-wide">soon</span>
                          )}
                        </div>
                      ) : (
                        <button
                          key={it.key}
                          type="button"
                          onClick={() => {
                            onSelect(it.key);
                            close();
                          }}
                          className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/50 ${
                            it.key === activeKey ? "text-ink" : "text-ink-dim"
                          }`}
                        >
                          {it.icon && <span className="shrink-0">{it.icon}</span>}
                          <span className="min-w-0 truncate font-medium">{it.label}</span>
                          {it.hint && <span className="shrink-0 text-ink-dim">{it.hint}</span>}
                          {it.key === activeKey && (
                            <span className="ml-auto shrink-0 text-[11px] text-ink">✓</span>
                          )}
                        </button>
                      ),
                    )}
                  </div>
                ))}
                {footer && (
                  <>
                    <div className="my-1 border-t border-edge" />
                    <button
                      type="button"
                      onClick={() => {
                        footer.onClick();
                        close();
                      }}
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink-dim hover:bg-edge/50 hover:text-ink"
                    >
                      <span className="font-medium">{footer.label}</span>
                      {footer.icon && <span className="ml-auto shrink-0">{footer.icon}</span>}
                    </button>
                  </>
                )}
              </>
            ) : (
              <>
                {items.map((it) =>
                  it.disabled ? (
                    <div
                      key={it.key}
                      title={it.hint || undefined}
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink-dim/40"
                    >
                      {it.icon && <span className="shrink-0">{it.icon}</span>}
                      <span className="font-medium">{it.label}</span>
                      {it.hint && <span>{it.hint}</span>}
                    </div>
                  ) : (
                    <button
                      key={it.key}
                      type="button"
                      onClick={() => {
                        onSelect(it.key);
                        close();
                      }}
                      className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/50 ${
                        it.danger
                          ? "text-bad"
                          : it.accent
                            ? "text-accent"
                            : it.key === activeKey
                              ? "text-ink"
                              : "text-ink-dim"
                      }`}
                    >
                      {it.icon && <span className="shrink-0">{it.icon}</span>}
                      <span className="font-medium">{it.label}</span>
                      {it.hint && <span className="text-ink-dim">{it.hint}</span>}
                      {it.key === activeKey && <span className="ml-auto text-[10px] text-ink-dim">●</span>}
                    </button>
                  ),
                )}
                {custom && (
                  <>
                    <div className="my-1 border-t border-edge" />
                    {editing ? (
                      <div className="px-2 py-1">
                        <input
                          autoFocus
                          value={text}
                          onChange={(e) => setText(e.target.value)}
                          onKeyDown={onCustomKey}
                          onBlur={commitCustom}
                          placeholder={custom.placeholder}
                          spellCheck={false}
                          className="w-full rounded-md border border-edge bg-base px-2 py-1 text-xs text-ink placeholder:text-ink-dim focus:border-accent focus:outline-none"
                        />
                      </div>
                    ) : (
                      <button
                        type="button"
                        onClick={beginCustom}
                        className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/50 ${
                          customActive ? "text-ink" : "text-ink-dim"
                        }`}
                      >
                        <span className="font-medium">{customActive ? activeKey : custom.label}</span>
                        {customActive && <span className="ml-auto text-[10px] text-ink-dim">●</span>}
                      </button>
                    )}
                  </>
                )}
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}

/** The composer "+" — opens upward into a small action menu for attaching
 *  context: library docs (centered modal) or files / screenshots (native OS
 *  file dialog). Available every turn — before a session starts a library doc
 *  becomes first-turn context; mid-conversation it rides the next message as an
 *  attachment (ProjectPage routes the lane). Unlike ComposerMenu these items are
 *  one-shot actions. */
function AttachMenu({
  onLibrary,
  onFiles,
  onScreenshots,
}: {
  onLibrary: () => void;
  onFiles: () => void;
  onScreenshots: () => void;
}) {
  const [open, setOpen] = useState(false);
  const items = [
    { key: "library", label: "Attach from library", icon: <BookIcon size={14} />, run: onLibrary },
    { key: "files", label: "Add files…", icon: <DocIcon size={14} />, run: onFiles },
    { key: "shots", label: "Add screenshots…", icon: <ImageIcon size={14} />, run: onScreenshots },
  ];
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title="Attach context"
        aria-label="Attach context"
        className="flex h-8 w-8 items-center justify-center rounded-md text-ink-dim hover:bg-edge/50 hover:text-ink"
      >
        <PlusIcon />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={() => setOpen(false)} aria-hidden="true" />
          <div className="absolute bottom-full right-0 z-30 mb-1.5 min-w-[200px] overflow-hidden rounded-lg border border-edge bg-panel py-1 shadow-xl">
            {items.map((it) => (
              <button
                key={it.key}
                type="button"
                onClick={() => {
                  setOpen(false);
                  it.run();
                }}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink-dim hover:bg-edge/50 hover:text-ink"
              >
                <span className="shrink-0">{it.icon}</span>
                <span className="font-medium">{it.label}</span>
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

/**
 * Context-handoff nudge above the composer. Two flavours driven by `mode`:
 *  - `suggest` — context is filling up; offer to run /create_handoff now.
 *  - `ready` — a handoff doc was captured; one click clears the session and
 *    prefills /resume_handoff so a single Enter resumes against the doc.
 * Either flavour can be dismissed; `ready` re-arms only if the handoff is
 * detached or a fresh one is written.
 */
function HandoffBanner({
  mode,
  contextPct,
  suggestRequested,
  onCreateHandoff,
  onClearAndResume,
  onDismiss,
}: {
  mode: "suggest" | "ready";
  contextPct: number;
  /** /create_handoff has already been sent or queued — disable the button so
   *  a double-click can't queue a duplicate run. */
  suggestRequested: boolean;
  onCreateHandoff: () => void;
  onClearAndResume: () => void;
  onDismiss: () => void;
}) {
  const isReady = mode === "ready";
  const disabled = !isReady && suggestRequested;
  const actionLabel = isReady
    ? "Clear & resume"
    : suggestRequested
      ? "Creating…"
      : "Create handoff";
  return (
    <div className="mb-2 flex items-center gap-2 rounded-md border border-warn/40 bg-warn/5 px-3 py-2">
      <span className="shrink-0 text-warn">
        <HandoffIcon size={14} />
      </span>
      <span className="min-w-0 flex-1 text-xs text-ink-dim">
        {isReady ? (
          <>Handoff ready — clear context to start a fresh session with the handoff attached.</>
        ) : suggestRequested ? (
          <>Handoff queued — the agent will write it on the next turn.</>
        ) : (
          <>
            Context is filling up ({Math.round(contextPct * 100)}%). Create a handoff doc and
            continue in a fresh session?
          </>
        )}
      </span>
      <button
        type="button"
        disabled={disabled}
        onClick={isReady ? onClearAndResume : onCreateHandoff}
        className="shrink-0 rounded-md border border-warn/40 bg-warn/10 px-2 py-1 text-xs font-medium text-warn hover:bg-warn/20 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-warn/10"
      >
        {actionLabel}
      </button>
      <button
        type="button"
        onClick={onDismiss}
        title={isReady ? "Dismiss — keep working in this session" : "Dismiss"}
        aria-label="Dismiss"
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge hover:text-ink"
      >
        <XIcon size={10} strokeWidth={2.4} />
      </button>
    </div>
  );
}

/** Messages typed while a turn was running, shown above the composer until the
 *  turn finishes and they're delivered. Each can be cancelled before it sends. */
function QueuedMessages({
  messages,
  onCancel,
}: {
  messages: QueuedMessage[];
  onCancel: (index: number) => void;
}) {
  return (
    <div className="mb-2 space-y-1">
      {messages.map((msg, i) => {
        const n = msg.attachments.length;
        return (
        <div
          key={i}
          className="flex items-center gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5"
        >
          <span className="shrink-0 text-[10px] font-medium uppercase tracking-wide text-ink-dim/70">
            Queued
          </span>
          <span className="flex-1 truncate text-xs text-ink">
            {msg.text || (n > 0 ? `${n} attachment(s)` : "")}
          </span>
          {n > 0 && msg.text && <span className="shrink-0 text-[10px] text-ink-dim">📎{n}</span>}
          <button
            type="button"
            onClick={() => onCancel(i)}
            title="Cancel queued message"
            aria-label="Cancel queued message"
            className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
          >
            <XIcon size={12} />
          </button>
        </div>
        );
      })}
    </div>
  );
}

/**
 * The interactive answer panel for an `AskUserQuestion`, pinned just above the
 * composer. Questions are asked one at a time (a stepper): each question shows
 * its options (single- or multi-select) plus an always-present "Other" field for
 * a custom free-text answer when none of the options fit (parity with Claude
 * Code's question UI). "Next" advances, and the last question's "Submit" sends
 * every collected answer back to the agent in one go. Selections and custom text
 * accumulate across steps (keyed by question text) and "Back" revisits them.
 * "Skip" sends no answer, which the agent handles by asking again in prose.
 * Remounted per prompt (keyed on requestId) so state never leaks between asks.
 */
function QuestionPrompt({
  pending,
  onSubmit,
}: {
  pending: PendingQuestion;
  onSubmit: (answers: Record<string, string>) => void;
}) {
  const [selections, setSelections] = useState<Record<string, string[]>>({});
  // Free-text "Other" answers, keyed by question text. A present entry means the
  // field is open for that question; its text is sent alongside (multi) or
  // instead of (single) the option picks.
  const [custom, setCustom] = useState<Record<string, string>>({});
  const [step, setStep] = useState(0);
  // Latch on the first submit/skip so a double-click can't answer twice (the
  // panel unmounts once the optimistic resolve clears the pending question).
  const [submitted, setSubmitted] = useState(false);

  const send = (answers: Record<string, string>) => {
    if (submitted) return;
    setSubmitted(true);
    onSubmit(answers);
  };

  const total = pending.questions.length;
  const question = pending.questions[step];

  // Defensive: a malformed payload can parse to zero questions — don't wedge the
  // session, let the user dismiss it (the agent then re-asks in prose).
  if (!question) {
    return (
      <div className="mb-2 flex items-center justify-between gap-2 rounded-xl border border-brand/30 bg-brand/[0.06] p-3.5">
        <span className="text-sm text-ink-dim">
          The agent asked a question that couldn’t be displayed.
        </span>
        <Button disabled={submitted} onClick={() => send({})}>
          Dismiss
        </Button>
      </div>
    );
  }

  const chosen = selections[question.question] ?? [];
  // A present `custom` entry means the "Other" free-text field is open for this
  // question; its trimmed text counts as an answer alongside any option picks.
  const otherText = custom[question.question];
  const otherOpen = otherText !== undefined;
  const currentAnswered = chosen.length > 0 || (otherText ?? "").trim().length > 0;
  const isLast = step === total - 1;

  // Close the "Other" field for this question, dropping its text.
  const closeOther = () =>
    setCustom((prev) => {
      if (!(question.question in prev)) return prev;
      const next = { ...prev };
      delete next[question.question];
      return next;
    });

  const toggle = (label: string) => {
    setSelections((prev) => {
      const current = prev[question.question] ?? [];
      if (question.multiSelect) {
        const next = current.includes(label)
          ? current.filter((l) => l !== label)
          : [...current, label];
        return { ...prev, [question.question]: next };
      }
      return { ...prev, [question.question]: [label] };
    });
    // Single-select: a preset option and the free-text "Other" are mutually
    // exclusive, so picking an option closes the field.
    if (!question.multiSelect) closeOther();
  };

  // Open/close the "Other" free-text field. Single-select clears any preset pick
  // so the two stay mutually exclusive; multi-select keeps picks and appends.
  const toggleOther = () => {
    if (otherOpen) {
      closeOther();
      return;
    }
    setCustom((prev) => ({ ...prev, [question.question]: "" }));
    if (!question.multiSelect) setSelections((prev) => ({ ...prev, [question.question]: [] }));
  };

  const submitAll = () => send(buildQuestionAnswers(pending.questions, selections, custom));

  // "Next" until the last question, which becomes "Submit"; both need an answer.
  const advance = () => {
    if (!currentAnswered) return;
    if (isLast) submitAll();
    else setStep((s) => s + 1);
  };

  // The radio/checkbox dot for the "Other" row — filled (accent) when its
  // free-text field is open. `mt` aligns it with the label's first line in the
  // closed two-line layout; the open inline-input row centers it instead.
  const otherDot = (mt: string) => (
    <span
      className={`flex h-4 w-4 shrink-0 items-center justify-center border ${mt} ${
        question.multiSelect ? "rounded" : "rounded-full"
      } ${otherOpen ? "border-accent bg-accent" : "border-ink-dim/50"}`}
      aria-hidden="true"
    >
      {otherOpen && (
        <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
          <path d="m2.5 6 2.5 2.5 4.5-5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-accent-ink" />
        </svg>
      )}
    </span>
  );

  return (
    <div className="mb-2 rounded-xl border border-brand/30 bg-brand/[0.06] p-3.5">
      <div className="mb-2.5 flex items-center gap-2 text-brand">
        <QuestionIcon size={15} />
        <span className="text-sm font-medium">The agent is asking</span>
        {total > 1 && (
          <span className="ml-auto text-xs font-normal tabular-nums text-ink-dim">
            {step + 1} of {total}
          </span>
        )}
      </div>

      {question.header && (
        <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-dim">
          {question.header}
        </span>
      )}
      <p className="text-sm font-medium text-ink">{question.question}</p>
      {question.multiSelect && (
        <p className="mt-0.5 text-xs text-ink-dim">Select all that apply.</p>
      )}
      <div className="mt-2 space-y-1.5">
        {question.options.map((opt) => {
          const selected = chosen.includes(opt.label);
          return (
            <button
              key={opt.label}
              type="button"
              disabled={submitted}
              onClick={() => toggle(opt.label)}
              aria-pressed={selected}
              className={`flex w-full items-start gap-2.5 rounded-lg border px-3 py-2 text-left transition-colors disabled:opacity-50 ${
                selected
                  ? "border-accent bg-accent/15"
                  : "border-edge bg-panel-2 hover:border-edge hover:bg-edge/50"
              }`}
            >
              <span
                className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center border ${
                  question.multiSelect ? "rounded" : "rounded-full"
                } ${selected ? "border-accent bg-accent" : "border-ink-dim/50"}`}
                aria-hidden="true"
              >
                {selected && (
                  <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
                    <path d="m2.5 6 2.5 2.5 4.5-5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-accent-ink" />
                  </svg>
                )}
              </span>
              <span className="min-w-0">
                <span className="block text-sm text-ink">{opt.label}</span>
                {opt.description && (
                  <span className="mt-0.5 block text-xs leading-snug text-ink-dim">
                    {opt.description}
                  </span>
                )}
              </span>
            </button>
          );
        })}
        {/* "Other": a free-text answer, always offered so the user isn't boxed
            into the preset options (parity with Claude Code's question UI). When
            open the option row itself becomes an inline input — its text is
            appended to any picks for multi-select, or replaces them for
            single-select. */}
        <div
          className={`rounded-lg border transition-colors ${
            otherOpen
              ? "border-accent bg-accent/15"
              : "border-edge bg-panel-2 hover:border-edge hover:bg-edge/50"
          }`}
        >
          {otherOpen ? (
            <div className="flex items-center gap-2.5 px-3 py-2">
              <button
                type="button"
                disabled={submitted}
                onClick={toggleOther}
                aria-pressed={otherOpen}
                aria-label="Remove custom answer"
                className="shrink-0 disabled:opacity-50"
              >
                {otherDot("")}
              </button>
              <input
                autoFocus
                disabled={submitted}
                value={otherText ?? ""}
                aria-label="Custom answer"
                onChange={(e) =>
                  setCustom((prev) => ({ ...prev, [question.question]: e.target.value }))
                }
                onKeyDown={(e) => {
                  // Enter submits the custom answer (single-line inline field).
                  if (e.key === "Enter") {
                    e.preventDefault();
                    advance();
                  }
                }}
                placeholder="Type your answer…"
                className="min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-ink-dim focus:outline-none disabled:opacity-50"
              />
            </div>
          ) : (
            <button
              type="button"
              disabled={submitted}
              onClick={toggleOther}
              aria-pressed={otherOpen}
              className="flex w-full items-start gap-2.5 px-3 py-2 text-left disabled:opacity-50"
            >
              {otherDot("mt-0.5")}
              <span className="min-w-0">
                <span className="block text-sm text-ink">Other…</span>
                <span className="mt-0.5 block text-xs leading-snug text-ink-dim">
                  Write a custom answer
                </span>
              </span>
            </button>
          )}
        </div>
      </div>

      <div className="mt-3 flex items-center gap-2">
        {step > 0 && (
          <Button disabled={submitted} onClick={() => setStep((s) => s - 1)}>
            Back
          </Button>
        )}
        <Button variant="primary" disabled={!currentAnswered || submitted} onClick={advance}>
          {isLast ? (total > 1 ? "Submit answers" : "Submit answer") : "Next"}
        </Button>
        <Button disabled={submitted} onClick={() => send({})}>
          Skip
        </Button>
      </div>
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
  onPermissionModeChange: (key: string) => void;
  /** Persist the chosen harness (claude|codex|opencode) on the tab so it survives
   *  a tab switch; fixed once the session starts. */
  onHarnessChange: (key: string) => void;
  /** Persist the chosen OpenCode `provider/model` id on the tab (the Claude model
   *  lives in `onModelChange`). */
  onOpencodeModelChange: (key: string) => void;
  /** Persist the chosen Codex model id / reasoning effort on the tab. Separate
   *  from the Claude setters because Codex's ids and effort ladder are its own. */
  onCodexModelChange: (key: string) => void;
  onCodexEffortChange: (key: string) => void;
  /** Open Settings → Harnesses (the model menu's "Harnesses →" footer). */
  onOpenHarnessSettings: () => void;
  onSessionStarted: (sessionId: string) => void;
  onResetSession: () => void;
  onOpenDoc: (id: string) => void;
  /** Remove a doc — detaches a manual doc, dismisses (and records) a suggested one. */
  onDetachDoc: (path: string) => void;
  /** Reconcile the pane's auto-suggested doc (called by the mode-suggestion effect). */
  onSuggestDoc: (path: string | null, role: string) => void;
  /** Hide the "context filling up" banner for this tab. */
  onDismissHandoffNudge: () => void;
  /** Capture a freshly written handoff doc as a chip on this tab — survives Clear
   *  Context so it auto-attaches to the next session this tab starts. */
  onAttachHandoffDoc: (path: string) => void;
  /** Open the library picker (centered modal) to attach context docs. */
  onBrowseDocs: () => void;
  /** Native OS file dialog → attach the chosen files as context. */
  onAddFiles: () => void;
  /** Native OS file dialog filtered to images → attach screenshots. */
  onAddScreenshots: () => void;
  /** Stage dropped/pasted attachments on this tab's composer. */
  onAddAttachments: (attachments: Attachment[]) => void;
  /** Remove one staged attachment by path. */
  onRemoveAttachment: (path: string) => void;
  /** Clear all staged attachments (after a turn is sent). */
  onClearAttachments: () => void;
};

// The composer grows from one row up to this height, then scrolls. ~14 rows of
// text-sm — generous enough to keep a whole prompt in view without letting the
// box crowd out the transcript above it.
const MAX_COMPOSER_HEIGHT = 320;

export function SessionChat({
  view,
  shell,
  onDraftChange,
  onModeChange,
  onModelChange,
  onEffortChange,
  onPermissionModeChange,
  onHarnessChange,
  onOpencodeModelChange,
  onCodexModelChange,
  onCodexEffortChange,
  onOpenHarnessSettings,
  onSessionStarted,
  onResetSession,
  onOpenDoc,
  onDetachDoc,
  onSuggestDoc,
  onDismissHandoffNudge,
  onAttachHandoffDoc,
  onBrowseDocs,
  onAddFiles,
  onAddScreenshots,
  onAddAttachments,
  onRemoveAttachment,
  onClearAttachments,
}: Props) {
  const session = useSession(shell.sessionId);
  const [starting, setStarting] = useState(false);
  const [resuming, setResuming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);
  // The add-attachments callback changes identity each render; keep the latest
  // in a ref so the window-level drag-drop subscription can stay mounted once.
  const addAttachmentsRef = useRef(onAddAttachments);
  addAttachmentsRef.current = onAddAttachments;
  const workstreamId = view.item.id;

  // Collapse the composer toolbar's pickers to icon-only when their labels would
  // otherwise overflow the row — a narrow split pane, a long permission label
  // ("Bypass permissions"), or a running session's extra meter/clear buttons.
  // We detect *actual* overflow off the toolbar element instead of guessing from
  // a pane-width threshold: the row's width is content-dependent, so any fixed
  // number is wrong for some state (and silently regresses when a button is
  // added). naturalWidthRef remembers the labelled width so we only expand again
  // once it genuinely fits — the hysteresis that stops collapse/expand flapping.
  const toolbarRef = useRef<HTMLDivElement>(null);
  const naturalWidthRef = useRef(0);
  const [compactComposer, setCompactComposer] = useState(false);
  // Re-evaluated on every render (content changes — a session starting, the
  // permission label, the live token meter) and on every resize (the observer
  // below — split-divider drags and window resizes, which don't re-render this
  // component). Kept in a ref so the once-bound observer always runs the latest.
  const measureRef = useRef<() => void>(() => {});
  measureRef.current = () => {
    const el = toolbarRef.current;
    if (!el) return;
    if (!compactComposer) {
      naturalWidthRef.current = el.scrollWidth;
      if (el.scrollWidth > el.clientWidth + 1) setCompactComposer(true);
    } else if (naturalWidthRef.current && el.clientWidth >= naturalWidthRef.current) {
      setCompactComposer(false);
    }
  };
  useLayoutEffect(() => {
    measureRef.current();
  });
  useLayoutEffect(() => {
    const el = toolbarRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => measureRef.current());
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const started = Boolean(shell.sessionId);
  const hasEntry = Boolean(session);
  const mode = asStageKey(shell.mode);
  // The configurable "cycle agent stage" chord (default Shift+Tab). Matched in
  // the composer because a chord with no primary modifier is inert in the global
  // dispatcher's editable-target guard, so it never fires while the textarea is
  // focused — onKeyDown picks it up here instead.
  const cycleStageChord = effectiveChord(
    useBootstrap().data?.settings.keybindings?.overrides ?? {},
    "ws.cycleStage",
    IS_MACOS,
  );
  // Pinned prompt chips, anchored beside the task drawer's handle above the
  // composer (they also drive that strip's space reservation).
  const pinnedPrompts = usePinnedPrompts(view.item.projectPath);
  // A tab with no explicit pick falls back to the provider defaults configured
  // in Settings → Models → Claude, then to claude's own defaults. `defaultPlanMode`
  // seeds the permission posture to "plan" for fresh tabs.
  const claudeDefaults = useBootstrap().data?.settings.providers?.claude;
  const model = modelOption(shell.model ?? claudeDefaults?.defaultModel ?? undefined);
  const effort = effectiveEffort(shell.effort ?? claudeDefaults?.defaultEffort ?? undefined, model);
  // Live per-model context windows from the Models API (fetched once, reused);
  // empty until it resolves, or if it can't (offline/logged out) — the static
  // `model.contextWindow` table then covers the meter.
  const modelWindows = useModelWindowMap();
  const permissionMode = permissionModeOption(
    shell.permissionMode ?? (claudeDefaults?.defaultPlanMode ? "plan" : undefined),
  );
  const modelItems = visibleModels(claudeDefaults?.visibleModels);

  // The harness is implied by which model you pick: the model menu groups models
  // by harness (Claude / Codex / OpenCode), so selecting one sets BOTH the model
  // and the harness. Both persist on the tab — like effort — so switching tabs
  // and coming back keeps the choice. Menu keys are namespaced `harness:key`
  // so the select handler knows which harness a row belongs to.
  const bootstrap = useBootstrap().data;
  const bootstrapSettings = bootstrap?.settings;
  const opencodeDefaults = bootstrapSettings?.providers?.opencode;
  const ollamaDefaults = bootstrapSettings?.providers?.ollama;
  const codexDefaults = bootstrapSettings?.providers?.codex;
  // Codex has no static preset list — its models come from a live probe. To offer
  // them in the picker by default (like Claude's presets) we probe whenever Codex
  // is installed, not only once a tab is already on it; otherwise you could never
  // switch TO Codex from a Claude tab with no curated selection.
  const codexInstalled = (bootstrap?.binaries ?? []).some(
    (b) => b.kind === "codex" && b.state.state === "ok",
  );
  // A started chat stays on the harness it LAUNCHED on — its transcript and tool
  // history belong to that harness's session, and changing the default harness
  // must only affect the next one. The session row is authoritative (the backend
  // pins it at launch and resumes by it), so it outranks both the tab's pick and
  // the current default; that also repairs a tab whose pick already drifted.
  // Unstarted: the tab's own pick, else the configured default, else Claude.
  const startedHarness = useWorkstreamSessions(workstreamId, Boolean(shell.sessionId)).data?.find(
    (s) => s.id === shell.sessionId,
  )?.harness;
  const harness = harnessOption(
    startedHarness ??
      shell.harness ??
      effectiveHarnessKey(bootstrapSettings?.providers?.defaultHarness),
  );
  const usingOpencode = harness.key === "opencode";
  const usingCodex = harness.key === "codex";
  const ocModels = opencodeDefaults?.selectedModels ?? [];
  // Ollama models run *through* the OpenCode harness as `ollama/<id>` ids, so they
  // share the opencode harness/model machinery — they're just a separate picker group.
  const ollamaModels = (ollamaDefaults?.selectedModels ?? []).map((id) => `ollama/${id}`);
  const ocModel =
    shell.opencodeModel ??
    opencodeDefaults?.defaultModel ??
    ollamaDefaults?.defaultModel ??
    ocModels[0] ??
    ollamaModels[0] ??
    null;
  // One probe serves two needs: it enumerates the models (so the picker can offer
  // them) AND reports each model's own effort tiers (GPT-5.6-Sol has `ultra`, Luna
  // stops at `max` — offering a tier the model rejects would fail the turn). Gated
  // on Codex being installed, and deduped by react-query across tabs, so it costs
  // one app-server spawn regardless of how many composers are open.
  const codexCatalog = useCodexProbe(codexInstalled);
  // No explicit selection ⇒ offer every model Codex reports, the same way Claude
  // shows all its presets until you narrow them.
  const codexModels = resolveCodexModelIds(
    codexDefaults?.selectedModels,
    codexCatalog.data?.models ?? [],
  );
  const codexModel = shell.codexModel ?? codexDefaults?.defaultModel ?? codexModels[0] ?? null;
  const codexSupportedEfforts = useMemo(
    () => codexCatalog.data?.models.find((m) => m.id === codexModel)?.supportedEfforts,
    [codexCatalog.data, codexModel],
  );
  const codexEffortItems = codexEfforts(codexSupportedEfforts);
  const codexEffort = resolveCodexEffort(
    shell.codexEffort ?? codexDefaults?.defaultEffort,
    codexSupportedEfforts,
  );
  const startModelFlag = usingOpencode ? ocModel : usingCodex ? codexModel : model.flag;
  // Each harness gets its OWN effort ladder: Codex's values differ from Claude's,
  // and OpenCode has no effort control at all (the driver ignores it).
  const startEffort = usingCodex ? codexEffort.key : usingOpencode ? null : effort.key;
  // Friendly OpenCode model names from the models.dev catalog — the raw ids are
  // long `provider/model` paths. Only fetched when this tab has OpenCode models
  // to label; the query is cached hard and falls back to a bundled snapshot.
  const ocCatalog = useOpencodeCatalog(ocModels.length > 0);
  const ocNames = useMemo(
    () => opencodeModelNames(ocCatalog.data?.providers ?? []),
    [ocCatalog.data],
  );

  // Once a session has started, its provider is LOCKED: a mid-session switch
  // replays the session's tool history to a different API endpoint, and a stricter
  // provider (e.g. OpenCode Zen / DeepSeek) rejects the synthesized tool_call_ids
  // ("Duplicate value for 'tool_call_id'…") — hard-failing the turn. Same-provider
  // model switches are safe (and are the only kind Claude or Codex ever offers,
  // since all their models share one provider), so we grey out cross-provider rows
  // until the user starts a new agent. The lock keys on the *underlying* provider —
  // the id prefix (`openrouter/…`, `opencode/…`, `ollama/…`) — not the harness,
  // because e.g. Kimi (OpenRouter) → Big Pickle (Zen) is cross-provider within OpenCode.
  const lockedProvider = lockedSessionProvider(started, harness.key, ocModel);
  const lockOther = (itemProvider: string): Pick<MenuItem, "disabled"> =>
    lockedProvider !== null && itemProvider !== lockedProvider ? { disabled: true } : {};

  const modelGroups: MenuGroup[] = [
    {
      key: "claude",
      label: "Claude Code",
      icon: composerMark("claude"),
      items: modelItems.map((m) => ({
        key: `claude:${m.key}`,
        label: m.label,
        hint: m.hint,
        icon: composerMark("claude"),
        ...lockOther("claude"),
      })),
    },
    {
      key: "codex",
      label: "Codex",
      icon: composerMark("codex"),
      // Codex model ids are already readable (`gpt-5.4-codex`), so — unlike
      // OpenCode's long `provider/model` paths — they need no catalog lookup.
      items: codexModels.map((id) => ({
        key: `codex:${id}`,
        label: id,
        icon: composerMark("codex"),
        ...lockOther("codex"),
      })),
    },
    {
      key: "opencode",
      label: "OpenCode",
      icon: composerMark("opencode"),
      items: ocModels.map((id) => ({
        key: `opencode:${id}`,
        label: opencodeModelLabel(id, ocNames),
        hint: ocNames.get(id)?.provider,
        icon: composerMark("opencode"),
        ...lockOther(modelProvider("opencode", id) ?? ""),
      })),
    },
    // Local models, routed through the OpenCode harness. Hidden until the user
    // selects some in Settings → Harnesses → Ollama.
    ...(ollamaModels.length > 0
      ? [
          {
            key: "ollama",
            label: "Ollama (local)",
            icon: <TerminalIcon size={13} />,
            items: ollamaModels.map((id) => ({
              key: `opencode:${id}`,
              label: id.replace(/^ollama\//, ""),
              hint: "Ollama",
              icon: <TerminalIcon size={13} />,
              ...lockOther(modelProvider("opencode", id) ?? ""),
            })),
          },
        ]
      : []),
  ];
  // Trigger reflects the active model; the active row carries a ✓ in the menu.
  const composerModelLabel = usingOpencode
    ? ocModel
      ? opencodeModelLabel(ocModel, ocNames)
      : "Select a model"
    : usingCodex
      ? (codexModel ?? "Select a model")
      : model.label;
  const composerModelHint = usingOpencode || usingCodex ? undefined : model.hint;
  const composerModelActiveKey = usingOpencode
    ? ocModel
      ? `opencode:${ocModel}`
      : ""
    : usingCodex
      ? codexModel
        ? `codex:${codexModel}`
        : ""
      : `claude:${model.key}`;

  // Picking a model selects its harness too: `onHarnessChange` + the per-harness
  // model setter both patch the tab.
  // Claude binds its model at launch, so switching the model on a LIVE session
  // relaunches the process via --resume onto the new model (transcript preserved)
  // — the model analog of handlePermissionModeChange. Effort is re-resolved for
  // the new model so ultracode only rides an xhigh-capable one. We confirm first
  // when a turn is in flight (the restart interrupts it). Before a session starts
  // (or when detached) we just record the choice; it applies at the next spawn.
  const applyClaudeModel = async (key: string) => {
    if (key === model.key) return; // no change (re-picked the active model)
    const next = modelOption(key);
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && session && isBusy(session)) {
      const ok = await confirm(
        `Restart this session to switch to ${next.label}? The active turn will be interrupted.`,
        { title: "Switch model?", kind: "warning" },
      );
      if (!ok) return; // declined — leave the running session (and picker) as-is
    }
    onModelChange(key); // persist on the tab (composer label + next fresh session)
    if (live && sid) {
      const nextEffort = effectiveEffort(
        shell.effort ?? claudeDefaults?.defaultEffort ?? undefined,
        next,
      );
      await applyModel(sid, resumeModelFlag(next), nextEffort.key, permissionMode.flag);
    }
  };

  const onPickModel = (namespaced: string) => {
    const sep = namespaced.indexOf(":");
    const pickedHarness = namespaced.slice(0, sep);
    const pickedModel = namespaced.slice(sep + 1);
    // Enforce the provider lock even if a disabled row is somehow activated: a
    // started session can only change models within its own provider.
    const pickedProvider = modelProvider(pickedHarness, pickedModel);
    if (lockedProvider !== null && pickedProvider !== lockedProvider) return;
    if (pickedHarness === "opencode") {
      onHarnessChange("opencode");
      onOpencodeModelChange(pickedModel);
    } else if (pickedHarness === "codex") {
      // Codex takes its model per turn, so — unlike Claude — no relaunch or
      // interrupt confirmation is needed even mid-conversation.
      onHarnessChange("codex");
      onCodexModelChange(pickedModel);
    } else {
      onHarnessChange("claude");
      void applyClaudeModel(pickedModel);
    }
  };

  // A tab bound to a session but with no transcript in memory means we came
  // back after an app restart — rebuild it from the persisted event log. Use the
  // tab's OWN stored permission mode (not the plan-mode default), so resuming an
  // old session re-applies what it actually started with rather than a default
  // that may have been toggled on since.
  const restorePermissionFlag = permissionModeOption(shell.permissionMode).flag;
  useEffect(() => {
    if (shell.sessionId && !hasEntry) {
      void restoreSessionIfNeeded(shell.sessionId, view.item.id, shell.mode, restorePermissionFlag);
    }
  }, [shell.sessionId, hasEntry, view.item.id, shell.mode, restorePermissionFlag]);

  // Auto-resume terminal sessions on first sight — a session that came back
  // crashed or killed. App-initiated teardown (quit, archive, relaunch) exits
  // `detached` instead, which never lands terminal, so it isn't this path's job:
  // those resume lazily on the next send. The ref guards against re-firing: if the
  // resume itself fails and bounces back to terminal, we surface the banner
  // for an explicit retry instead of looping. Cleared when the user opts
  // out via "New session" (sessionId becomes undefined).
  const autoResumedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!shell.sessionId) {
      autoResumedRef.current = null;
      return;
    }
    if (autoResumedRef.current === shell.sessionId) return;
    if (!session || !isTerminal(session.state)) return;
    // Only attempt when there's actually something to resume — empty sessions
    // (spawn_failed before any events) don't have a transcript worth chasing.
    if (session.items.length === 0) return;
    autoResumedRef.current = shell.sessionId;
    setError(null);
    setResuming(true);
    void resumeSession(shell.sessionId)
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setResuming(false));
  }, [shell.sessionId, session]);

  // Mode-aware suggestion: auto-attach the most recent relevant doc (research
  // for plan-mode, plan for implement/validate) as a "suggested" chip while the
  // session hasn't started yet. A ref absorbs the parent callback's identity
  // churn so the effect can depend on the real inputs it reads — the resolved
  // suggestion (path + kind) and whether the session has started — without a
  // dependency suppression.
  const suggestKind = suggestedDocKind(mode);
  const recent = useMostRecentDoc(workstreamId, suggestKind);
  const suggestDocRef = useRef(onSuggestDoc);
  suggestDocRef.current = onSuggestDoc;
  useEffect(() => {
    if (started) return; // suggestions only apply before a session is launched
    suggestDocRef.current(recent?.path ?? null, recent?.kind ?? "other");
  }, [started, recent?.path, recent?.kind]);

  // Drag-and-drop. Tauri intercepts OS file drops at the window level (HTML5
  // drop events never fire), so we subscribe to the webview's drag-drop stream
  // and hit-test the drop position against THIS pane's container — each pane
  // handles only drops that land on it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    const within = (pos?: { x: number; y: number }) => {
      const el = dropRef.current;
      if (!el || !pos) return false;
      const r = el.getBoundingClientRect();
      // The event position already arrives in CSS/logical pixels on macOS
      // (matching getBoundingClientRect) despite its PhysicalPosition type, so
      // compare directly. Dividing by devicePixelRatio halves the coordinates
      // on a Retina display and pulls every drop toward the window's top-left —
      // harmless in a single full-width pane, but it made a split always route
      // drops to the left pane instead of the one under the cursor.
      return pos.x >= r.left && pos.x <= r.right && pos.y >= r.top && pos.y <= r.bottom;
    };
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          setDragOver(within(p.position));
        } else if (p.type === "drop") {
          const hit = within(p.position);
          setDragOver(false);
          if (hit && p.paths.length > 0) {
            void importDroppedPaths(workstreamId, p.paths).then((atts) => {
              if (atts.length) addAttachmentsRef.current(atts);
            });
          }
        } else {
          setDragOver(false);
        }
      })
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [workstreamId]);

  // Paste an image straight from the clipboard (e.g. Cmd-Shift-Ctrl-4 screen
  // grabs). The bytes are persisted into the worktree so the agent can read
  // them; the paste of image data doesn't also insert text.
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    const images: File[] = [];
    for (let i = 0; i < items.length; i += 1) {
      const it = items[i];
      if (it.kind === "file" && it.type.startsWith("image/")) {
        const file = it.getAsFile();
        if (file) images.push(file);
      }
    }
    if (images.length === 0) return;
    e.preventDefault();
    void Promise.all(images.map((f) => saveClipboardImage(workstreamId, f))).then((res) => {
      const atts = res.filter((a): a is Attachment => a !== null);
      if (atts.length) onAddAttachments(atts);
    });
  };

  const busy = session ? isBusy(session) : started;
  // `working` (actively computing) drives the transcript's running-tool spinner
  // and smooth-scroll; `busy` (which also covers "blocked on the user") gates
  // the composer. They diverge while a session is held on a permission/question.
  // Falls back to false (not `started`) so a tab still rebuilding from the log
  // after a restart shows the "Connecting…" spinner, not a fake running turn.
  const working = session ? isWorking(session) : false;
  const terminal = session ? isTerminal(session.state) : false;
  const pendingQuestion = session?.pendingQuestion ?? null;
  const draft = shell.draft;
  // Tabs persisted before attachments existed rehydrate without the field; the
  // store migration backfills it, but default here too so a stale in-memory tab
  // can't crash on `.length` before the next reload.
  const attachments = shell.attachments ?? [];
  const canSubmit = draft.trim().length > 0 || attachments.length > 0;
  const sid = shell.sessionId;

  // Background tasks bar.
  const backgroundTasks = useSessionBackgroundTasks(sid);

  // Prompt history navigation: derive prompts from the live transcript,
  // hold ephemeral nav state, and register focus-gated recall commands.
  const prompts = useMemo(
    () => (session ? userPrompts(session.items) : []),
    [session],
  );
  const [historyNav, setHistoryNav] = useState<HistoryNav>(IDLE_NAV);
  const [composerFocused, setComposerFocused] = useState(false);
  // The value we last wrote via recall, so the textarea onChange can tell a
  // programmatic recall apart from a real user edit.
  const recalledValueRef = useRef<string | null>(null);

  const applyRecall = useCallback(
    (res: NavResult) => {
      if (!res) return;
      setHistoryNav(res.nav);
      recalledValueRef.current = res.draft;
      onDraftChange(res.draft);
      requestAnimationFrame(() => {
        const ta = textareaRef.current;
        if (!ta) return;
        ta.focus();
        const n = ta.value.length;
        ta.setSelectionRange(n, n); // cursor to end for natural editing
      });
    },
    [onDraftChange],
  );

  // Reset navigation when the bound session changes (Clear Context / restart).
  useEffect(() => {
    setHistoryNav(IDLE_NAV);
    recalledValueRef.current = null;
  }, [sid]);

  const canRecall = composerFocused && prompts.length > 0 && !pendingQuestion;
  useCommand(
    "session.historyPrev",
    canRecall ? () => applyRecall(recallPrev(prompts, historyNav, draft)) : null,
  );
  useCommand(
    "session.historyNext",
    historyNav.index !== null
      ? () => applyRecall(recallNext(prompts, historyNav, draft))
      : null,
  );

  // The plan/task list moved from the composer into the changes sidebar; keep the
  // "Show / Hide Tasks" shortcut wired to that section's (persisted) collapse toggle.
  const toggleTasks = useUiStore((s) => s.toggleTasks);
  useCommand("session.togglePlan", toggleTasks);

  const transcriptRef = useRef<TranscriptHandle | null>(null);

  // Handoff flow: at ≥60% context fill, nudge the user to create a handoff doc
  // and start fresh. The flow is two banners and a button glow:
  //   1. suggest — show while pct ≥ NUDGE_THRESHOLD and the user hasn't acted;
  //      "Create handoff" sends `/create_handoff`.
  //   2. ready — once the agent's Write produces a doc under `…/handoffs/…md`,
  //      it auto-attaches to this tab as a regular doc chip (so it survives
  //      `handleClear` and rides into the next session as an attachment). The
  //      Eraser button lights up; "Clear & resume" runs handleClear and
  //      prefills `/resume_handoff`.
  const HANDOFF_NUDGE_THRESHOLD = 0.6;
  // The meter's denominator, in precedence order: the backend-reported window
  // for this session (e.g. an Ollama model's fixed num_ctx); else the Models API
  // value for the session's resolved model id (the live source of truth, keyed on
  // the id `claude` echoed at init); else our static per-model table. The `model`
  // table only knows Claude models, so for an OpenCode/Ollama session it would
  // otherwise wrongly read the 1M Opus default.
  const contextWindow =
    session?.contextWindow ??
    apiContextWindow(session?.model, modelWindows) ??
    model.contextWindow;
  const contextPct =
    session?.contextTokens != null ? session.contextTokens / contextWindow : 0;
  const handoffDoc = shell.docs.find((d) => d.role === "handoff") ?? null;
  // Last handoff doc written in the CURRENT session's items, ignoring ones the
  // user already dismissed. We scan the live transcript rather than the
  // workstream-wide selector so an old handoff from a previous session of this
  // workstream doesn't pop in as soon as the new session opens.
  const sessionItems = session?.items;
  const dismissedSet = useMemo(
    () => new Set(shell.dismissedSuggestions ?? []),
    [shell.dismissedSuggestions],
  );
  const detectedHandoffPath = useMemo(() => {
    if (!sessionItems) return null;
    for (let i = sessionItems.length - 1; i >= 0; i--) {
      const item = sessionItems[i];
      if (item.kind !== "tool") continue;
      const doc = docFromTool(item.name, item.input);
      if (doc?.kind === "handoff" && !dismissedSet.has(doc.path)) return doc.path;
    }
    return null;
  }, [sessionItems, dismissedSet]);
  // The parent callback's identity churns; a ref keeps the effect deps tight to
  // the input that actually matters (the detected path), so we don't re-fire on
  // every parent render.
  const attachHandoffRef = useRef(onAttachHandoffDoc);
  attachHandoffRef.current = onAttachHandoffDoc;
  useEffect(() => {
    if (!detectedHandoffPath) return;
    if (shell.docs.some((d) => d.path === detectedHandoffPath)) return;
    attachHandoffRef.current(detectedHandoffPath);
  }, [detectedHandoffPath, shell.docs]);
  const showHandoffSuggest =
    started &&
    !terminal &&
    !handoffDoc &&
    !shell.handoffNudgeDismissed &&
    contextPct >= HANDOFF_NUDGE_THRESHOLD;
  const showHandoffReady = Boolean(handoffDoc) && started && !terminal;
  // True once /create_handoff has been sent (queued mid-turn or already in the
  // transcript) but before the handoff doc shows up — disables the suggest
  // button so a double-click can't queue a second /create_handoff.
  const HANDOFF_COMMAND = "/create_handoff";
  const handoffRequested =
    session != null &&
    (session.queued.some((m) => m.text.trim() === HANDOFF_COMMAND) ||
      session.items.some((it) => it.kind === "user" && it.text.trim() === HANDOFF_COMMAND));

  // Composer autocomplete: `/skill` commands and `@file` references, plus the
  // recognized-skill highlight painted behind the textarea text.
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [composerScroll, setComposerScroll] = useState(0);

  // Auto-grow the composer from a single row to fit its content (capped at
  // MAX_COMPOSER_HEIGHT, then it scrolls). The composer is anchored at the
  // bottom of the column, so a taller textarea pushes its own top edge up
  // toward the transcript — the box grows *upward*, keeping the whole draft in
  // view. Reset to one row is automatic: send/start clears `draft`, re-running
  // this. `height:auto` first so the box can shrink as well as grow.
  const autosizeComposer = useCallback(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, MAX_COMPOSER_HEIGHT)}px`;
  }, []);
  useLayoutEffect(autosizeComposer, [autosizeComposer, draft]);
  // Re-wrapping on a width change (split-pane drag, toolbar compaction) changes
  // how tall the text needs to be, so recompute then too. Guarded on width:
  // our own height mutations don't touch the width, so this can't loop.
  const composerWidthRef = useRef(0);
  useLayoutEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0].contentRect.width;
      if (w === composerWidthRef.current) return;
      composerWidthRef.current = w;
      autosizeComposer();
    });
    ro.observe(ta);
    return () => ro.disconnect();
  }, [autosizeComposer]);

  // Grab focus when something swaps this agent back on screen (e.g. the terminal
  // is toggled away) so the user can type straight away. We consume the signal
  // inside the frame, not in the effect body, so React 19 StrictMode's
  // double-invoke (setup→cleanup→setup) can't consume-then-cancel and drop the
  // focus — only the surviving frame claims it, exactly once.
  const pendingFocusShellId = useComposerFocusStore((s) => s.pendingShellId);
  useEffect(() => {
    if (pendingFocusShellId !== shell.id) return;
    const raf = requestAnimationFrame(() => {
      if (consumeComposerFocus(shell.id)) textareaRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [pendingFocusShellId, shell.id]);
  const autocomplete = useComposerAutocomplete({
    workstreamId,
    value: draft,
    onChange: onDraftChange,
    textareaRef,
    enabled: !pendingQuestion,
    // Picking from the @file dropdown stages the file as an attachment card
    // (same lane as drop/paste/dialog) instead of leaving a raw path in the
    // prompt. The search returns worktree-relative paths; the agent needs an
    // absolute one.
    onPickFile: (rel) => {
      const root = view.item.worktreePath.replace(/\/+$/, "");
      onAddAttachments([{ path: `${root}/${rel}`, name: basename(rel), kind: "file" }]);
    },
  });

  const handleStart = async () => {
    const input = draft.trim();
    if (!canSubmit || starting) return;
    setStarting(true);
    setError(null);
    try {
      // Library docs and staged files/screenshots both ride along as pin cards
      // in the echo (so no raw path shows) and as paths in the backend prompt.
      const atts: Attachment[] = [
        ...shell.docs.map((d) => ({ path: d.path, name: chipLabel(d.path), kind: "file" as const })),
        ...attachments,
      ];
      const created = await startSession({
        workstreamId: view.item.id,
        stage: shell.mode,
        input,
        attachments: atts,
        harness: harness.key,
        model: startModelFlag,
        effort: startEffort,
        permissionMode: permissionMode.flag,
      });
      onSessionStarted(created.id);
      // Auto-title this tab from the first prompt (best-effort, fire-and-forget;
      // a no-op for already-locked tabs and when the feature is off). Skip
      // attachment-only starts — there's no prompt text to summarize.
      if (input) {
        void generateTabTitle({ workstreamId: view.item.id, shellId: shell.id, input });
      }
      onDraftChange("");
      setHistoryNav(IDLE_NAV);
      recalledValueRef.current = null;
      onClearAttachments();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  const handleSend = async () => {
    const text = draft.trim();
    // No `busy` guard: while a turn is in flight `sendMessage` queues the message
    // and delivers it when the turn finishes (conversation steering).
    if ((!text && attachments.length === 0) || !sid) return;
    setError(null);
    try {
      await sendMessage(sid, text, attachments);
      // Sending is an explicit "bring me back to the conversation" — re-stick to
      // the bottom so the user sees their message (and the reply) even if they'd
      // scrolled up. No-ops when already at the bottom.
      transcriptRef.current?.scrollToBottom();
      onDraftChange("");
      setHistoryNav(IDLE_NAV);
      recalledValueRef.current = null;
      onClearAttachments();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  // Clear context: detach this tab from its session so the next message opens a
  // brand-new claude conversation (empty context — the manual escape hatch
  // before auto-compaction kicks in). A live session is ended first so it can't
  // keep running headless; the typed draft is kept so it can be resent fresh.
  // `skipConfirm` is set when the user is already on the explicit "Clear &
  // resume" path (handoff banner) — the create-handoff step is implicit consent.
  const handleClear = async (opts: { skipConfirm?: boolean } = {}) => {
    if (!started) return;
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && !opts.skipConfirm) {
      const message = handoffDoc
        ? "Clear this conversation and start fresh? The captured handoff doc will attach to the next session in this tab."
        : "Clear this conversation and start fresh? The current session ends and its context is discarded.";
      const ok = await confirm(message, { title: "Clear context?", kind: "warning" });
      if (!ok) return;
    }
    if (live && sid) await killSession(sid).catch(() => {});
    onResetSession();
  };

  // "Clear & resume" path triggered from the handoff-ready banner. The handoff
  // doc is already pinned to `shell.docs` (so it'll ride as an attachment to the
  // fresh session); also prefill `/resume_handoff` so a single Enter resumes.
  const handleClearAndResume = async () => {
    await handleClear({ skipConfirm: true });
    if (draft.trim() === "") onDraftChange("/resume_handoff");
  };

  // Permission mode is a launch-time decision (claude won't switch a running
  // session into bypass), so changing it on a LIVE session restarts the process
  // via --resume with the new mode — the transcript is preserved. We confirm
  // first only when a turn is in flight, since the restart interrupts it. Before
  // a session starts (or for a detached one) we just record the choice; it
  // applies at the next spawn/resume.
  const handlePermissionModeChange = async (key: string) => {
    const opt = permissionModeOption(key);
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && session && opt.flag !== session.permissionMode && isBusy(session)) {
      const ok = await confirm(
        `Restart this session to apply “${opt.label}”? The active turn will be interrupted.`,
        { title: "Restart session?", kind: "warning" },
      );
      if (!ok) return; // declined — leave the running session (and picker) as-is
    }
    onPermissionModeChange(key); // persist on the tab (badge + next fresh session)
    if (live && sid) await applyPermissionMode(sid, opt.flag);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // The autocomplete owns arrows / Enter / Tab / Escape while its menu is open
    // (accepting a pick must not also send the message).
    if (autocomplete.handleKeyDown(e)) return;
    // Cycle the agent stage (default Shift+Tab) while the mode picker is live —
    // once the session starts the stage is fixed at spawn, so it's a no-op. We
    // only take a chord WITHOUT a primary modifier: that's exactly the case the
    // global dispatcher skips in an editable target, so matching it here covers
    // the composer without double-firing on a remapped ⌘/Ctrl/Alt chord.
    if (
      !started &&
      cycleStageChord &&
      !hasPrimaryModifier(cycleStageChord) &&
      eventToChord(e.nativeEvent, IS_MACOS) === cycleStageChord
    ) {
      e.preventDefault();
      onModeChange(nextModeStage(mode));
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!started) void handleStart();
      else void handleSend();
    }
  };

  // Stable across renders (deps are the session id + the draft setter) so the
  // memoized transcript rows that receive these — UserMessage, PermissionCard —
  // aren't forced to re-render on every streamed token.
  const permission: PermissionHandlers = useMemo(
    () => ({
      onAllow: (rid) => sid && void respondPermission(sid, rid, true, null),
      onAllowForSession: (rid, tool) => sid && void allowForSession(sid, rid, tool),
      onDeny: (rid) => sid && void respondPermission(sid, rid, false, null),
    }),
    [sid],
  );

  const prompt: PromptHandlers = useMemo(
    () => ({
      onRewind: (itemId, text) => {
        if (!sid) return;
        // Land the user back on the prompt they're rewinding to (Claude-Desktop
        // behaviour): the composer is repopulated with the original text so they
        // can edit and re-send, while the backend truncates the conversation in
        // parallel.
        onDraftChange(text);
        // The first turn has nothing before it for claude to rewind to, so an
        // in-place truncation is impossible — a log-only prune there would leave
        // the model remembering everything. Rewinding it means "start over", so
        // reset the tab to a fresh session: the next send begins a brand-new
        // conversation the model has no memory of.
        if (isFirstUserTurn(sid, itemId)) {
          onResetSession();
        } else {
          void rewindToMessage(sid, itemId);
        }
      },
    }),
    [sid, onDraftChange, onResetSession],
  );

  return (
    <div ref={dropRef} className="relative flex min-h-0 flex-1 flex-col">
      {dragOver && (
        <div className="pointer-events-none absolute inset-0 z-40 m-2 flex items-center justify-center rounded-xl border-2 border-dashed border-accent bg-accent/10">
          <div className="flex items-center gap-2 rounded-lg bg-panel px-3.5 py-2 text-sm font-medium text-ink shadow-lg">
            <ImageIcon size={16} />
            Drop to attach
          </div>
        </div>
      )}
      {/* Prompt-history stack: floats in the pane's top-right corner over the
          transcript gutter. Gated to states where the composer is actually
          editable (not terminal / not blocked on a question), since recall
          targets the composer draft. */}
      {!terminal && !pendingQuestion && prompts.length > 0 && (
        <PromptHistoryStack
          prompts={prompts}
          activeIndex={historyNav.index}
          onRecall={(i) => applyRecall(recallAt(prompts, historyNav, draft, i))}
        />
      )}
      {started ? (
        <TranscriptView
          ref={transcriptRef}
          items={session?.items ?? []}
          busy={working}
          connecting={!session || session.state === "starting"}
          harnessLabel={harness.label}
          workingSince={session?.workingSince ?? null}
          permission={permission}
          prompt={prompt}
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          <div className="mx-auto w-[90%]">
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

      <div className="p-4">
        <div className="mx-auto w-[90%]">
          {error && (
            <div className="mb-2">
              <ErrorNote error={{ code: "session", message: error, remedy: null }} />
            </div>
          )}

          {terminal ? (
            <div className="flex items-center justify-between gap-3 rounded-md border border-edge bg-panel-2 px-3 py-2.5">
              <span className="text-sm text-ink-dim">
                {resuming
                  ? "Resuming the previous session…"
                  : "This session has ended. Resume to continue, or start fresh."}
              </span>
              <div className="flex items-center gap-2">
                <Button onClick={onResetSession} disabled={resuming}>
                  New session
                </Button>
                <Button
                  variant="primary"
                  busy={resuming}
                  disabled={!sid || resuming}
                  onClick={async () => {
                    if (!sid) return;
                    setError(null);
                    setResuming(true);
                    try {
                      await resumeSession(sid);
                    } catch (e) {
                      setError(errorMessage(e));
                    } finally {
                      setResuming(false);
                    }
                  }}
                >
                  Resume
                </Button>
              </div>
            </div>
          ) : (
            <>
              {/* The plan drawer floats up from the composer over this area;
                  keep these banners in their own stacking context so the drawer
                  never covers the handoff box or a pending question. */}
              <div className="relative z-30">
                {(showHandoffSuggest || showHandoffReady) && (
                  <HandoffBanner
                    mode={showHandoffReady ? "ready" : "suggest"}
                    contextPct={contextPct}
                    suggestRequested={handoffRequested}
                    onCreateHandoff={() => {
                      if (sid && !handoffRequested) void sendMessage(sid, HANDOFF_COMMAND, []);
                    }}
                    onClearAndResume={() => void handleClearAndResume()}
                    onDismiss={
                      showHandoffReady && handoffDoc
                        ? () => onDetachDoc(handoffDoc.path)
                        : onDismissHandoffNudge
                    }
                  />
                )}
                {pendingQuestion && (
                  <QuestionPrompt
                    key={pendingQuestion.requestId}
                    pending={pendingQuestion}
                    onSubmit={(answers) =>
                      sid && void answerQuestion(sid, pendingQuestion.requestId, answers)
                    }
                  />
                )}
                {session && session.queued.length > 0 && (
                  <QueuedMessages
                    messages={session.queued}
                    onCancel={(i) => {
                      if (sid) cancelQueuedMessage(sid, i);
                    }}
                  />
                )}
              </div>
              {/* Top margin reserves room for the pinned prompt chips, which
                  float on their own strip just above the composer (absolute
                  bottom-full right) so they clear the banners above (queued
                  messages / handoff nudge) with breathing room. */}
              <div
                className={`relative rounded-xl bg-panel-2 ${
                  pinnedPrompts.length > 0 ? "mt-12" : ""
                }`}
              >
                {pinnedPrompts.length > 0 && (
                  <PromptChips
                    view={view}
                    shellId={shell.id}
                    className="absolute bottom-full right-3 z-20 mb-1.5 max-w-[70%]"
                  />
                )}
                <AutocompleteDropdown state={autocomplete} />
                {/* Staged context lives INSIDE the composer as cards, consistent
                    whether or not the session has started. Pre-start, the app's
                    automatic docs (the mode's auto-suggestion, a captured handoff)
                    show as chips above the per-turn attachment cards — the picked
                    docs and files the user staged. */}
                {(attachments.length > 0 || (!started && shell.docs.length > 0)) && (
                  <div className="space-y-1.5 px-3.5 pt-3">
                    {!started && (
                      <WorkstreamDocChips
                        docs={shell.docs}
                        onOpenDoc={onOpenDoc}
                        onDetach={onDetachDoc}
                      />
                    )}
                    <AttachmentCards attachments={attachments} onRemove={onRemoveAttachment} />
                  </div>
                )}
                <div className="relative">
                  <HighlightBackdrop
                    value={draft}
                    commandNames={autocomplete.commandNames}
                    scrollTop={composerScroll}
                  />
                  <textarea
                    ref={textareaRef}
                    value={draft}
                    onChange={(e) => {
                      const val = e.target.value;
                      // A change we didn't originate via recall = a user edit: leave
                      // navigation so further typing builds on this text and the stash
                      // is dropped.
                      if (historyNav.index !== null && val !== recalledValueRef.current) {
                        setHistoryNav(IDLE_NAV);
                      }
                      recalledValueRef.current = null;
                      onDraftChange(val);
                      autocomplete.syncCaret();
                    }}
                    onFocus={() => setComposerFocused(true)}
                    onBlur={() => setComposerFocused(false)}
                    onKeyDown={onKeyDown}
                    onPaste={onPaste}
                    onSelect={autocomplete.syncCaret}
                    onScroll={(e) => setComposerScroll(e.currentTarget.scrollTop)}
                    rows={1}
                    disabled={Boolean(pendingQuestion)}
                    placeholder={
                      pendingQuestion
                        ? "Answer the question above to continue…"
                        : !started
                          ? mode === "general"
                            ? "Ask the project agent…  (Enter to start, Shift+Enter for a newline)"
                            : `Brief the ${stageLabel(mode).toLowerCase()} agent…  (Enter to start)`
                          : busy
                            ? "Send a follow-up — it'll queue until the turn finishes…"
                            : "Reply to the agent…  (Enter to send)"
                    }
                    className="selectable relative block w-full resize-none bg-transparent px-3.5 py-4 text-sm text-ink placeholder:text-ink-dim/60 focus:outline-none disabled:cursor-not-allowed"
                  />
                </div>
                {/* Single row, never wrapping: when the labelled pickers don't
                    fit, the layout effect above flips compactComposer so they
                    collapse to icons and everything stays on one line. We clip
                    the horizontal axis only (overflow-x) to hide the one
                    pre-collapse layout frame so the Send/attach cluster can't
                    bleed past the pane edge — overflow-y stays visible so the
                    pickers' upward dropdowns (bottom-full) aren't clipped. */}
                <div ref={toolbarRef} className="flex items-center gap-0.5 overflow-x-clip px-2 pb-2">
                  {/* Mode is fixed at spawn → a badge once running; a picker before. */}
                  {started ? (
                    <span
                      title={compactComposer ? stageLabel(mode) : undefined}
                      className={`inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded px-2 py-1 text-xs font-medium ${MODE_STYLES[mode].badge}`}
                    >
                      <ModeIcon mode={mode} size={13} />
                      {!compactComposer && stageLabel(mode)}
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
                      compact={compactComposer}
                    />
                  )}
                  {/* Model picker, grouped by harness — picking a model also picks
                      its harness. Mid-session changes take effect on the next
                      session this tab starts. The "Harnesses →" footer opens the
                      settings where providers/models are configured. */}
                  <ComposerMenu
                    title="Model"
                    leading={composerMark(harness.key, 14)}
                    label={composerModelLabel}
                    hint={composerModelHint}
                    groups={modelGroups}
                    activeKey={composerModelActiveKey}
                    onSelect={onPickModel}
                    footer={{
                      label: "Harnesses",
                      icon: <ChevronRightIcon size={14} />,
                      onClick: onOpenHarnessSettings,
                    }}
                    compact={compactComposer}
                  />
                  {/* Effort + permission mode apply to Claude and Codex; OpenCode
                      manages its own reasoning/permission settings. Codex has its
                      own effort ladder, so it gets its own picker rather than
                      being offered a Claude-only level the app-server rejects. */}
                  {usingCodex && (
                    <ComposerMenu
                      title="Reasoning effort"
                      leading={<EffortIcon size={14} />}
                      label={codexEffort.label}
                      items={codexEffortItems.map((e) => ({ key: e.key, label: e.label }))}
                      activeKey={codexEffort.key}
                      onSelect={onCodexEffortChange}
                      compact={compactComposer}
                    />
                  )}
                  {!usingOpencode && !usingCodex && (
                    <ComposerMenu
                      title="Reasoning effort"
                      leading={<EffortIcon size={14} />}
                      label={effort.label}
                      items={EFFORTS.map((e) => {
                        // Ultracode needs an xhigh-capable model; grey it out (with
                        // the reason) on any other model so it can't be picked into
                        // a session the CLI would reject.
                        const blocked = e.key === "ultracode" && !modelSupportsUltracode(model);
                        return {
                          key: e.key,
                          label: e.label,
                          hint: blocked ? "needs Opus 4.7+, Sonnet 5, or Fable 5" : undefined,
                          disabled: blocked,
                          // Accent-tint Ultracode so it's spottable in the list and,
                          // when selected, in the composer trigger.
                          accent: e.key === "ultracode",
                        };
                      })}
                      activeKey={effort.key}
                      onSelect={onEffortChange}
                      compact={compactComposer}
                    />
                  )}
                  {!usingOpencode && (
                    <ComposerMenu
                      title="Permission mode"
                      leading={<ShieldIcon size={14} />}
                      label={permissionMode.label}
                      hint={permissionMode.hint}
                      items={PERMISSION_MODES.map((p) => ({
                        key: p.key,
                        label: p.label,
                        hint: p.hint,
                        danger: p.danger,
                      }))}
                      activeKey={permissionMode.key}
                      onSelect={(k) => void handlePermissionModeChange(k)}
                      compact={compactComposer}
                    />
                  )}

                  <div className="ml-auto flex items-center gap-1">
                    <AttachMenu
                      onLibrary={onBrowseDocs}
                      onFiles={onAddFiles}
                      onScreenshots={onAddScreenshots}
                    />
                    {/* Clear context: end this conversation and start fresh — the
                        manual way to dodge auto-compaction. Only meaningful once a
                        session exists. The handoff-ready state lights the button
                        amber + adds a slow pulse so the user knows the next step
                        is to clear and resume from the captured handoff doc. */}
                    {started && (
                      <button
                        type="button"
                        onClick={() => void handleClear()}
                        title={
                          showHandoffReady
                            ? "Clear context and resume from the handoff doc"
                            : "Clear context — end this conversation and start fresh"
                        }
                        aria-label="Clear context"
                        className={
                          showHandoffReady
                            ? "flex h-8 w-8 animate-pulse items-center justify-center rounded-md text-warn ring-1 ring-warn/50 hover:bg-warn/10"
                            : "flex h-8 w-8 items-center justify-center rounded-md text-ink-dim hover:bg-edge/50 hover:text-ink"
                        }
                      >
                        <EraserIcon />
                      </button>
                    )}
                    {/* Live background shells (Bash run_in_background): a pulsing
                        terminal icon + count that opens a per-task popover. Sits
                        by the context meter; renders nothing when none run. */}
                    <BackgroundTasksIndicator
                      tasks={backgroundTasks}
                      onJumpTo={(toolUseId) => transcriptRef.current?.scrollToTool(toolUseId)}
                    />
                    {/* Drop or paste a screenshot anytime — see the pane drop overlay. */}
                    {session?.contextTokens != null && (
                      <ContextMeter tokens={session.contextTokens} windowSize={contextWindow} />
                    )}
                    {/* Stop stays available while a turn runs; Send stays too, so
                        a follow-up can be queued mid-turn (conversation steering). */}
                    {busy && started && (
                      <button
                        type="button"
                        onClick={() => sid && void stopSession(sid)}
                        title="Stop the current turn (interrupt, then force-stop if it doesn't respond)"
                        aria-label="Stop"
                        className="flex h-8 w-8 items-center justify-center rounded-md border border-bad/40 bg-bad/10 text-bad hover:bg-bad/20"
                      >
                        <StopIcon />
                      </button>
                    )}
                    <button
                      type="button"
                      disabled={!canSubmit || starting || Boolean(pendingQuestion)}
                      onClick={() => (started ? void handleSend() : void handleStart())}
                      title={
                        !started
                          ? "Start session"
                          : busy
                            ? "Queue message — sends when the turn finishes"
                            : "Send"
                      }
                      aria-label={!started ? "Start" : busy ? "Queue message" : "Send"}
                      className="flex h-8 w-8 items-center justify-center rounded-md border border-edge bg-edge/70 text-ink hover:bg-edge disabled:opacity-40 disabled:hover:bg-edge/70"
                    >
                      {starting ? <Spinner size={14} /> : <SendIcon size={15} />}
                    </button>
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
