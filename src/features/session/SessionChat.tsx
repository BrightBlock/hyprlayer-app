import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
} from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import { Button, ErrorNote, MenuItem, Popover, Spinner, useConfirm } from "@/components/ui";
import { type OptionMenuGroup, type OptionMenuItem } from "@/components/OptionMenuList";
import { BrandMark } from "@/components/BrandMark";
import { Icon, stageIconName } from "@/components/Icon";
import { goalDraft, goalSpent } from "@/lib/domain/goals";
import { openGoalSheet } from "@/lib/goalSheet";
import { errorMessage, isStaleTranscript } from "@/lib/ipc";
import { eventToChord, formatChord, hasPrimaryModifier } from "@/lib/keybindings/chord";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { basename } from "@/lib/path";
import { IS_MACOS } from "@/lib/platform";
import {
  useBootstrap,
  useClaudeProbe,
  useCodexProbe,
  useOpencodeCatalog,
  useResumeGoal,
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
import type { ShellTab } from "./workstreamTabs";
import { MODE_STYLES, asStageKey, nextModeStage, suggestedDocKind, type StageKey } from "@/lib/domain/stages";
import {
  claudeModels,
  codexEfforts,
  effectiveEffort,
  effortsForModel,
  modelDisplayName,
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
import { permissionModeOption } from "@/lib/domain/permissions";
import { AgentModePicker } from "./AgentModePicker";
import { controlsKey, usePublishSessionControls } from "./sessionControls";
import { formatTokens } from "@/lib/format";
import { detectHandoffDoc } from "./toolDisplay";
import { useWrittenDocs } from "./writtenDocs";
import { planRevisions } from "./planRevisions";
import { PlanCardsContext } from "./planCardSlot";
import { interceptSend } from "./sendInterceptor";
import { goalActiveSince, useSessionGoal } from "./sessionGoal";
import {
  TranscriptView,
  type PermissionHandlers,
  type PromptHandlers,
  type SkeletonTail,
  type TranscriptGoal,
  type TranscriptHandle,
  type TranscriptSync,
} from "./TranscriptView";
import { clearTranscriptJump, useTranscriptJumpStore } from "./transcriptJump";
import {
  allowForSession,
  answerQuestion,
  applyModel,
  applyPermissionMode,
  cancelQueuedMessage,
  dismissDrift,
  isFirstUserTurn,
  killSession,
  pullNow,
  refreshSession,
  respondPermission,
  restoreSessionIfNeeded,
  resumeSession,
  rewindToMessage,
  sendMessage,
  startSession,
  steerQueued,
  stopSession,
  takeArmedResume,
  useSession,
  useMostRecentDoc,
  useSessionBackgroundTasks,
  type QueuedMessage,
} from "./sessionStore";
import { dismissHeld, noteComposerInput, setDivider, syncOf, useTranscriptSync } from "./syncStore";
import { canSend } from "./transcriptSync";
import { pendingCount, sentenceDevice, SyncComposerNotice, syncNotice } from "./SyncComposerNotice";
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

type Divider = { seq: number; device: string };

// Dividers whose fade has played, kept outside the component so a remount doesn't replay it.
const fadedDividers = new WeakSet<Divider>();
const STEER_CHORD = formatChord("mod+enter", IS_MACOS);
const FRESH_MS = 400;

function useFreshFromSeq(divider: Divider | null): number | null {
  const [, expire] = useState(0);
  useEffect(() => {
    if (!divider || fadedDividers.has(divider)) return;
    let mounted = true;
    // Not cleared on unmount: the fade is spent whether or not this view saw it end.
    setTimeout(() => {
      fadedDividers.add(divider);
      if (mounted) expire((n) => n + 1);
    }, FRESH_MS);
    return () => {
      mounted = false;
    };
  }, [divider]);
  return divider && !fadedDividers.has(divider) ? divider.seq : null;
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

export function SessionTabIcon({ mode }: { mode: StageKey }) {
  return (
    <span className={`rounded p-0.5 ${MODE_STYLES[mode].badge}`}>
      <Icon name={stageIconName(mode)} />
    </span>
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
      ? [{ key: "library", label: "Attach from library", icon: <Icon name="library" size="sm" />, run: onLibrary }]
      : []),
    { key: "files", label: "Add files…", icon: <Icon name="doc" size="sm" />, run: onFiles },
    { key: "shots", label: "Add screenshots…", icon: <Icon name="image" size="sm" />, run: onScreenshots },
  ];
  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        title="Attach context"
        aria-label="Attach context"
        className="flex h-8 w-8 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:bg-edge/60 focus-visible:text-ink"
      >
        <Icon name="add" />
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
      <span className="shrink-0 text-warn-ink">
        <Icon name="handoff" size="sm" />
      </span>
      <span className="min-w-0 flex-1 text-ui text-ink-dim">
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
        className="shrink-0 rounded-md border border-warn/40 bg-warn/10 px-2 py-1 text-ui font-medium text-warn-ink hover:bg-warn/20 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-warn/10"
      >
        {actionLabel}
      </button>
      <button
        type="button"
        onClick={onDismiss}
        title={isReady ? "Dismiss, keep working in this session" : "Dismiss"}
        aria-label="Dismiss"
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
      >
        <Icon name="close" size="xs" />
      </button>
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
      <div className="mb-2 flex items-center justify-between gap-2 rounded-card border border-q-card-border bg-q-card-bg p-3.5 shadow-card">
        <span className="text-body text-ink-dim">
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
      } ${otherOpen ? "border-accent bg-accent" : "border-radio-border"}`}
      aria-hidden="true"
    >
      {otherOpen && (
        <Icon name="check" size="xs" className="text-accent-ink" />
      )}
    </span>
  );

  return (
    <div className="mb-2 rounded-card border border-q-card-border bg-q-card-bg p-3.5 shadow-card">
      <div className="mb-2.5 flex items-center gap-2 text-q-label">
        <Icon name="question" className="text-warn" />
        <span className="text-body font-medium">The agent is asking</span>
        {total > 1 && (
          <span className="ml-auto text-ui font-normal tabular-nums text-ink-dim">
            {step + 1} of {total}
          </span>
        )}
      </div>

      {question.header && (
        <span className="mb-1 block text-label text-ink-dim">
          {question.header}
        </span>
      )}
      <p className="text-body font-medium text-ink">{question.question}</p>
      {question.multiSelect && (
        <p className="mt-0.5 text-ui text-ink-dim">Select all that apply.</p>
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
              className={`flex w-full items-start gap-2.5 rounded-control border px-3 py-2 text-left transition-colors disabled:opacity-50 ${
                selected
                  ? "border-accent bg-accent/15"
                  : "border-edge bg-panel-2 hover:border-edge hover:bg-edge/50"
              }`}
            >
              <span
                className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center border ${
                  question.multiSelect ? "rounded" : "rounded-full"
                } ${selected ? "border-accent bg-accent" : "border-radio-border"}`}
                aria-hidden="true"
              >
                {selected && (
                  <Icon name="check" size="xs" className="text-accent-ink" />
                )}
              </span>
              <span className="min-w-0">
                <span className="block text-body text-ink">{opt.label}</span>
                {opt.description && (
                  <span className="mt-0.5 block text-ui text-ink-dim">
                    {opt.description}
                  </span>
                )}
              </span>
            </button>
          );
        })}
        <div
          className={`rounded-control border transition-colors ${
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
                className="min-w-0 flex-1 bg-transparent text-body text-ink placeholder:text-ink-dim focus:outline-none disabled:opacity-50"
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
              <span className="block text-body text-ink">Other…</span>
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
  focused: boolean;
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
  onReturnQueued: (messages: QueuedMessage[]) => void;
};

export function SessionChat({
  view,
  shell,
  focused,
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
  onReturnQueued,
}: Props) {
  const session = useSession(shell.sessionId);
  const [starting, setStarting] = useState(false);
  const [resuming, setResuming] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [confirmDialog, confirm] = useConfirm();
  const dropRef = useRef<HTMLDivElement>(null);
  const addAttachmentsRef = useRef(onAddAttachments);
  addAttachmentsRef.current = onAddAttachments;
  const workstreamId = view.item.id;

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
  const lockOther = (itemProvider: string): Pick<OptionMenuItem, "disabled"> =>
    lockedProvider !== null && itemProvider !== lockedProvider ? { disabled: true } : {};

  const claudeRow = (m: ModelOption): OptionMenuItem => ({
    key: `claude:${m.key}`,
    label: m.label,
    hint: m.disabled ? (m.description ?? "unavailable") : m.hint,
    icon: <BrandMark brand="claude" size="sm" />,
    ...(m.disabled
      ? { disabled: true, disabledReason: m.description ?? "Unavailable in this CLI version" }
      : lockOther("claude")),
  });

  const modelGroups: OptionMenuGroup[] = [
    {
      key: "claude",
      label: "Claude Code",
      icon: <BrandMark brand="claude" size="sm" />,
      items: modelItems.filter((m) => !m.unlisted).map(claudeRow),
      more: {
        label: "More models",
        items: modelItems.filter((m) => m.unlisted).map(claudeRow),
      },
    },
    {
      key: "codex",
      label: "Codex",
      icon: <BrandMark brand="openai" size="sm" />,
      items: codexModels.map((id) => ({
        key: `codex:${id}`,
        label: id,
        icon: <BrandMark brand="openai" size="sm" />,
        ...lockOther("codex"),
      })),
    },
    {
      key: "opencode",
      label: "OpenCode",
      icon: <BrandMark brand="opencode" size="sm" />,
      items: ocModels.map((id) => ({
        key: `opencode:${id}`,
        label: opencodeModelLabel(id, ocNames),
        hint: ocNames.get(id)?.provider,
        icon: <BrandMark brand="opencode" size="sm" />,
        ...lockOther(modelProvider("opencode", id) ?? ""),
      })),
    },
    ...(ollamaModels.length > 0
      ? [
          {
            key: "ollama",
            label: "Ollama (local)",
            icon: <BrandMark brand="ollama" size="sm" />,
            items: ollamaModels.map((id) => ({
              key: `opencode:${id}`,
              label: id.replace(/^ollama\//, ""),
              hint: "Ollama",
              icon: <BrandMark brand="ollama" size="sm" />,
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
  const codexEffortOptions: OptionMenuItem[] = codexEffortItems.map((e) => ({
    key: e.key,
    label: e.label,
  }));
  const claudeEffortOptions: OptionMenuItem[] = claudeEffortItems.map((e) => {
    const blocked = e.key === "ultracode" && !modelSupportsUltracode(model);
    return {
      key: e.key,
      label: e.label,
      hint: blocked ? "needs an xhigh-capable model" : undefined,
      disabled: blocked,
      accent: e.key === "ultracode",
    };
  });

  const applyClaudeModel = async (key: string) => {
    if (key === model.key) return;
    const next = modelOption(key, claudeCatalog);
    if (next.disabled) return;
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && session && isBusy(session)) {
      const ok = await confirm({
        title: "Switch model?",
        message: `Restart this session to switch to ${next.label}? The active turn will be interrupted.`,
        confirmLabel: "Switch model",
      });
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
      void restoreSessionIfNeeded(
        shell.sessionId,
        view.item.id,
        shell.sessionStage ?? shell.mode,
        restorePermissionFlag,
      );
    }
  }, [shell.sessionId, hasEntry, view.item.id, shell.sessionStage, shell.mode, restorePermissionFlag]);

  const autoResumeDecidedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!shell.sessionId) {
      autoResumeDecidedRef.current = null;
      return;
    }
    if (autoResumeDecidedRef.current === shell.sessionId) return;
    if (!session || session.items.length === 0) return;
    // Decided once, when the tab first sees the session: one that ends while
    // open was stopped on purpose (here or on another device), and every open
    // window relaunching it would race to spawn duplicate harnesses.
    autoResumeDecidedRef.current = shell.sessionId;
    if (!isTerminal(session.state) || shell.frozen) return;
    setError(null);
    setResuming(true);
    void resumeSession(shell.sessionId)
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setResuming(false));
  }, [shell.sessionId, shell.frozen, session]);

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

  const sid = shell.sessionId;
  const sync = useTranscriptSync((s) => (sid ? s.bySession[sid] : undefined));
  const divider = useTranscriptSync((s) => (sid ? (s.divider[sid] ?? null) : null));
  const held = useTranscriptSync((s) => (sid ? s.held[sid] : undefined));
  const selfClientId = useTranscriptSync((s) => s.self?.clientId ?? null);
  const freshFromSeq = useFreshFromSeq(divider);
  // Deleted on the server: the ended-session composer, with nothing to resume.
  const gone = Boolean(sync?.gone);
  // A server without sync keeps no sync state, and nothing holds its sends back.
  const sendable = !sync || canSend(sync);
  const busy = session ? isBusy(session) : started;
  const working = session ? isWorking(session) : false;
  const terminal = (session ? isTerminal(session.state) : false) || gone;
  const pendingQuestion = session?.pendingQuestion ?? null;
  const draft = shell.draft;
  const attachments = shell.attachments ?? [];
  const canSubmit = draft.trim().length > 0 || attachments.length > 0;
  const otherTurnRunning = Boolean(sync?.workingDevice) || (session?.turnAuthor === "other" && working);
  const tail = useMemo<SkeletonTail | null>(() => {
    if (!sync) return null;
    const count = pendingCount(sync);
    const workingLabel = otherTurnRunning ? sentenceDevice(sync) : null;
    if (count === 0 && sync.pendingTurns.length === 0 && !workingLabel) return null;
    return { phase: sync.phase, turns: sync.pendingTurns, count, working: workingLabel };
  }, [sync, otherTurnRunning]);
  const transcriptSync = useMemo<TranscriptSync>(
    () => ({ tail, divider, freshFromSeq, selfClientId }),
    [tail, divider, freshFromSeq, selfClientId],
  );

  // Every draft change, typed or not: a send or a recall clears or fills it without a keystroke.
  useEffect(() => {
    if (sid) noteComposerInput(sid, draft);
  }, [sid, draft]);

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
  // Scrolled up: the rev and divider then, so a later pull's turns can be counted on the pill.
  const [away, setAway] = useState<{ rev: number; divider: Divider | null } | null>(null);
  const dividerRef = useRef(divider);
  dividerRef.current = divider;
  const onBottomChange = useCallback(
    (atBottom: boolean) =>
      setAway(atBottom || !sid ? null : { rev: syncOf(sid)?.localRev ?? 0, divider: dividerRef.current }),
    [sid],
  );
  useEffect(() => setAway(null), [sid]);
  const newTurns =
    away && divider && divider !== away.divider ? Math.max(0, (sync?.localRev ?? 0) - away.rev) : 0;
  const onDividerPassed = useCallback(() => {
    if (sid) setDivider(sid, null);
  }, [sid]);
  const onCancelQueued = useCallback(
    (index: number) => {
      if (sid) void cancelQueuedMessage(sid, index);
    },
    [sid],
  );
  const jumpRequest = useTranscriptJumpStore((s) =>
    s.pending && s.pending.sessionId === sid ? s.pending : null,
  );
  useEffect(() => {
    if (!jumpRequest) return;
    clearTranscriptJump();
    const { target } = jumpRequest;
    if ("toolUseId" in target) transcriptRef.current?.scrollToTool(target.toolUseId);
    else transcriptRef.current?.scrollToItem(target.itemId);
  }, [jumpRequest]);

  const HANDOFF_NUDGE_THRESHOLD = 0.6;
  // Only the harness's own report; it arrives with the first completed turn.
  const contextWindow = session?.contextWindow || null;
  const contextPct =
    session?.contextTokens != null && contextWindow != null
      ? session.contextTokens / contextWindow
      : 0;
  const handoffDoc = shell.docs.find((d) => d.role === "handoff") ?? null;
  const sessionItems = session?.items;
  const dismissedSet = useMemo(
    () => new Set(shell.dismissedSuggestions ?? []),
    [shell.dismissedSuggestions],
  );
  const writtenDocs = useWrittenDocs(workstreamId);
  const sessionStage = session?.stage;
  // An implement session only ticks the plan's checkboxes; those edits are not revisions.
  const planCards = useMemo(
    () =>
      sid && sessionItems && sessionStage !== "implement"
        ? { sessionId: sid, workstreamId, revisions: planRevisions(sessionItems, writtenDocs) }
        : null,
    [sid, workstreamId, sessionItems, sessionStage, writtenDocs],
  );
  const { goal: sessionGoal, detail: goalDetail } = useSessionGoal(sid, workstreamId);
  const goalIterations = goalDetail?.iterations ?? null;
  const transcriptGoal = useMemo<TranscriptGoal | null>(
    () => (sid && sessionGoal ? { sessionId: sid, workstreamId, goal: sessionGoal, iterations: goalIterations } : null),
    [sid, workstreamId, sessionGoal, goalIterations],
  );
  const resumeGoal = useResumeGoal();
  const readyCard = useMemo(
    () =>
      sid && (sessionStage === "implement" || sessionStage === "validate") ? { sessionId: sid, workstreamId } : null,
    [sid, sessionStage, workstreamId],
  );
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

  const openGoalFromDraft = (text: string) => {
    openGoalSheet({
      mode: "start",
      workstreamId,
      source: "composer",
      text,
      target: sid ? { sessionId: sid } : { newSession: { stage: mode, shellId: shell.id } },
    });
    onDraftChange("");
    setHistoryNav(IDLE_NAV);
    recalledValueRef.current = null;
  };

  // A `/goal` draft reaching the harness would start its own native goal.
  const interceptGoalDraft = (): boolean => {
    const text = goalDraft(draft);
    if (text === null) return false;
    openGoalFromDraft(text);
    return true;
  };

  const handleStart = async () => {
    if (interceptGoalDraft()) return;
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

  const handleSend = async (steer = false) => {
    if (interceptGoalDraft()) return;
    const text = draft.trim();
    if (steer && !text && attachments.length === 0 && sid) {
      void steerQueued(sid);
      return;
    }
    if ((!text && attachments.length === 0) || pendingArg || !sid) return;
    // The store, not the render: a send right after a pull must see what the pull left.
    const current = syncOf(sid);
    if (current && !canSend(current)) return;
    setError(null);
    try {
      const intercepted =
        attachments.length === 0 &&
        (await interceptSend({ sessionId: sid, workstreamId: view.item.id, shell, text }));
      if (intercepted) {
        onDraftChange("");
        setHistoryNav(IDLE_NAV);
        recalledValueRef.current = null;
        return;
      }
      const expanded = await expandDraft(text);
      if (!expanded) return;
      const outcome = await sendMessage(sid, expanded.text, attachments, expanded.title, undefined, steer);
      // Reply and resume: the server's boundary waits for this reply's turn before it re-checks.
      const armed = outcome === "dropped" ? null : takeArmedResume(sid);
      if (armed) resumeGoal.mutate(armed, { onError: (e) => setError(errorMessage(e)) });
      setDivider(sid, null);
      transcriptRef.current?.scrollToBottom();
      onDraftChange("");
      setHistoryNav(IDLE_NAV);
      recalledValueRef.current = null;
      onClearAttachments();
    } catch (e) {
      // A refusal for staleness shows as the guard.
      if (!isStaleTranscript(e)) setError(errorMessage(e));
    }
  };

  // Never a bypass: the pull runs first, and turns that land meanwhile keep the guard up.
  const handleSendAnyway = async () => {
    if (!sid) return;
    await pullNow(sid);
    await handleSend();
  };

  const handleClear = async (opts: { skipConfirm?: boolean } = {}) => {
    if (!started) return;
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    if (live && !opts.skipConfirm) {
      const message = handoffDoc
        ? "Clear this conversation and start fresh? The captured handoff doc will attach to the next session in this tab."
        : "Clear this conversation and start fresh? The current session ends and its context is discarded.";
      const ok = await confirm({
        title: "Clear context?",
        message,
        confirmLabel: "Clear",
        confirmVariant: "danger",
      });
      if (!ok) return;
    }
    if (live && sid) await killSession(sid).catch(() => {});
    onResetSession();
  };
  useCommand("session.clear", focused && started ? () => void handleClear() : null);

  const handleRefresh = async () => {
    if (!sid) return;
    setError(null);
    setRefreshing(true);
    try {
      await refreshSession(sid);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setRefreshing(false);
    }
  };

  const handleClearAndResume = async () => {
    await handleClear({ skipConfirm: true });
    if (draft.trim() === "") onDraftChange("/resume_handoff");
  };

  const handlePermissionModeChange = async (key: string) => {
    const opt = permissionModeOption(key);
    const live = Boolean(sid && session && session.live && !isTerminal(session.state));
    const inPlace = harness.key === "claude";
    if (live && session && !inPlace && opt.flag !== session.permissionMode && isBusy(session)) {
      const ok = await confirm({
        title: "Restart session?",
        message: `Restart this session to apply “${opt.label}”? The active turn will be interrupted.`,
        confirmLabel: "Restart",
      });
      if (!ok) return;
    }
    onPermissionModeChange(key);
    if (live && sid) await applyPermissionMode(sid, opt.flag, inPlace);
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
    // Mid-turn, Enter waits for the turn's end; the primary modifier steers into its next step.
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!started) void handleStart();
      else void handleSend(IS_MACOS ? e.metaKey : e.ctrlKey);
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

  usePublishSessionControls(
    controlsKey(workstreamId, shell.id),
    {
      harness: harness.key,
      interactive: !terminal,
      model: {
        // The live catalog labels its default model "Default"; name the model it resolves to.
        label:
          usingOpencode || usingCodex
            ? composerModelLabel
            : (modelDisplayName(model.resolvedModel) ?? model.label),
        hint: composerModelHint ?? null,
        activeKey: composerModelActiveKey,
        groups: modelGroups,
      },
      effort: usingCodex
        ? { label: codexEffort.label, activeKey: codexEffort.key, items: codexEffortOptions }
        : !usingOpencode && claudeEffortItems.length > 0
          ? { label: effort?.label ?? "Default", activeKey: effort?.key ?? "", items: claudeEffortOptions }
          : null,
      permissionKey: usingOpencode ? null : permissionMode.key,
      planning: Boolean(session?.live && session.planning),
      context:
        session?.contextTokens != null && contextWindow != null
          ? {
              tokens: session.contextTokens,
              window: contextWindow,
              title: `Context ${formatTokens(session.contextTokens)} / ${formatTokens(contextWindow)}`,
            }
          : null,
      goal: sessionGoal && {
        id: sessionGoal.id,
        status: sessionGoal.status,
        step: sessionGoal.step,
        iteration: sessionGoal.iteration,
        maxIterations: sessionGoal.limits.maxIterations,
        spent: goalSpent(sessionGoal),
        activeMs: sessionGoal.activeMs,
        activeSince: goalActiveSince(sessionGoal),
      },
    },
    {
      pickModel: onPickModel,
      pickEffort: usingCodex ? onCodexEffortChange : onEffortChange,
      setPermissionMode: (key) => void handlePermissionModeChange(key),
      openHarnessSettings: onOpenHarnessSettings,
    },
  );

  // On an ended view only fallback A shows: another device may have resumed the session.
  const notice = syncNotice(sync, Boolean(session?.drifted), refreshing, terminal, held);
  // The new-turns strip sits inside the composer box; every other notice sits above it.
  const behind = notice?.kind === "behind";
  const noticeProps = {
    onPull: () => {
      if (sid) void pullNow(sid);
    },
    onSendAnyway: () => void handleSendAnyway(),
    onLoad: () => void handleRefresh(),
    onDismiss: () => {
      if (!sid) return;
      if (behind) dismissHeld(sid);
      else dismissDrift(sid);
    },
  };
  const composerNotice = <SyncComposerNotice notice={behind ? null : notice} {...noticeProps} />;

  return (
    <div ref={dropRef} className="relative flex min-h-0 flex-1 flex-col">
      {dragOver && (
        <div className="pointer-events-none absolute inset-0 z-40 m-2 flex items-center justify-center rounded-xl border-2 border-dashed border-accent bg-accent/10">
          <div className="flex items-center gap-2 rounded-lg bg-panel px-3.5 py-2 text-body font-medium text-ink shadow-lg">
            <Icon name="image" />
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
        <PlanCardsContext.Provider value={planCards}>
          <div className="relative flex min-h-0 flex-1 flex-col">
            <TranscriptView
              ref={transcriptRef}
              items={session?.items ?? []}
              busy={working}
              connecting={!session || session.state === "starting"}
              harnessLabel={harness.label}
              workingSince={session?.workingSince ?? null}
              permission={permission}
              prompt={prompt}
              sync={transcriptSync}
              ready={readyCard}
              goal={transcriptGoal}
              queued={session?.queued}
              canWithdrawQueued={!usingCodex}
              onCancelQueued={onCancelQueued}
              onBottomChange={onBottomChange}
              onDividerPassed={onDividerPassed}
            />
            {newTurns > 0 && (
              <button
                type="button"
                onClick={() => {
                  transcriptRef.current?.scrollToBottom();
                  setAway(null);
                }}
                className="absolute bottom-3 left-1/2 z-20 flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-info/40 bg-panel px-3 py-1 text-meta text-info shadow-pop hover:bg-panel-2 focus:outline-none focus-visible:bg-panel-2"
              >
                <Icon name="arrowDown" size="xs" />
                {newTurns} new {newTurns === 1 ? "turn" : "turns"}
              </button>
            )}
          </div>
        </PlanCardsContext.Provider>
      ) : (
        <div className="min-h-0 flex-1" />
      )}

      <div className="px-4 pb-4 pt-2">
        <div className="mx-auto w-[90%]">
          {error && (
            <div className="mb-2">
              <ErrorNote error={{ code: "session", message: error, remedy: null }} />
            </div>
          )}

          {terminal ? (
            <>
              {composerNotice}
              <div className="flex items-center justify-between gap-3 rounded-md border border-edge bg-panel-2 px-3 py-2.5">
                <span className="text-body text-ink-dim">
                  {resuming
                    ? "Resuming the previous session…"
                    : "Session ended."}
                </span>
                <div className="flex items-center gap-2">
                  <Button onClick={onResetSession} disabled={resuming}>
                    New session
                  </Button>
                  <Button
                    variant="primary"
                    busy={resuming}
                    disabled={!sid || resuming || gone}
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
            </>
          ) : (
            <>
              <div className="relative z-30">
                {composerNotice}
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
              </div>
              {pinnedPrompts.length > 0 && (
                <PromptChips view={view} onPick={pickPrompt} className="mb-1.5 ml-auto max-w-[70%]" />
              )}
              <div
                className={`relative rounded-lg border bg-panel-2 transition-colors ${
                  sync?.phase === "guard"
                    ? "border-warn/50 focus-within:border-warn/50"
                    : behind
                      ? "border-info/40 focus-within:border-info/60"
                      : "border-edge focus-within:border-accent/50"
                }`}
              >
                <SyncComposerNotice notice={behind ? notice : null} {...noticeProps} />
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
                  <div className="flex shrink-0 items-center pb-1 pl-1">
                    <AgentModePicker mode={mode} started={started} onChange={onModeChange} />
                  </div>
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
                      className="selectable relative block w-full resize-none bg-transparent px-3 py-2.5 text-body leading-5 text-ink focus:outline-none disabled:cursor-not-allowed"
                    />
                  </div>
                  <div className="flex shrink-0 items-center gap-1 pb-1">
                    <BackgroundTasksIndicator
                      tasks={backgroundTasks}
                      onJumpTo={(toolUseId) => transcriptRef.current?.scrollToTool(toolUseId)}
                    />
                    <button
                      type="button"
                      onClick={() => openGoalFromDraft(goalDraft(draft) ?? draft.trim())}
                      title="Run until a goal is met"
                      aria-label="Start a goal"
                      className="flex h-8 w-8 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:bg-edge/60 focus-visible:text-ink"
                    >
                      <Icon name="goal" />
                    </button>
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
                            ? "flex h-8 w-8 items-center justify-center rounded-md text-warn-ink ring-1 ring-warn/50 hover:bg-warn/10"
                            : "flex h-8 w-8 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink"
                        }
                      >
                        <Icon name="clear" />
                      </button>
                    )}
                    <AttachMenu
                      onLibrary={onBrowseDocs}
                      onFiles={onAddFiles}
                      onScreenshots={onAddScreenshots}
                    />
                    {busy && started && (
                      <button
                        type="button"
                        onClick={() => sid && void stopSession(sid, onReturnQueued)}
                        title="Stop the current turn (interrupt, then force-stop if it doesn't respond)"
                        aria-label="Stop"
                        className="flex h-8 w-8 items-center justify-center rounded-md border border-bad/40 bg-bad/10 text-bad hover:bg-bad/20"
                      >
                        <Icon name="stop" />
                      </button>
                    )}
                    <button
                      type="button"
                      disabled={
                        !canSubmit ||
                        Boolean(pendingArg) ||
                        starting ||
                        Boolean(pendingQuestion) ||
                        (started && !sendable)
                      }
                      onClick={() => (started ? void handleSend() : void handleStart())}
                      title={
                        pendingArg
                          ? `Type this prompt's argument after the command${pendingArg.label ? ` (${pendingArg.label})` : ""}`
                          : !started
                            ? "Start session"
                            : busy
                              ? usingOpencode
                                ? "Queue message, sent when the turn finishes"
                                : `Queue message, sent when the turn finishes (${STEER_CHORD} sends it at the agent's next step)`
                              : "Send"
                      }
                      aria-label={!started ? "Start" : busy ? "Queue message" : "Send"}
                      className="flex h-8 w-8 items-center justify-center rounded-md bg-accent text-accent-ink hover:bg-accent/90 disabled:bg-edge/70 disabled:text-ink-dim disabled:hover:bg-edge/70"
                    >
                      {starting ? <Spinner /> : <Icon name="send" />}
                    </button>
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
      {confirmDialog}
    </div>
  );
}
