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

import { Badge, Button, ErrorNote, MenuItem, MenuSeparator, Popover, Spinner } from "@/components/ui";
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
import {
  useBootstrap,
  useClaudeProbe,
  useCodexProbe,
  useOpencodeCatalog,
  useSyncClaudeModels,
  useWorkstreamSessions,
} from "@/lib/queries";
import type { CustomPrompt, WorkstreamView } from "@/lib/bindings";
import { chipLabel, WorkstreamDocChips } from "@/features/workstreams/WorkstreamDocChips";
import { PromptChips, usePinnedPrompts } from "@/features/prompts/PromptChips";
import {
  pendingComposerArg,
  prefillSlashCommand,
  promptInputLabel,
  promptSlug,
} from "@/features/prompts/prompts";
import { resolveCustomPrompt, runCustomPrompt } from "@/features/prompts/runPrompt";
import { AttachmentCards } from "./AttachmentCard";
import { fitComposerHeight, MAX_COMPOSER_HEIGHT } from "./composerAutosize";
import {
  importDroppedPaths,
  saveClipboardImage,
  saveClipboardImageFromOs,
  type Attachment,
} from "./attachments";
import { consumeComposerFocus, useComposerFocusStore } from "./workstreamTabs";
import { generateTabTitle } from "./tabTitles";
import type { ShellTab } from "./workstreamTabs";
import { MODE_STAGES, MODE_STYLES, asStageKey, nextModeStage, stageLabel, suggestedDocKind, type StageKey } from "@/lib/domain/stages";
import {
  claudeModels,
  codexEfforts,
  effectiveEffort,
  effortsForModel,
  modelOption,
  modelSupportsUltracode,
  resolveCodexEffort,
  resolveCodexModelIds,
  resumeModelFlag,
  visibleModels,
  type ModelOption,
} from "@/lib/domain/models";
import {
  effectiveHarnessKey,
  firstSupportedOpencodeModel,
  harnessOption,
  lockedSessionProvider,
  modelProvider,
  opencodeModelLabel,
  opencodeModelNames,
} from "@/lib/domain/harnessing";
import { PERMISSION_MODES, permissionModeOption } from "@/lib/domain/permissions";
import { detectHandoffDoc, formatTokens } from "./toolDisplay";
import { useWrittenDocs } from "./writtenDocs";
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

export function SessionTabIcon({ mode, sessionId }: { mode: StageKey; sessionId?: string }) {
  const session = useSession(sessionId);
  const working = session ? isWorking(session) : false;
  return (
    <span className={`rounded p-0.5 ${MODE_STYLES[mode].badge} ${working ? "animate-tab-glow" : ""}`}>
      <ModeIcon mode={mode} />
    </span>
  );
}

type ComposerMenuOption = {
  key: string;
  label: string;
  hint?: string;
  icon?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  disabledReason?: string;
  accent?: boolean;
};

type MenuGroup = {
  key: string;
  label: string;
  icon?: ReactNode;
  items: ComposerMenuOption[];
  disabled?: boolean;
  more?: { label: string; items: ComposerMenuOption[] };
};

function composerMark(key: string, size = 13) {
  if (key === "codex") return <OpenAiIcon size={size} />;
  if (key === "opencode") return <CodeIcon size={size} />;
  return <ClaudeMarkIcon size={size} />;
}

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
  items?: ComposerMenuOption[];
  groups?: MenuGroup[];
  footer?: { label: string; icon?: ReactNode; onClick: () => void };
  activeKey: string;
  onSelect: (key: string) => void;
  title: string;
  custom?: { label: string; placeholder?: string };
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(false);
  const [expandedGroups, setExpandedGroups] = useState<ReadonlySet<string>>(new Set());
  const [text, setText] = useState("");
  const allItems = groups
    ? groups.flatMap((g) => (g.disabled ? [] : g.items.filter((it) => !it.disabled)))
    : items;
  const customActive = custom != null && !allItems.some((it) => it.key === activeKey);
  const activeDanger = allItems.some((it) => it.key === activeKey && it.danger);
  const activeAccent = allItems.some((it) => it.key === activeKey && it.accent);

  const close = () => {
    setOpen(false);
    setEditing(false);
    setText("");
    setExpandedGroups(new Set());
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
      e.stopPropagation();
      setEditing(false);
    }
  };

  return (
    <div className="relative">
      <button
        ref={triggerRef}
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
      <Popover
        open={open}
        onClose={close}
        anchorRef={triggerRef}
        side="top"
        align="start"
        className="max-h-[60vh] min-w-44 max-w-[22rem] overflow-y-auto overflow-x-hidden"
      >
        {groups ? (
          <>
            {groups.map((g, gi) => {
              const renderItem = (it: ComposerMenuOption) =>
                g.disabled || it.disabled ? (
                  <MenuItem
                    key={it.key}
                    disabled
                    title={
                      g.disabled
                        ? undefined
                        : (it.disabledReason ??
                          "Locked to this session's provider. Start a new agent to use a different provider")
                    }
                  >
                    <span className="flex w-full items-center gap-2 text-ink-dim/40">
                      {it.icon && <span className="shrink-0">{it.icon}</span>}
                      <span className="min-w-0 flex-1 truncate">{it.label}</span>
                      {it.hint && <span className="shrink-0 text-[10px] text-ink-dim/70">{it.hint}</span>}
                      {g.disabled && (
                        <span className="ml-auto shrink-0 text-[10px] uppercase tracking-wide">soon</span>
                      )}
                    </span>
                  </MenuItem>
                ) : (
                  <MenuItem
                    key={it.key}
                    onClick={() => {
                      onSelect(it.key);
                      close();
                    }}
                    className={it.key === activeKey ? "text-ink" : "text-ink-dim"}
                  >
                    <span className="flex w-full items-center gap-2">
                      {it.icon && <span className="shrink-0">{it.icon}</span>}
                      <span className="min-w-0 flex-1 truncate">{it.label}</span>
                      {it.hint && <span className="shrink-0 text-ink-dim">{it.hint}</span>}
                      {it.key === activeKey && (
                        <span aria-hidden="true" className="ml-auto shrink-0">
                          ✓
                        </span>
                      )}
                    </span>
                  </MenuItem>
                );
              const showMore = g.more && (expandedGroups.has(g.key) || g.more.items.some((it) => it.key === activeKey));
              return (
                <div key={g.key}>
                  {gi > 0 && <MenuSeparator />}
                  <div className="flex items-center gap-2 px-2.5 py-1 text-[11px] font-medium text-ink-dim/70">
                    {g.icon && <span className="shrink-0">{g.icon}</span>}
                    <span>{g.label}</span>
                  </div>
                  {g.items.map(renderItem)}
                  {g.more && g.more.items.length > 0 && (
                    <>
                      <MenuItem
                        onClick={() =>
                          setExpandedGroups((prev) => {
                            const next = new Set(prev);
                            if (!next.delete(g.key)) next.add(g.key);
                            return next;
                          })
                        }
                        className="text-ink-dim"
                      >
                        <span className="flex w-full items-center gap-2">
                          <span className="shrink-0">
                            {showMore ? <ChevronDownIcon size={14} /> : <ChevronRightIcon size={14} />}
                          </span>
                          <span className="min-w-0 flex-1 truncate">{g.more.label}</span>
                          <span className="ml-auto shrink-0 text-[10px] text-ink-dim/70">
                            {g.more.items.length}
                          </span>
                        </span>
                      </MenuItem>
                      {showMore && g.more.items.map(renderItem)}
                    </>
                  )}
                </div>
              );
            })}
            {footer && (
              <>
                <MenuSeparator />
                <MenuItem
                  onClick={() => {
                    footer.onClick();
                    close();
                  }}
                  className="text-ink-dim hover:text-ink"
                >
                  <span className="flex w-full items-center gap-2">
                    <span>{footer.label}</span>
                    {footer.icon && <span className="ml-auto shrink-0">{footer.icon}</span>}
                  </span>
                </MenuItem>
              </>
            )}
          </>
        ) : (
          <>
            {items.map((it) =>
              it.disabled ? (
                <MenuItem key={it.key} disabled title={it.hint || undefined}>
                  <span className="flex w-full items-center gap-2 text-ink-dim/40">
                    {it.icon && <span className="shrink-0">{it.icon}</span>}
                    <span>{it.label}</span>
                    {it.hint && <span>{it.hint}</span>}
                  </span>
                </MenuItem>
              ) : (
                <MenuItem
                  key={it.key}
                  onClick={() => {
                    onSelect(it.key);
                    close();
                  }}
                  danger={it.danger}
                  className={
                    it.danger
                      ? undefined
                      : it.accent
                        ? "text-accent hover:text-accent"
                        : it.key === activeKey
                          ? "text-ink"
                          : "text-ink-dim"
                  }
                >
                  <span className="flex w-full items-center gap-2">
                    {it.icon && <span className="shrink-0">{it.icon}</span>}
                    <span>{it.label}</span>
                    {it.hint && <span className="text-ink-dim">{it.hint}</span>}
                    {it.key === activeKey && (
                      <span aria-hidden="true" className="ml-auto">
                        ✓
                      </span>
                    )}
                  </span>
                </MenuItem>
              ),
            )}
            {custom && (
              <>
                <MenuSeparator />
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
                      className="w-full rounded-md border border-edge bg-base px-2 py-1 text-sm text-ink placeholder:text-ink-dim focus:border-accent focus:outline-none"
                    />
                  </div>
                ) : (
                  <MenuItem onClick={beginCustom} className={customActive ? "text-ink" : "text-ink-dim"}>
                    <span className="flex w-full items-center gap-2">
                      <span>{customActive ? activeKey : custom.label}</span>
                      {customActive && (
                        <span aria-hidden="true" className="ml-auto">
                          ✓
                        </span>
                      )}
                    </span>
                  </MenuItem>
                )}
              </>
            )}
          </>
        )}
      </Popover>
    </div>
  );
}

function AttachMenu({
  onLibrary,
  onFiles,
  onScreenshots,
}: {
  onLibrary: (() => void) | null;
  onFiles: () => void;
  onScreenshots: () => void;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const items = [
    ...(onLibrary
      ? [{ key: "library", label: "Attach from library", icon: <BookIcon size={14} />, run: onLibrary }]
      : []),
    { key: "files", label: "Add files…", icon: <DocIcon size={14} />, run: onFiles },
    { key: "shots", label: "Add screenshots…", icon: <ImageIcon size={14} />, run: onScreenshots },
  ];
  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        title="Attach context"
        aria-label="Attach context"
        className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
      >
        <PlusIcon />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="min-w-44"
      >
        {items.map((it) => (
          <MenuItem
            key={it.key}
            onClick={() => {
              setOpen(false);
              it.run();
            }}
            className="text-ink-dim hover:text-ink"
          >
            <span className="flex w-full items-center gap-2">
              <span className="shrink-0">{it.icon}</span>
              <span>{it.label}</span>
            </span>
          </MenuItem>
        ))}
      </Popover>
    </div>
  );
}

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

function ContextMeter({ tokens, windowSize }: { tokens: number; windowSize: number }) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const pct = Math.min(1, tokens / windowSize);
  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={`Context ${formatTokens(tokens)} / ${formatTokens(windowSize)}`}
        aria-label="Context usage"
        className="flex h-6 items-center gap-1.5 rounded px-1.5 text-ink-dim hover:bg-edge/60 hover:text-ink"
      >
        <ContextRing pct={pct} />
        <span className="font-mono text-[11px]">{formatTokens(tokens)}</span>
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="w-64 p-3"
      >
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
      </Popover>
    </div>
  );
}

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
          <>Handoff ready. Clear context to start a fresh session with the handoff attached.</>
        ) : suggestRequested ? (
          <>Handoff queued. The agent writes it on the next turn.</>
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
        title={isReady ? "Dismiss, keep working in this session" : "Dismiss"}
        aria-label="Dismiss"
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge hover:text-ink"
      >
        <XIcon size={10} strokeWidth={2.4} />
      </button>
    </div>
  );
}

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

function QuestionPrompt({
  pending,
  onSubmit,
}: {
  pending: PendingQuestion;
  onSubmit: (answers: Record<string, string>) => void;
}) {
  const [selections, setSelections] = useState<Record<string, string[]>>({});
  const [custom, setCustom] = useState<Record<string, string>>({});
  const [step, setStep] = useState(0);
  const [submitted, setSubmitted] = useState(false);

  const send = (answers: Record<string, string>) => {
    if (submitted) return;
    setSubmitted(true);
    onSubmit(answers);
  };

  const total = pending.questions.length;
  const question = pending.questions[step];

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
  const otherText = custom[question.question];
  const otherOpen = otherText !== undefined;
  const currentAnswered = chosen.length > 0 || (otherText ?? "").trim().length > 0;
  const isLast = step === total - 1;

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
    if (!question.multiSelect) closeOther();
  };

  const toggleOther = () => {
    if (otherOpen) {
      closeOther();
      return;
    }
    setCustom((prev) => ({ ...prev, [question.question]: "" }));
    if (!question.multiSelect) setSelections((prev) => ({ ...prev, [question.question]: [] }));
  };

  const submitAll = () => send(buildQuestionAnswers(pending.questions, selections, custom));

  const advance = () => {
    if (!currentAnswered) return;
    if (isLast) submitAll();
    else setStep((s) => s + 1);
  };

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
  onHarnessChange: (key: string) => void;
  onOpencodeModelChange: (key: string) => void;
  onCodexModelChange: (key: string) => void;
  onCodexEffortChange: (key: string) => void;
  onOpenHarnessSettings: () => void;
  onSessionStarted: (sessionId: string) => void;
  onResetSession: () => void;
  onOpenDoc: (id: string) => void;
  onDetachDoc: (path: string) => void;
  onSuggestDoc: (path: string | null, role: string) => void;
  onDismissHandoffNudge: () => void;
  onAttachHandoffDoc: (path: string) => void;
  onBrowseDocs: (() => void) | null;
  onAddFiles: () => void;
  onAddScreenshots: () => void;
  onAddAttachments: (attachments: Attachment[]) => void;
  onRemoveAttachment: (path: string) => void;
  onClearAttachments: () => void;
};

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
  const addAttachmentsRef = useRef(onAddAttachments);
  addAttachmentsRef.current = onAddAttachments;
  const workstreamId = view.item.id;

  const toolbarRef = useRef<HTMLDivElement>(null);
  const naturalWidthRef = useRef(0);
  const [compactComposer, setCompactComposer] = useState(false);
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
  const cycleStageChord = effectiveChord(
    useBootstrap().data?.settings.keybindings?.overrides ?? {},
    "ws.cycleStage",
    IS_MACOS,
  );
  const pinnedPrompts = usePinnedPrompts(view.item.projectPath);
  const claudeDefaults = useBootstrap().data?.settings.providers?.claude;
  const claudeProbe = useClaudeProbe();
  const claudeCatalog = claudeModels(claudeProbe.data?.models);
  useSyncClaudeModels(
    claudeCatalog,
    !claudeProbe.isPending || claudeProbe.fetchStatus === "paused",
    Boolean(claudeProbe.data?.models.length),
  );
  const model = modelOption(shell.model ?? claudeDefaults?.defaultModel ?? undefined, claudeCatalog);
  const effort = effectiveEffort(shell.effort ?? claudeDefaults?.defaultEffort ?? undefined, model);
  const claudeEffortItems = effortsForModel(model);
  const permissionMode = permissionModeOption(
    shell.permissionMode ?? (claudeDefaults?.defaultPlanMode ? "plan" : undefined),
  );
  const modelItems = visibleModels(claudeDefaults?.visibleModels, claudeCatalog);

  const bootstrap = useBootstrap().data;
  const bootstrapSettings = bootstrap?.settings;
  const opencodeDefaults = bootstrapSettings?.providers?.opencode;
  const ollamaDefaults = bootstrapSettings?.providers?.ollama;
  const codexDefaults = bootstrapSettings?.providers?.codex;
  const codexInstalled = (bootstrap?.binaries ?? []).some(
    (b) => b.kind === "codex" && b.state.state === "ok",
  );
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
  const ocSelected = opencodeDefaults?.selectedModels ?? [];
  const ollamaModels = (ollamaDefaults?.selectedModels ?? []).map((id) => `ollama/${id}`);
  const ocCatalog = useOpencodeCatalog(ocSelected.length > 0);
  const ocKnown = useMemo(() => {
    const providers = ocCatalog.data?.providers;
    if (!providers?.some((p) => p.available != null)) return null;
    const ids = new Set<string>();
    for (const p of providers) for (const m of p.models) ids.add(`${p.id}/${m.id}`);
    return ids;
  }, [ocCatalog.data]);
  const ocUsable = (id: string | null | undefined) =>
    id != null && ocKnown != null && !ocKnown.has(id) ? null : id;
  const ocModels = ocKnown ? ocSelected.filter((id) => ocKnown.has(id)) : ocSelected;
  const ocModel = firstSupportedOpencodeModel(
    ocUsable(shell.opencodeModel),
    ocUsable(opencodeDefaults?.defaultModel),
    ollamaDefaults?.defaultModel,
    ...ocModels,
    ...ollamaModels,
  );
  const codexCatalog = useCodexProbe(codexInstalled);
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
  const startEffort = usingCodex ? codexEffort.key : usingOpencode ? null : (effort?.key ?? null);
  const ocNames = useMemo(
    () => opencodeModelNames(ocCatalog.data?.providers ?? []),
    [ocCatalog.data],
  );

  const lockedProvider = lockedSessionProvider(started, harness.key, ocModel);
  const lockOther = (itemProvider: string): Pick<ComposerMenuOption, "disabled"> =>
    lockedProvider !== null && itemProvider !== lockedProvider ? { disabled: true } : {};

  const claudeRow = (m: ModelOption): ComposerMenuOption => ({
    key: `claude:${m.key}`,
    label: m.label,
    hint: m.disabled ? (m.description ?? "unavailable") : m.hint,
    icon: composerMark("claude"),
    ...(m.disabled
      ? { disabled: true, disabledReason: m.description ?? "Unavailable in this CLI version" }
      : lockOther("claude")),
  });

  const modelGroups: MenuGroup[] = [
    {
      key: "claude",
      label: "Claude Code",
      icon: composerMark("claude"),
      items: modelItems.filter((m) => !m.unlisted).map(claudeRow),
      more: {
        label: "More models",
        items: modelItems.filter((m) => m.unlisted).map(claudeRow),
      },
    },
    {
      key: "codex",
      label: "Codex",
      icon: composerMark("codex"),
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

  const applyClaudeModel = async (key: string) => {
    if (key === model.key) return;
    const next = modelOption(key, claudeCatalog);
    if (next.disabled) return;
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && session && isBusy(session)) {
      const ok = await confirm(
        `Restart this session to switch to ${next.label}? The active turn will be interrupted.`,
        { title: "Switch model?", kind: "warning" },
      );
      if (!ok) return;
    }
    onModelChange(key);
    if (live && sid) {
      const nextEffort = effectiveEffort(
        shell.effort ?? claudeDefaults?.defaultEffort ?? undefined,
        next,
      );
      await applyModel(sid, resumeModelFlag(next), nextEffort?.key ?? null, permissionMode.flag);
    }
  };

  const onPickModel = (namespaced: string) => {
    const sep = namespaced.indexOf(":");
    const pickedHarness = namespaced.slice(0, sep);
    const pickedModel = namespaced.slice(sep + 1);
    const pickedProvider = modelProvider(pickedHarness, pickedModel);
    if (lockedProvider !== null && pickedProvider !== lockedProvider) return;
    if (pickedHarness === "opencode") {
      onHarnessChange("opencode");
      onOpencodeModelChange(pickedModel);
    } else if (pickedHarness === "codex") {
      onHarnessChange("codex");
      onCodexModelChange(pickedModel);
    } else {
      onHarnessChange("claude");
      void applyClaudeModel(pickedModel);
    }
  };

  const restorePermissionFlag = permissionModeOption(shell.permissionMode).flag;
  useEffect(() => {
    if (shell.sessionId && !hasEntry) {
      void restoreSessionIfNeeded(shell.sessionId, view.item.id, shell.mode, restorePermissionFlag);
    }
  }, [shell.sessionId, hasEntry, view.item.id, shell.mode, restorePermissionFlag]);

  const autoResumedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!shell.sessionId) {
      autoResumedRef.current = null;
      return;
    }
    if (autoResumedRef.current === shell.sessionId) return;
    if (!session || !isTerminal(session.state)) return;
    if (session.items.length === 0) return;
    autoResumedRef.current = shell.sessionId;
    setError(null);
    setResuming(true);
    void resumeSession(shell.sessionId)
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setResuming(false));
  }, [shell.sessionId, session]);

  const suggestKind = suggestedDocKind(mode);
  const recent = useMostRecentDoc(workstreamId, suggestKind);
  const suggestDocRef = useRef(onSuggestDoc);
  suggestDocRef.current = onSuggestDoc;
  useEffect(() => {
    if (started) return;
    suggestDocRef.current(recent?.path ?? null, recent?.kind ?? "other");
  }, [started, recent?.path, recent?.kind]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    const within = (pos?: { x: number; y: number }) => {
      const el = dropRef.current;
      if (!el || !pos) return false;
      const r = el.getBoundingClientRect();
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
    if (images.length > 0) {
      e.preventDefault();
      void Promise.all(images.map((f) => saveClipboardImage(workstreamId, f))).then((res) => {
        const atts = res.filter((a): a is Attachment => a !== null);
        if (atts.length) onAddAttachments(atts);
      });
      return;
    }
    if (e.clipboardData?.getData("text/plain")) return;
    void saveClipboardImageFromOs(workstreamId).then((att) => {
      if (att) onAddAttachments([att]);
    });
  };

  const busy = session ? isBusy(session) : started;
  const working = session ? isWorking(session) : false;
  const terminal = session ? isTerminal(session.state) : false;
  const pendingQuestion = session?.pendingQuestion ?? null;
  const draft = shell.draft;
  const attachments = shell.attachments ?? [];
  const canSubmit = draft.trim().length > 0 || attachments.length > 0;
  const sid = shell.sessionId;

  const backgroundTasks = useSessionBackgroundTasks(sid);

  const prompts = useMemo(
    () => (session ? userPrompts(session.items) : []),
    [session],
  );
  const [historyNav, setHistoryNav] = useState<HistoryNav>(IDLE_NAV);
  const [composerFocused, setComposerFocused] = useState(false);
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
        ta.setSelectionRange(n, n);
      });
    },
    [onDraftChange],
  );

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

  const toggleTasks = useUiStore((s) => s.toggleTasks);
  useCommand("session.togglePlan", toggleTasks);

  const transcriptRef = useRef<TranscriptHandle | null>(null);

  const HANDOFF_NUDGE_THRESHOLD = 0.6;
  const contextWindow = session?.contextWindow ?? model.contextWindow;
  const contextPct =
    session?.contextTokens != null ? session.contextTokens / contextWindow : 0;
  const handoffDoc = shell.docs.find((d) => d.role === "handoff") ?? null;
  const sessionItems = session?.items;
  const dismissedSet = useMemo(
    () => new Set(shell.dismissedSuggestions ?? []),
    [shell.dismissedSuggestions],
  );
  const writtenDocs = useWrittenDocs(workstreamId);
  const detectedHandoffPath = useMemo(
    () => (sessionItems ? detectHandoffDoc(sessionItems, writtenDocs, dismissedSet) : null),
    [sessionItems, dismissedSet, writtenDocs],
  );
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
  const HANDOFF_COMMAND = "/create_handoff";
  const handoffRequested =
    session != null &&
    (session.queued.some((m) => m.text.trim() === HANDOFF_COMMAND) ||
      session.items.some((it) => it.kind === "user" && it.text.trim() === HANDOFF_COMMAND));

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [composerScroll, setComposerScroll] = useState(0);

  const autosizeComposer = useCallback(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    const paneHeight = ta.closest<HTMLElement>("[data-pane-content]")?.clientHeight;
    fitComposerHeight(
      ta,
      paneHeight ? Math.min(MAX_COMPOSER_HEIGHT, Math.floor(paneHeight * 0.4)) : MAX_COMPOSER_HEIGHT,
    );
  }, []);
  useLayoutEffect(autosizeComposer, [autosizeComposer, draft]);
  const composerBoxRef = useRef<HTMLDivElement>(null);
  const composerWidthRef = useRef(0);
  const paneHeightRef = useRef(0);
  useLayoutEffect(() => {
    const box = composerBoxRef.current;
    if (!box) return;
    const pane = box.closest<HTMLElement>("[data-pane-content]");
    const ro = new ResizeObserver((entries) => {
      let changed = false;
      for (const entry of entries) {
        if (entry.target === box) {
          const w = entry.contentRect.width;
          if (w === composerWidthRef.current) continue;
          composerWidthRef.current = w;
          changed = true;
        } else if (entry.target === pane) {
          const h = entry.contentRect.height;
          if (h === paneHeightRef.current) continue;
          paneHeightRef.current = h;
          changed = true;
        }
      }
      if (changed) autosizeComposer();
    });
    ro.observe(box);
    if (pane) ro.observe(pane);
    return () => ro.disconnect();
  }, [autosizeComposer]);

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
    projectPath: view.item.projectPath,
    value: draft,
    onChange: onDraftChange,
    textareaRef,
    enabled: !pendingQuestion,
    harness: harness.key,
    model: usingOpencode ? ocModel : null,
    onPickFile: (rel) => {
      const root = view.item.worktreePath.replace(/\/+$/, "");
      onAddAttachments([{ path: `${root}/${rel}`, name: basename(rel), kind: "file" }]);
    },
  });

  const { matchPrompt } = autocomplete;
  const pendingArg = useMemo(() => {
    const hit = matchPrompt(draft);
    return hit ? pendingComposerArg(hit, view.item.projectPath) : null;
  }, [matchPrompt, draft, view.item.projectPath]);

  type ExpandedDraft = { text: string; title: string | null };
  const expandDraft = async (text: string): Promise<ExpandedDraft | null> => {
    const hit = autocomplete.matchPrompt(text);
    if (!hit) return { text, title: null };
    const resolved = await resolveCustomPrompt(
      view.item.id,
      view.item.projectPath,
      hit.prompt,
      hit.typed,
    );
    if (!resolved) return null;
    if (!resolved.text.trim()) {
      setError(`${hit.prompt.name || "That prompt"} needs an argument, so nothing to send.`);
      return null;
    }
    return { text: resolved.text, title: resolved.title };
  };

  const pickPrompt = (prompt: CustomPrompt) => {
    const slug = promptSlug(prompt.name ?? "");
    if (!slug || autocomplete.offeredPrompts.get(slug) !== prompt) {
      void runCustomPrompt(view, prompt, shell.id);
      return;
    }
    onDraftChange(prefillSlashCommand(draft, slug));
    const caret = slug.length + 2;
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(caret, caret);
      autocomplete.syncCaret();
    });
  };

  const handleStart = async () => {
    const input = draft.trim();
    if (!canSubmit || pendingArg || starting) return;
    if (!usingOpencode && !usingCodex && model.disabled) {
      setError(`${model.label} is disabled by org policy. Pick another model.`);
      return;
    }
    setStarting(true);
    setError(null);
    try {
      const expanded = await expandDraft(input);
      if (!expanded) return;
      const atts: Attachment[] = [
        ...shell.docs.map((d) => ({ path: d.path, name: chipLabel(d.path), kind: "file" as const })),
        ...attachments,
      ];
      const created = await startSession({
        workstreamId: view.item.id,
        stage: shell.mode,
        input: expanded.text,
        attachments: atts,
        harness: harness.key,
        model: startModelFlag,
        effort: startEffort,
        permissionMode: permissionMode.flag,
        promptName: expanded.title,
      });
      onSessionStarted(created.id);
      if (expanded.text) {
        void generateTabTitle({
          workstreamId: view.item.id,
          shellId: shell.id,
          input: expanded.text,
        });
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
    if ((!text && attachments.length === 0) || pendingArg || !sid) return;
    setError(null);
    try {
      const expanded = await expandDraft(text);
      if (!expanded) return;
      await sendMessage(sid, expanded.text, attachments, expanded.title);
      transcriptRef.current?.scrollToBottom();
      onDraftChange("");
      setHistoryNav(IDLE_NAV);
      recalledValueRef.current = null;
      onClearAttachments();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

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

  const handleClearAndResume = async () => {
    await handleClear({ skipConfirm: true });
    if (draft.trim() === "") onDraftChange("/resume_handoff");
  };

  const handlePermissionModeChange = async (key: string) => {
    const opt = permissionModeOption(key);
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && session && opt.flag !== session.permissionMode && isBusy(session)) {
      const ok = await confirm(
        `Restart this session to apply “${opt.label}”? The active turn will be interrupted.`,
        { title: "Restart session?", kind: "warning" },
      );
      if (!ok) return;
    }
    onPermissionModeChange(key);
    if (live && sid) await applyPermissionMode(sid, opt.flag);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (autocomplete.handleKeyDown(e)) return;
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
        onDraftChange(text);
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
        <div className="min-h-0 flex-1" />
      )}

      <div className="px-4 pb-2 pt-2">
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
              {pinnedPrompts.length > 0 && (
                <PromptChips view={view} onPick={pickPrompt} className="mb-1.5 ml-auto max-w-[70%]" />
              )}
              <div className="relative rounded-lg border border-edge bg-panel-2 transition-colors focus-within:border-accent/50">
                <AutocompleteDropdown state={autocomplete} />
                {(attachments.length > 0 || (!started && shell.docs.length > 0)) && (
                  <div className="space-y-1.5 px-3 pt-2.5">
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
                <div className="flex items-end gap-1 pr-1">
                  <div ref={composerBoxRef} className="relative min-w-0 flex-1">
                    <HighlightBackdrop
                      value={draft}
                      commandNames={autocomplete.commandNames}
                      sigil={autocomplete.skillSigil}
                      scrollTop={composerScroll}
                      argHint={pendingArg && promptInputLabel(pendingArg)}
                    />
                    <textarea
                      ref={textareaRef}
                      value={draft}
                      onChange={(e) => {
                        const val = e.target.value;
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
                      className="selectable relative block w-full resize-none bg-transparent px-3 py-2.5 text-sm text-ink focus:outline-none disabled:cursor-not-allowed"
                    />
                  </div>
                  <div className="flex shrink-0 items-center gap-1 pb-1">
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
                      disabled={
                        !canSubmit || Boolean(pendingArg) || starting || Boolean(pendingQuestion)
                      }
                      onClick={() => (started ? void handleSend() : void handleStart())}
                      title={
                        pendingArg
                          ? `Type this prompt's argument after the command${pendingArg.label ? ` (${pendingArg.label})` : ""}`
                          : !started
                            ? "Start session"
                            : busy
                              ? "Queue message, sent when the turn finishes"
                              : "Send"
                      }
                      aria-label={!started ? "Start" : busy ? "Queue message" : "Send"}
                      className="flex h-8 w-8 items-center justify-center rounded-md bg-accent text-accent-ink hover:bg-accent/90 disabled:bg-edge/70 disabled:text-ink-dim disabled:hover:bg-edge/70"
                    >
                      {starting ? <Spinner size={14} /> : <SendIcon size={15} />}
                    </button>
                  </div>
                </div>
              </div>
              <div ref={toolbarRef} className="mt-1 flex items-center gap-0.5 overflow-x-clip">
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
                {!usingOpencode && !usingCodex && claudeEffortItems.length > 0 && (
                  <ComposerMenu
                    title="Reasoning effort"
                    leading={<EffortIcon size={14} />}
                    label={effort?.label ?? "Default"}
                    items={claudeEffortItems.map((e) => {
                      const blocked = e.key === "ultracode" && !modelSupportsUltracode(model);
                      return {
                        key: e.key,
                        label: e.label,
                        hint: blocked ? "needs an xhigh-capable model" : undefined,
                        disabled: blocked,
                        accent: e.key === "ultracode",
                      };
                    })}
                    activeKey={effort?.key ?? ""}
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

                {/* Right inset = the box's 1px border + its pr-1, so the w-8 column
                    below centres Attach under Send. */}
                <div className="ml-auto flex items-center gap-1 pr-[5px]">
                  {started && (
                    <button
                      type="button"
                      onClick={() => void handleClear()}
                      title={
                        showHandoffReady
                          ? "Clear context and resume from the handoff doc"
                          : "Clear context, ending this conversation to start fresh"
                      }
                      aria-label="Clear context"
                      className={
                        showHandoffReady
                          ? "flex h-6 w-6 animate-pulse items-center justify-center rounded text-warn ring-1 ring-warn/50 hover:bg-warn/10"
                          : "flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
                      }
                    >
                      <EraserIcon />
                    </button>
                  )}
                  <BackgroundTasksIndicator
                    tasks={backgroundTasks}
                    onJumpTo={(toolUseId) => transcriptRef.current?.scrollToTool(toolUseId)}
                  />
                  {session?.contextTokens != null && (
                    <ContextMeter tokens={session.contextTokens} windowSize={contextWindow} />
                  )}
                  <div className="flex w-8 justify-center">
                    <AttachMenu
                      onLibrary={onBrowseDocs}
                      onFiles={onAddFiles}
                      onScreenshots={onAddScreenshots}
                    />
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
