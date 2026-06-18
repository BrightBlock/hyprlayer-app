import {
  useEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { confirm } from "@tauri-apps/plugin-dialog";

import { Badge, Button, ErrorNote, Spinner } from "../../components/ui";
import {
  BookIcon,
  ChevronDownIcon,
  DocIcon,
  EffortIcon,
  ImageIcon,
  ModeIcon,
  PlusIcon,
  QuestionIcon,
  SendIcon,
  ShieldIcon,
  SparkleIcon,
  StopIcon,
  XIcon,
} from "../../components/icons";
import { errorMessage } from "../../lib/ipc";
import type { WorkstreamView } from "../../lib/bindings";
import { chipLabel, WorkstreamDocChips } from "../workstreams/WorkstreamDocChips";
import { AttachmentCards } from "./AttachmentCard";
import { importDroppedPaths, saveClipboardImage, type Attachment } from "./attachments";
import type { ShellTab } from "../../routes/ProjectPage";
import { MODE_STAGES, MODE_STYLES, asStageKey, stageLabel, type StageKey } from "./stages";
import { EFFORTS, MODELS, effortOption, modelOption } from "./models";
import { PERMISSION_MODES, permissionModeOption } from "./permissions";
import { formatTokens } from "./toolDisplay";
import { TranscriptView, type PermissionHandlers } from "./TranscriptView";
import {
  allowForSession,
  answerQuestion,
  applyPermissionMode,
  cancelQueuedMessage,
  respondPermission,
  restoreSessionIfNeeded,
  sendMessage,
  startSession,
  stopSession,
  useSession,
  type QueuedMessage,
} from "./sessionStore";
import { describeState, isBusy, isTerminal, isWorking, type PendingQuestion } from "./transcript";

const STAGE_INTRO: Record<StageKey, string> = {
  general: "Chat with the project agent in this worktree. Pick a stage below to run a spec-driven step.",
  research: "Runs /research_codebase to investigate the code and write a research doc into the library.",
  plan: "Runs /create_plan to turn research into a concrete, reviewable implementation plan.",
  implement: "Runs /implement_plan to make the changes in this worktree, committing as it goes.",
  validate: "Runs /validate_plan to check the work against the plan and the project's tests.",
  ship: "Runs /commit, then helps push the branch and open a pull request.",
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
  accent: "bg-sky-400",
};

/** A tiny status dot for an agent tab encoding the session's tone (none until a
 *  session is started). Motion lives on the tab icon's glow, so this stays
 *  steady — it carries the color, not the "live" signal. */
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

/** The mode badge that leads an agent tab. Glows while the session is actively
 *  working (same `isWorking` signal as the elapsed counter) and goes quiet when
 *  the turn completes OR pauses for the user — a session awaiting a permission
 *  shows the warn-toned dot, not a "working" pulse. Subscribes per-session so
 *  only this tab re-renders on deltas. */
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
};

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
  custom,
}: {
  leading: ReactNode;
  label: string;
  hint?: string;
  items: MenuItem[];
  activeKey: string;
  onSelect: (key: string) => void;
  title: string;
  /** When set, appends a free-text row so the user can enter a value that isn't
   *  one of `items` — used by the model picker to accept ANY `--model` the CLI
   *  takes (an alias or a full id). Submitting calls `onSelect` with the text. */
  custom?: { label: string; placeholder?: string };
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  // The active selection is "custom" when it matches none of the preset items.
  const customActive = custom != null && !items.some((it) => it.key === activeKey);
  // A risky active choice (e.g. "Bypass permissions") tints the whole trigger red.
  const activeDanger = items.some((it) => it.key === activeKey && it.danger);

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
        title={title}
        className={`flex items-center gap-1.5 rounded-md px-2 py-1 text-xs hover:bg-edge/50 ${
          activeDanger ? "text-bad hover:text-bad" : "text-ink-dim hover:text-ink"
        }`}
      >
        <span className="shrink-0">{leading}</span>
        <span className={`font-medium ${activeDanger ? "text-bad" : "text-ink"}`}>{label}</span>
        {hint && <span className={activeDanger ? "text-bad/80" : "text-ink-dim"}>{hint}</span>}
        <ChevronDownIcon size={12} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={close} aria-hidden="true" />
          <div className="absolute bottom-full left-0 z-30 mb-1.5 min-w-[190px] overflow-hidden rounded-lg border border-edge bg-panel py-1 shadow-xl">
            {items.map((it) => (
              <button
                key={it.key}
                type="button"
                onClick={() => {
                  onSelect(it.key);
                  close();
                }}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/50 ${
                  it.danger ? "text-bad" : it.key === activeKey ? "text-ink" : "text-ink-dim"
                }`}
              >
                {it.icon && <span className="shrink-0">{it.icon}</span>}
                <span className="font-medium">{it.label}</span>
                {it.hint && <span className="text-ink-dim">{it.hint}</span>}
                {it.key === activeKey && <span className="ml-auto text-[10px] text-ink-dim">●</span>}
              </button>
            ))}
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
          </div>
        </>
      )}
    </div>
  );
}

/** The composer "+" — opens upward into a small action menu for attaching
 *  context: library docs (centered modal) or files / screenshots (native OS
 *  file dialog). Unlike ComposerMenu these items are one-shot actions. */
function AttachMenu({
  onLibrary,
  onFiles,
  onScreenshots,
  showLibrary = true,
}: {
  onLibrary: () => void;
  onFiles: () => void;
  onScreenshots: () => void;
  /** Library docs are first-turn context, so the option only shows before a
   *  session starts; files/screenshots can be attached to any turn. */
  showLibrary?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const items = [
    ...(showLibrary
      ? [{ key: "library", label: "Attach from library", icon: <BookIcon size={14} />, run: onLibrary }]
      : []),
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
 * The interactive answer panel for an `AskUserQuestion`, pinned just above the
 * composer. Questions are asked one at a time (a stepper): each question shows
 * its options (single- or multi-select), "Next" advances, and the last question's
 * "Submit" sends every collected answer back to the agent in one go. Selections
 * accumulate across steps (keyed by question text) and "Back" revisits them.
 * "Skip" sends no answer, which the agent handles by asking again in prose.
 * Remounted per prompt (keyed on requestId) so state never leaks between asks.
 */
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

function QuestionPrompt({
  pending,
  onSubmit,
}: {
  pending: PendingQuestion;
  onSubmit: (answers: Record<string, string>) => void;
}) {
  const [selections, setSelections] = useState<Record<string, string[]>>({});
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
      <div className="mb-2 flex items-center justify-between gap-2 rounded-xl border border-sky-400/30 bg-sky-400/[0.06] p-3.5">
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
  const currentAnswered = chosen.length > 0;
  const isLast = step === total - 1;

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
  };

  const submitAll = () => {
    const answers: Record<string, string> = {};
    for (const q of pending.questions) {
      const picks = selections[q.question] ?? [];
      if (picks.length > 0) answers[q.question] = picks.join(", ");
    }
    send(answers);
  };

  // "Next" until the last question, which becomes "Submit"; both need an answer.
  const advance = () => {
    if (!currentAnswered) return;
    if (isLast) submitAll();
    else setStep((s) => s + 1);
  };

  return (
    <div className="mb-2 rounded-xl border border-sky-400/30 bg-sky-400/[0.06] p-3.5">
      <div className="mb-2.5 flex items-center gap-2 text-sky-200">
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
  onSessionStarted: (sessionId: string) => void;
  onResetSession: () => void;
  onOpenDoc: (id: string) => void;
  onDetachDoc: (path: string) => void;
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

export function SessionChat({
  view,
  shell,
  onDraftChange,
  onModeChange,
  onModelChange,
  onEffortChange,
  onPermissionModeChange,
  onSessionStarted,
  onResetSession,
  onOpenDoc,
  onDetachDoc,
  onBrowseDocs,
  onAddFiles,
  onAddScreenshots,
  onAddAttachments,
  onRemoveAttachment,
  onClearAttachments,
}: Props) {
  const session = useSession(shell.sessionId);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);
  // The add-attachments callback changes identity each render; keep the latest
  // in a ref so the window-level drag-drop subscription can stay mounted once.
  const addAttachmentsRef = useRef(onAddAttachments);
  addAttachmentsRef.current = onAddAttachments;
  const workstreamId = view.item.id;

  const started = Boolean(shell.sessionId);
  const hasEntry = Boolean(session);
  const mode = asStageKey(shell.mode);
  const model = modelOption(shell.model);
  const effort = effortOption(shell.effort);
  const permissionMode = permissionModeOption(shell.permissionMode);

  // A tab bound to a session but with no transcript in memory means we came
  // back after an app restart — rebuild it from the persisted event log.
  useEffect(() => {
    if (shell.sessionId && !hasEntry) {
      void restoreSessionIfNeeded(shell.sessionId, view.item.id, shell.mode, permissionMode.flag);
    }
  }, [shell.sessionId, hasEntry, view.item.id, shell.mode, permissionMode.flag]);

  // Drag-and-drop. Tauri intercepts OS file drops at the window level (HTML5
  // drop events never fire), so we subscribe to the webview's drag-drop stream
  // and hit-test the drop position (physical px → CSS px) against THIS pane's
  // container — each pane handles only drops that land on it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    const within = (pos?: { x: number; y: number }) => {
      const el = dropRef.current;
      if (!el || !pos) return false;
      const r = el.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const x = pos.x / dpr;
      const y = pos.y / dpr;
      return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
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
        model: model.flag,
        effort: effort.key,
        permissionMode: permissionMode.flag,
      });
      onSessionStarted(created.id);
      onDraftChange("");
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
      onDraftChange("");
      onClearAttachments();
    } catch (e) {
      setError(errorMessage(e));
    }
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
    <div ref={dropRef} className="relative flex min-h-0 flex-1 flex-col">
      {dragOver && (
        <div className="pointer-events-none absolute inset-0 z-40 m-2 flex items-center justify-center rounded-xl border-2 border-dashed border-accent bg-accent/10">
          <div className="flex items-center gap-2 rounded-lg bg-panel px-3.5 py-2 text-sm font-medium text-ink shadow-lg">
            <ImageIcon size={16} />
            Drop to attach
          </div>
        </div>
      )}
      {started ? (
        <TranscriptView
          items={session?.items ?? []}
          busy={working}
          connecting={!session || session.state === "starting"}
          workingSince={session?.workingSince ?? null}
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

      <div className="p-4">
        <div className="mx-auto w-4/5">
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
              <div className="rounded-xl bg-panel-2">
                {attachments.length > 0 && (
                  <div className="px-3.5 pt-3">
                    <AttachmentCards attachments={attachments} onRemove={onRemoveAttachment} />
                  </div>
                )}
                <textarea
                  value={draft}
                  onChange={(e) => onDraftChange(e.target.value)}
                  onKeyDown={onKeyDown}
                  onPaste={onPaste}
                  rows={3}
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
                  className="selectable block w-full resize-none bg-transparent px-3.5 py-3 text-sm text-ink placeholder:text-ink-dim/60 focus:outline-none disabled:cursor-not-allowed"
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
                    custom={{ label: "Custom…", placeholder: "alias or claude-…" }}
                  />
                  <ComposerMenu
                    title="Reasoning effort"
                    leading={<EffortIcon size={14} />}
                    label={effort.label}
                    items={EFFORTS.map((e) => ({ key: e.key, label: e.label }))}
                    activeKey={effort.key}
                    onSelect={onEffortChange}
                  />
                  {/* Permission posture for the next session this tab starts.
                      "Bypass permissions" runs claude with no approval prompts —
                      same as the CLI's --dangerously-skip-permissions. Fixed at
                      spawn, so a change applies to the next session, not this one. */}
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
                  />

                  <div className="ml-auto flex items-center gap-1">
                    <AttachMenu
                      onLibrary={onBrowseDocs}
                      onFiles={onAddFiles}
                      onScreenshots={onAddScreenshots}
                      showLibrary={!started}
                    />
                    {/* Drop or paste a screenshot anytime — see the pane drop overlay. */}
                    {session?.contextTokens != null && (
                      <ContextMeter tokens={session.contextTokens} windowSize={model.contextWindow} />
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
