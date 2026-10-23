import {
  createContext,
  forwardRef,
  memo,
  useCallback,
  useContext,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
  type WheelEvent,
} from "react";

import { Button, Skeleton, Spinner, cx } from "@/components/ui";
import { useCopyFeedback } from "@/lib/clipboard";
import { HyprlayerMark } from "@/components/Logo";
import { Icon } from "@/components/Icon";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { stageLabel } from "@/lib/domain/stages";
import type { ClientRef, Goal, GoalCheckResult, GoalIteration } from "@/lib/bindings";
import {
  GOAL_CONTROL_LABEL,
  GOAL_END_REASON_LABEL,
  GOAL_PAUSE_REASON_LABEL,
  goalBudgetUnit,
  goalControls,
  goalSpent,
} from "@/lib/domain/goals";
import type { SyncPhase } from "@/lib/domain/sync";
import { exitLabel, type TranscriptItem } from "./transcript";
import type { PendingTurn } from "./transcriptSync";
import { backgroundTasksFromItems } from "./backgroundTask";
import { AttachmentCards } from "./AttachmentCard";
import { FileCard } from "./FileCard";
import { PromptCard } from "./PromptCard";
import { STEP_ROW, STEP_ROW_STATIC, StepChevron } from "./stepRow";
import type { Attachment } from "./attachments";
import type { QueuedMessage } from "./sessionStore";
import { formatElapsed, formatRelativeTime, formatTokens, formatUsd, plural } from "@/lib/format";
import { useTickingNow } from "@/lib/useTickingNow";
import { PlanCardsContext, planCardComponent } from "./planCardSlot";
import { readyCardComponent, type ReadyCardProps } from "./readyCardSlot";
import { goalHandoffComponent } from "./goalHandoffSlot";
import { REVIEW_PROMPT, reviewCardComponent } from "./reviewCardSlot";
import { GOAL_PROMPT } from "./goalPrompt";
import { useGoalControl } from "./sessionGoal";
import { armResumeAfterSend } from "./sessionStore";
import { requestComposerFocus, useWorkstreamTabsStore } from "./workstreamTabs";
import { SkillChip } from "./SkillChip";
import {
  baseName,
  editDiffFromTool,
  parseToolInput,
  prettyToolInput,
  resultPreview,
  subagentType,
  toolFilePath,
  toolSummary,
} from "./toolDisplay";

export type PermissionHandlers = {
  onAllow: (requestId: string) => void;
  onAllowForSession: (requestId: string, toolName: string) => void;
  onDeny: (requestId: string) => void;
};

export type PromptHandlers = {
  onRewind: (itemId: string, text: string) => void;
};


function UserMessageToolbar({
  itemId,
  text,
  createdAt,
  onRewind,
}: {
  itemId: string;
  text: string;
  createdAt: number | null;
  onRewind: (id: string, text: string) => void;
}) {
  const { copied, trigger } = useCopyFeedback();
  const now = useTickingNow(createdAt);
  const rel = createdAt === null ? "" : formatRelativeTime(createdAt, now);
  const btn =
    "flex h-6 w-6 cursor-pointer items-center justify-center rounded text-ink-dim opacity-0 transition hover:bg-edge/60 hover:text-ink group-hover:opacity-100";
  return (
    <div className="mt-1 flex items-center justify-end gap-1 text-ui-sm text-ink-dim">
      <button
        type="button"
        onClick={() => trigger(text)}
        className={btn}
        title={copied ? "Copied" : "Copy prompt"}
        aria-label="Copy prompt"
      >
        <Icon name="copy" size="xs" />
      </button>
      <button
        type="button"
        onClick={() => onRewind(itemId, text)}
        className={btn}
        title="Rewind to here"
        aria-label="Rewind to here"
      >
        <Icon name="rewind" size="xs" />
      </button>
      {rel && <span className="ml-1 tabular-nums">{rel}</span>}
    </div>
  );
}

function RemoteChip({ device, createdAt }: { device: string; createdAt: number | null }) {
  const now = useTickingNow(createdAt);
  return (
    <span className="mb-1 inline-flex items-center gap-1 rounded-control bg-info/10 px-1.5 py-0.5 text-tag normal-case text-info">
      <Icon name="device" size="xs" />
      from {device}
      {createdAt !== null && ` · ${formatRelativeTime(createdAt, now)}`}
    </span>
  );
}

const UserMessage = memo(function UserMessage({
  id,
  text,
  stage,
  attachments,
  createdAt,
  promptName,
  prompt,
  author,
  selfClientId,
}: {
  id: string;
  text: string;
  stage: string | null;
  attachments: Attachment[];
  createdAt: number | null;
  promptName: string | null;
  prompt: PromptHandlers;
  author: ClientRef | null;
  selfClientId: string | null;
}) {
  // Unknown identity attributes nothing, as in routing.
  const remote = author && selfClientId !== null && author.clientId !== selfClientId ? author : null;
  const workstreamId = useContext(PlanCardsContext)?.workstreamId ?? null;
  const ReviewCard = promptName === REVIEW_PROMPT ? reviewCardComponent() : null;
  return (
    <div className="group flex flex-col items-end">
      <div className="max-w-[85%] rounded-lg border border-edge bg-panel-2 px-3.5 py-2.5">
        {remote && <RemoteChip device={remote.device} createdAt={createdAt} />}
        {stage && stage !== "general" && (
          <span className="mb-1 block text-label text-ink-dim">
            {stageLabel(stage)}
          </span>
        )}
        {text &&
          (ReviewCard && promptName ? (
            <ReviewCard
              text={text}
              workstreamId={workstreamId}
              fallback={<PromptCard name={promptName} body={text} />}
            />
          ) : promptName ? (
            <PromptCard name={promptName} body={text} />
          ) : (
            <p className="selectable whitespace-pre-wrap break-words text-body text-ink">{text}</p>
          ))}
        <AttachmentCards attachments={attachments} className={text ? "mt-2" : ""} />
      </div>
      <UserMessageToolbar
        itemId={id}
        text={text}
        createdAt={createdAt}
        onRewind={prompt.onRewind}
      />
    </div>
  );
});

// Greyed until the harness takes it in; its echo then lands as a regular message.
function QueuedUserMessage({ message, onCancel }: { message: QueuedMessage; onCancel: (() => void) | null }) {
  const { text, attachments, promptName, clientMessageId } = message;
  return (
    <div className="flex flex-col items-end">
      <div className="max-w-[85%] rounded-lg border border-dashed border-edge bg-panel-2/60 px-3.5 py-2.5 opacity-60">
        {text &&
          (promptName ? (
            <PromptCard name={promptName} body={text} />
          ) : (
            <p className="selectable whitespace-pre-wrap break-words text-body text-ink-dim">{text}</p>
          ))}
        <AttachmentCards attachments={attachments} className={text ? "mt-2" : ""} />
      </div>
      <div className="mt-1 flex items-center justify-end gap-1 text-ui-sm text-ink-dim">
        <Icon name="queued" size="xs" />
        {clientMessageId ? "Steering" : "Queued"}
        {onCancel && (
          <button
            type="button"
            onClick={onCancel}
            title="Cancel queued message"
            aria-label="Cancel queued message"
            className="ml-0.5 flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
          >
            <Icon name="close" size="xs" />
          </button>
        )}
      </div>
    </div>
  );
}

function useSmoothText(text: string, streaming: boolean): string {
  const [revealed, setRevealed] = useState(() => (streaming ? 0 : text.length));
  const targetLen = useRef(text.length);
  targetLen.current = text.length;

  useEffect(() => {
    if (!streaming) {
      setRevealed(targetLen.current);
      return;
    }
    let last = 0;
    let frame = requestAnimationFrame(function tick(ts) {
      if (ts - last >= 33) {
        last = ts;
        setRevealed((cur) => {
          const target = targetLen.current;
          if (cur >= target) return cur;
          return Math.min(target, cur + Math.max(2, Math.ceil((target - cur) / 6)));
        });
      }
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [streaming]);

  return text.slice(0, Math.min(revealed, text.length));
}

const StreamCursor = () => (
  <span className="ml-0.5 inline-block h-3.5 w-1.5 translate-y-0.5 animate-pulse rounded-sm bg-ink-dim align-baseline" />
);

const AssistantMessage = memo(function AssistantMessage({
  text,
  streaming,
}: {
  text: string;
  streaming: boolean;
}) {
  const shown = useSmoothText(text, streaming);

  if (streaming) {
    return (
      <div className="selectable">
        {shown ? (
          <MarkdownPreview body={shown} />
        ) : (
          <span className="text-body leading-prose text-ink-dim">…</span>
        )}
        <StreamCursor />
      </div>
    );
  }

  return (
    <div className="selectable">
      <MarkdownPreview body={text} />
    </div>
  );
});


function ToolResultOutput({ result }: { result: { content: string; isError: boolean } }) {
  return (
    <pre
      className={`selectable max-h-96 overflow-auto rounded p-2 text-meta ${
        result.isError ? "bg-bad/10 text-bad" : "bg-surface/60 text-ink-dim"
      }`}
    >
      {result.content || (result.isError ? "(error, no output)" : "(no output)")}
    </pre>
  );
}

function Collapsible({
  open,
  onToggle,
  header,
  children,
}: {
  open: boolean;
  onToggle: () => void;
  header: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="overflow-hidden rounded-md border border-edge bg-panel-2">
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-edge/40"
      >
        {header}
        <Icon
          name="chevronRight"
          size="xs"
          className={`ml-auto text-ink-dim transition-transform ${open ? "rotate-90" : ""}`}
        />
      </button>
      {open && <div className="border-t border-edge">{children}</div>}
    </div>
  );
}

const ThinkingBlock = memo(function ThinkingBlock({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const preview = text.split("\n").find((line) => line.trim() !== "") ?? "";
  return (
    <div className="text-ui">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`${STEP_ROW} text-ink-dim`}
      >
        <span className="shrink-0 font-medium">Thinking</span>
        {!open && preview && <span className="min-w-0 flex-1 truncate italic">{preview}</span>}
        <StepChevron open={open} reveal="hover" />
      </button>
      {open && (
        <div className="selectable mt-1 whitespace-pre-wrap border-l border-edge pl-3 italic leading-prose text-ink-dim">
          {text}
        </div>
      )}
    </div>
  );
});

// A backgrounded spawn answers with a launch ack in milliseconds, so `result`
// says nothing about whether its work is still going; only the open-task set does.
const OpenBackgroundIds = createContext<ReadonlySet<string>>(new Set<string>());

const JumpTargetId = createContext<string | null>(null);

function useBackgroundRunning(toolUseId: string): boolean {
  return useContext(OpenBackgroundIds).has(toolUseId);
}

function ToolStatusDot({
  result,
  busy,
  running,
}: {
  result: { isError: boolean } | null;
  busy: boolean;
  running: boolean;
}) {
  if (running || (!result && busy)) return <Spinner size="xs" />;
  if (result) {
    return (
      <span
        className={`h-2 w-2 shrink-0 rounded-full ${result.isError ? "bg-bad" : "bg-ok"}`}
        aria-hidden="true"
      />
    );
  }
  return <span className="h-2 w-2 shrink-0 rounded-full bg-ink-dim/50" aria-hidden="true" />;
}

function FileChip({ path }: { path: string }) {
  return (
    <span
      className="min-w-0 shrink truncate rounded bg-edge/60 px-1.5 py-0.5 text-meta text-ink-dim"
      title={path}
    >
      {baseName(path)}
    </span>
  );
}

function ToolArg({ name, input }: { name: string; input: string }) {
  const filePath = toolFilePath(name, input);
  if (filePath) {
    return <FileCard path={filePath} diff={editDiffFromTool(name, input)} />;
  }
  const summary = toolSummary(name, input);
  if (!summary || summary === subagentType(name, input)) return null;
  return (
    <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={summary}>
      {summary}
    </span>
  );
}

const ToolBlock = memo(function ToolBlock({
  item,
  busy,
}: {
  item: Extract<TranscriptItem, { kind: "tool" }>;
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const result = item.result;
  const bgRunning = useBackgroundRunning(item.toolUseId);
  const isFileTool = toolFilePath(item.name, item.input) !== null;
  const isSkill = item.name === "Skill";
  const preview =
    result && !((isFileTool || isSkill) && !result.isError)
      ? resultPreview(result.content, result.isError)
      : null;

  return (
    <div className="text-ui" data-tooluse-id={item.toolUseId}>
      <div
        role="button"
        tabIndex={0}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (e.key !== "Enter" && e.key !== " ") return;
          e.preventDefault();
          setOpen((v) => !v);
        }}
        className={`${STEP_ROW} cursor-pointer`}
      >
        <ToolStatusDot result={result} busy={busy} running={bgRunning} />
        <span className="shrink-0 font-mono font-medium text-ink">
          {subagentType(item.name, item.input) || item.name}
        </span>
        <ToolArg name={item.name} input={item.input} />
        <StepChevron open={open} reveal="hover" />
      </div>

      {!open && preview && (
        <div className="flex gap-1.5 pl-4 text-ink-dim">
          <span aria-hidden="true" className="select-none text-ink-dim/50">
            ⎿
          </span>
          <span
            className={`truncate text-meta ${result?.isError ? "text-bad/90" : ""}`}
          >
            {preview}
          </span>
        </div>
      )}

      {open && (
        <div className="mt-1 space-y-2 border-l border-edge pl-3">
          {item.input && item.input !== "{}" && (
            <pre className="selectable max-h-48 overflow-auto rounded bg-surface/60 p-2 text-meta text-ink-dim">
              {prettyToolInput(item.input)}
            </pre>
          )}
          {item.skillBody && (
            <div className="selectable">
              <div className="mb-1 text-label text-ink-dim/70">
                Skill instructions
              </div>
              <MarkdownPreview body={item.skillBody} />
            </div>
          )}
          {result && !isSkill && <ToolResultOutput result={result} />}
        </div>
      )}
    </div>
  );
});

function ToolItem({ item, busy }: { item: Extract<TranscriptItem, { kind: "tool" }>; busy: boolean }) {
  const planCards = useContext(PlanCardsContext);
  const revision = planCards?.revisions.get(item.toolUseId);
  const PlanCard = planCardComponent();
  if (planCards && revision && PlanCard) {
    return (
      <PlanCard
        sessionId={planCards.sessionId}
        workstreamId={planCards.workstreamId}
        toolUseId={item.toolUseId}
        {...revision}
      />
    );
  }
  if (item.name === "Skill" && !item.result?.isError) {
    const skill = parseToolInput(item.input)?.skill;
    if (typeof skill === "string" && skill) {
      return <SkillChip name={skill} body={item.skillBody ?? null} toolUseId={item.toolUseId} />;
    }
  }
  return <ToolBlock item={item} busy={busy} />;
}

const PermissionCard = memo(function PermissionCard({
  item,
  handlers,
}: {
  item: Extract<TranscriptItem, { kind: "permission" }>;
  handlers: PermissionHandlers;
}) {
  const filePath = toolFilePath(item.toolName, item.input);
  const summary = toolSummary(item.toolName, item.input);

  if (item.decision === "expired") {
    return (
      <div className="flex min-w-0 items-center gap-2 text-ui text-ink-dim">
        <Icon name="permissions" size="xs" />
        <span className="shrink-0 font-mono text-ink">{item.toolName}</span>
        {summary && <span className="min-w-0 truncate font-mono">{summary}</span>}
        <span className="shrink-0">· expired</span>
      </div>
    );
  }
  if (item.decision !== null) {
    if (item.decision === "allow") return null;
    return (
      <div className="flex items-center gap-2 text-ui text-ink-dim">
        <Icon name="permissions" size="xs" />
        <span className="font-mono text-ink">{item.toolName}</span>
        <span>· denied</span>
      </div>
    );
  }

  return (
    <div className="rounded-card border border-q-card-border bg-q-card-bg p-3.5 shadow-card">
      <div className="flex items-center gap-2 text-q-label">
        <Icon name="permissions" size="sm" />
        <span className="text-body font-medium">Permission required</span>
      </div>
      <div className="mt-2 flex items-center gap-1.5 rounded-control border border-warn/20 bg-surface/40 px-2.5 py-2">
        <span className="shrink-0 text-meta font-medium text-ink">{item.toolName}</span>
        {filePath ? (
          <FileChip path={filePath} />
        ) : (
          summary && (
            <span className="min-w-0 break-all text-meta text-ink-dim">{summary}</span>
          )
        )}
      </div>
      <div className="mt-2.5 flex flex-wrap gap-2">
        <Button variant="primary" onClick={() => handlers.onAllow(item.requestId)}>
          Allow once
        </Button>
        <Button onClick={() => handlers.onAllowForSession(item.requestId, item.toolName)}>
          Allow for this agent
        </Button>
        <Button variant="danger" onClick={() => handlers.onDeny(item.requestId)}>
          Deny
        </Button>
      </div>
    </div>
  );
});

const ProtocolErrorBlock = memo(function ProtocolErrorBlock({
  detail,
  raw,
}: {
  detail: string;
  raw: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Collapsible
      open={open}
      onToggle={() => setOpen((v) => !v)}
      header={
        <span className="inline-flex items-center gap-1 text-ui text-ink-dim">
          <Icon name="warning" size="xs" className="text-warn" />
          {detail}
        </span>
      }
    >
      <pre className="selectable max-h-48 overflow-auto px-3 py-2 text-meta text-ink-dim">
        {raw}
      </pre>
    </Collapsible>
  );
});

function exitRemedy(stderrTail: string): string | null {
  if (/\/login\b|not logged in|unauthor/i.test(stderrTail)) {
    return "claude isn't logged in. Run `claude /login` in a terminal, then start a new session.";
  }
  return null;
}

const CompactDivider = memo(function CompactDivider({
  item,
}: {
  item: Extract<TranscriptItem, { kind: "compact" }>;
}) {
  const [open, setOpen] = useState(false);
  const label = item.trigger === "manual" ? "Conversation compacted" : "Conversation auto-compacted";
  const tokens = item.preTokens != null ? ` · ${formatTokens(item.preTokens)} before` : "";
  const { summary } = item;
  return (
    <div className="text-label text-ink-dim/70">
      <div className="flex items-center gap-2">
        <span className="h-px flex-1 bg-edge" aria-hidden />
        {summary ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            className="flex shrink-0 items-center gap-1 rounded px-1 py-0.5 transition hover:bg-panel-2 hover:text-ink"
            title="claude summarized the prior turns; expand to read the summary."
          >
            <span>
              {label}
              {tokens}
            </span>
            <Icon
              name="chevronRight"
              size="xs"
              className={`transition-transform ${open ? "rotate-90" : ""}`}
            />
          </button>
        ) : (
          <span
            className="shrink-0"
            title="claude summarized the prior turns; the next turn continues on the summary."
          >
            {label}
            {tokens}
          </span>
        )}
        <span className="h-px flex-1 bg-edge" aria-hidden />
      </div>
      {open && summary && (
        <div className="selectable mt-2 max-h-96 overflow-auto rounded-md border border-edge bg-panel-2 p-3 font-sans normal-case tracking-normal text-ink-dim">
          <MarkdownPreview body={summary} />
        </div>
      )}
    </div>
  );
});

const BoundaryDivider = memo(function BoundaryDivider({
  label,
  hint,
}: {
  label: string;
  hint: string;
}) {
  return (
    <div className="flex items-center gap-2 text-label text-ink-dim/70">
      <span className="h-px flex-1 bg-edge" aria-hidden />
      <span className="shrink-0" title={hint}>
        {label}
      </span>
      <span className="h-px flex-1 bg-edge" aria-hidden />
    </div>
  );
});

/** The session's newest goal and its iterations, for the dividers, check cards and finished card. */
export type TranscriptGoal = {
  sessionId: string;
  workstreamId: string;
  goal: Goal;
  iterations: GoalIteration[] | null;
};

const GoalTranscript = createContext<TranscriptGoal | null>(null);

function useIteration(clientMessageId: string | null | undefined): GoalIteration | null {
  const ctx = useContext(GoalTranscript);
  if (!clientMessageId) return null;
  return ctx?.iterations?.find((it) => it.clientMessageId === clientMessageId) ?? null;
}

export function iterationSpend(goal: Goal, iteration: GoalIteration): string | null {
  if (goalBudgetUnit(goal) === "usd") return iteration.costUsd == null ? null : formatUsd(iteration.costUsd);
  return iteration.tokens == null ? null : `${formatTokens(iteration.tokens)} tokens`;
}

// Replaces the iteration's prompt bubble; an earlier goal's iteration isn't in the detail.
const GoalIterationDivider = memo(function GoalIterationDivider({
  item,
}: {
  item: Extract<TranscriptItem, { kind: "user" }>;
}) {
  const [open, setOpen] = useState(false);
  const goal = useContext(GoalTranscript)?.goal ?? null;
  const iteration = useIteration(item.clientMessageId);
  const parts =
    goal && iteration ? [`Iteration ${iteration.n} of ${goal.limits.maxIterations}`] : ["Goal iteration"];
  if (goal && iteration?.startedAt != null && iteration.endedAt != null) {
    parts.push(formatElapsed(iteration.endedAt - iteration.startedAt));
    const spend = iterationSpend(goal, iteration);
    if (spend) parts.push(spend);
  }
  return (
    <div data-item-id={item.id} data-testid="goal-iteration-divider" className="text-label text-ink-dim/70">
      <div className="flex items-center gap-2">
        <span className="h-px flex-1 bg-edge" aria-hidden />
        <span className="flex shrink-0 items-center gap-1.5">
          <Icon name="goal" size="xs" />
          {parts.join(" · ")}
        </span>
        <span className="h-px flex-1 bg-edge" aria-hidden />
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="shrink-0 rounded px-1 py-0.5 transition hover:bg-panel-2 hover:text-ink focus-visible:bg-panel-2 focus-visible:text-ink"
        >
          {open ? "Hide prompt" : "Show prompt"}
        </button>
      </div>
      {open && (
        <p className="selectable mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-md border border-edge bg-panel-2 p-3 font-sans text-body normal-case tracking-normal text-ink-dim">
          {item.text}
        </p>
      )}
    </div>
  );
});

const NativeContinuationRow = memo(function NativeContinuationRow({
  item,
}: {
  item: Extract<TranscriptItem, { kind: "nativeContinuation" }>;
}) {
  return (
    <div className="text-ui" data-item-id={item.id} data-testid="native-continuation">
      <div className={STEP_ROW_STATIC}>
        <Icon name="iteration" size="xs" className="text-ink-dim" />
        <span className="shrink-0 font-medium text-ink">
          {item.native === "goal" ? "Native /goal not met:" : "Stop hook continued the turn:"}
        </span>
        <span className="min-w-0 flex-1 truncate text-ink-dim" title={item.reason}>
          {item.reason}
        </span>
      </div>
    </div>
  );
});

const TAIL_LINES = 6;

function CheckTail({ tail }: { tail: string }) {
  const [all, setAll] = useState(false);
  const lines = tail.trimEnd().split("\n");
  const clipped = lines.length > TAIL_LINES;
  return (
    <div className="mt-1 pl-5">
      <pre className="selectable max-h-96 overflow-auto rounded bg-surface/60 p-2 text-meta text-ink-dim">
        {all || !clipped ? tail.trimEnd() : lines.slice(-TAIL_LINES).join("\n")}
      </pre>
      {clipped && (
        <button
          type="button"
          onClick={() => setAll((v) => !v)}
          className="mt-1 rounded px-1 py-0.5 text-ui-sm text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink"
        >
          {all ? "Show less" : "Show all"}
        </button>
      )}
    </div>
  );
}

const CHECK_KIND_LABEL = { plan_boxes: "Plan boxes", evaluator: "Evaluator", validation: "Validation" } as const;

// The result carries no counts: the server writes one finding per tail line, failing checks prefixed.
function validationCounts(tail: string): string | null {
  const findings = tail.split("\n").filter((line) => line.trim());
  const failing = findings.filter((line) => line.startsWith("Failed check: ")).length;
  const issues = findings.length - failing;
  const parts = [issues > 0 && plural(issues, "issue"), failing > 0 && plural(failing, "failing check")];
  return parts.filter(Boolean).join(" · ") || null;
}

function CheckResultRow({ result, workstreamId }: { result: GoalCheckResult; workstreamId: string }) {
  const [open, setOpen] = useState(false);
  const pass = result.outcome === "pass";
  const validation = result.kind === "validation";
  const counts = validation ? validationCounts(result.outputTail) : null;
  const detail =
    result.exitCode !== null ? `exit ${result.exitCode}` : result.outcome === "error" ? "error" : counts;
  const tail = !pass && result.outputTail ? result.outputTail : null;
  const validateSessionId = validation ? (result.sessionId ?? null) : null;
  return (
    <div data-testid={validation ? "goal-check-validation" : undefined}>
      <div className="flex min-w-0 items-center gap-2">
        <Icon
          name={pass ? "check" : "failed"}
          size="xs"
          className={pass ? "shrink-0 text-ok" : "shrink-0 text-bad-ink"}
        />
        <span className="min-w-0 truncate font-mono text-ink" title={result.label}>
          {result.label}
        </span>
        {detail && <span className="shrink-0 font-mono text-ink-dim">{detail}</span>}
        {(validateSessionId || tail) && (
          <span className="ml-auto flex shrink-0 items-center gap-1">
            {validateSessionId && (
              <button
                type="button"
                data-testid="goal-check-open-validate"
                onClick={() => void selectAgentTab(workstreamId, validateSessionId, { frozen: true })}
                className="rounded px-1 py-0.5 text-ui-sm text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink"
              >
                Open
              </button>
            )}
            {tail && (
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                aria-expanded={open}
                className="rounded px-1 py-0.5 text-ui-sm text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink"
              >
                {open ? "Hide output" : "Output"}
              </button>
            )}
          </span>
        )}
      </div>
      {result.reason && (
        <p className="selectable mt-0.5 pl-5 text-ink-dim">
          {result.kind === "evaluator" ? `“${result.reason}”` : result.reason}
        </p>
      )}
      {open && tail && <CheckTail tail={tail} />}
    </div>
  );
}

function checkLabel(check: Goal["checks"][number]): string {
  return check.kind === "command" ? (check.label ?? check.command) : CHECK_KIND_LABEL[check.kind];
}

/** One line per result, then the layers an earlier failure kept from running. */
export function GoalCheckList({ goal, iteration }: { goal: Goal; iteration: GoalIteration }) {
  // Layers run in order and stop at the first failure, so the unrun ones are the tail.
  const ranKinds = new Set(iteration.results.map((r) => r.kind));
  const commandsRun = iteration.results.filter((r) => r.kind === "command").length;
  const skipped = [
    ...goal.checks.filter((c) => c.kind === "command").slice(commandsRun),
    ...goal.checks.filter((c) => c.kind !== "command" && !ranKinds.has(c.kind)),
  ].map(checkLabel);
  return (
    <div className="space-y-1.5 text-ui">
      {iteration.results.map((result, i) => (
        <CheckResultRow key={`${result.label}:${i}`} result={result} workstreamId={goal.workstreamId} />
      ))}
      {skipped.length > 0 && (
        <div className="flex min-w-0 items-center gap-2 text-ink-dim">
          <span className="w-3 shrink-0 text-center" aria-hidden>
            –
          </span>
          <span className="min-w-0 truncate">{skipped.join(", ")}</span>
          <span className="shrink-0">not run</span>
        </div>
      )}
    </div>
  );
}

// Under each finished iteration; output tails mount only when expanded.
function GoalCheckCard({ clientMessageId }: { clientMessageId: string }) {
  const goal = useContext(GoalTranscript)?.goal ?? null;
  const iteration = useIteration(clientMessageId);
  if (!goal || !iteration || iteration.results.length === 0) return null;
  const pass = iteration.outcome === "pass";
  const spend = iterationSpend(goal, iteration);
  return (
    <div
      data-testid="goal-check-card"
      className={cx(
        "mt-3 rounded-card border px-3 py-2.5 text-ui shadow-card",
        pass ? "border-line-strong bg-panel" : "border-fail-card-border bg-fail-card-bg",
      )}
    >
      <div className="flex items-center gap-2">
        <Icon name={pass ? "check" : "failed"} size="xs" className={pass ? "text-ok" : "text-bad-ink"} />
        <span className="font-medium text-ink">
          Checks · iteration {iteration.n} · {iteration.outcome ?? "pending"}
        </span>
        {spend && <span className="ml-auto text-meta text-ink-dim">{spend}</span>}
      </div>
      <div className="mt-2">
        <GoalCheckList goal={goal} iteration={iteration} />
      </div>
    </div>
  );
}

function lastCheckLine(iterations: GoalIteration[] | null): string | null {
  const last = [...(iterations ?? [])].reverse().find((it) => it.results.length > 0);
  if (!last) return null;
  const failed = last.results.find((r) => r.outcome !== "pass");
  if (!failed) return `last check: ${last.results.length} of ${last.results.length} passed`;
  if (failed.exitCode !== null) return `last: ${failed.label} exited ${failed.exitCode}`;
  return `last: ${failed.label}${failed.reason ? `: ${failed.reason}` : ""}`;
}

function goalComposerShell(workstreamId: string, sessionId: string): number | null {
  const shells = useWorkstreamTabsStore.getState().byWs[workstreamId]?.shells ?? [];
  return shells.find((s) => s.sessionId === sessionId)?.id ?? null;
}

const FINISHED_TONE = {
  neutral: "border-line-strong bg-panel",
  failed: "border-fail-card-border bg-fail-card-bg",
  ask: "border-q-card-border bg-q-card-bg",
} as const;

/** Once the session's newest goal has ended, or paused for anything but the user. */
function GoalFinishedCard({ data }: { data: TranscriptGoal }) {
  const { goal, iterations, sessionId, workstreamId } = data;
  const [collapsedFor, setCollapsedFor] = useState<string | null>(null);
  const control = useGoalControl();
  const needsYou = goal.status === "paused" && goal.pauseReason !== null && goal.pauseReason !== "user";
  const ended = goal.status !== "running" && goal.status !== "paused";
  const key = `${goal.id}:${goal.status}:${goal.pauseReason ?? ""}`;
  if ((!ended && !needsYou) || collapsedFor === key) return null;

  const focusComposer = () => {
    const shellId = goalComposerShell(workstreamId, sessionId);
    if (shellId !== null) requestComposerFocus(shellId);
  };
  const takeOver = () => {
    focusComposer();
    setCollapsedFor(key);
  };
  const controls = goalControls(goal).map((c) => (
    <Button key={c} size="sm" onClick={() => control.run(c, goal)}>
      {GOAL_CONTROL_LABEL[c]}
    </Button>
  ));

  if (goal.status === "stopped") {
    return (
      <div data-testid="goal-finished-card" className="mt-3 flex items-center gap-2 text-ui text-ink-dim">
        <Icon name="goal" size="xs" />
        <span>Goal stopped</span>
        {controls}
      </div>
    );
  }

  const last = lastCheckLine(iterations);
  const Handoff = goal.status === "met" ? goalHandoffComponent() : null;
  let title: string;
  let tone: keyof typeof FINISHED_TONE = "neutral";
  let body: ReactNode;
  let actions: ReactNode;
  if (goal.status === "met") {
    title = "Goal met";
    body = [
      `${goal.iteration} ${goal.iteration === 1 ? "iteration" : "iterations"}`,
      goalSpent(goal),
      `${formatElapsed(goal.activeMs)} active`,
      last,
    ]
      .filter(Boolean)
      .join(" · ");
  } else if (needsYou) {
    title = "Goal paused, needs you";
    tone = "ask";
    body = goal.lastReason ?? GOAL_PAUSE_REASON_LABEL[goal.pauseReason!];
    actions = (
      <>
        <Button
          size="sm"
          onClick={() => {
            armResumeAfterSend(sessionId, goal.id);
            focusComposer();
          }}
        >
          Reply and resume
        </Button>
        <Button size="sm" disabled={control.pending} onClick={() => control.run("resume", goal)}>
          {GOAL_CONTROL_LABEL.resume}
        </Button>
        <Button size="sm" disabled={control.pending} onClick={() => control.run("stop", goal)}>
          {GOAL_CONTROL_LABEL.stop}
        </Button>
      </>
    );
  } else {
    const failed = goal.status === "failed";
    title = failed
      ? `Goal failed: ${goal.endReason ? GOAL_END_REASON_LABEL[goal.endReason] : "unknown"}`
      : "Goal stopped at its limit";
    tone = failed ? "failed" : "neutral";
    body = [`${goal.iteration} of ${goal.limits.maxIterations} iterations`, goalSpent(goal), last]
      .filter(Boolean)
      .join(" · ");
    actions = (
      <>
        {controls}
        <Button size="sm" onClick={takeOver}>
          Take over
        </Button>
      </>
    );
  }

  return (
    <div
      data-testid="goal-finished-card"
      className={cx("mt-5 rounded-card border p-3.5 shadow-card", FINISHED_TONE[tone])}
    >
      <div className="flex items-center gap-2">
        <Icon
          name={tone === "ask" ? "warning" : goal.status === "met" ? "goal" : tone === "failed" ? "failed" : "paused"}
          size="sm"
          className={tone === "ask" ? "text-q-label" : tone === "failed" ? "text-bad-ink" : "text-ink-2"}
        />
        <span className="text-body font-medium text-ink">{title}</span>
      </div>
      {goal.status === "met" && <p className="mt-1.5 text-body text-ink">{goal.condition}</p>}
      <p className="selectable mt-1 text-ui text-ink-dim">{body}</p>
      {actions && <div className="mt-2.5 flex flex-wrap gap-2">{actions}</div>}
      {Handoff && <Handoff sessionId={sessionId} workstreamId={workstreamId} goal={goal} />}
    </div>
  );
}

const QueuedCommandRow = memo(function QueuedCommandRow({
  item,
}: {
  item: Extract<TranscriptItem, { kind: "queuedCommand" }>;
}) {
  return (
    <div className="text-ui">
      <div className={STEP_ROW_STATIC}>
        <span className="h-2 w-2 shrink-0 rounded-full bg-accent" aria-hidden="true" />
        <Icon name="sync" size="xs" className="text-ink-dim" />
        <span className="shrink-0 font-mono font-medium text-ink">{item.label}</span>
        {item.detail && (
          <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={item.detail}>
            {item.detail}
          </span>
        )}
      </div>
    </div>
  );
});

const ExitNotice = memo(function ExitNotice({
  item,
}: {
  item: Extract<TranscriptItem, { kind: "exit" }>;
}) {
  const bad = item.outcome === "crashed" || item.outcome === "spawn_failed";
  const remedy = bad ? exitRemedy(item.stderrTail) : null;
  return (
    <div
      className={`rounded-md border px-3 py-2 shadow-card ${
        bad ? "border-fail-card-border bg-fail-card-bg" : "border-line-strong bg-panel"
      }`}
    >
      <p className={`text-center text-ui font-medium ${bad ? "text-bad-ink" : "text-ink-dim"}`}>
        {exitLabel(item.outcome)}
        {item.code !== null && item.code !== 0 ? ` (exit ${item.code})` : ""}
      </p>
      {remedy && <p className="mt-1 text-center text-ui text-ink-dim">{remedy}</p>}
      {bad && !remedy && item.stderrTail && (
        <pre className="selectable mt-2 max-h-40 overflow-auto rounded bg-surface/60 p-2 text-meta text-ink-dim">
          {item.stderrTail}
        </pre>
      )}
    </div>
  );
});

function ElapsedCounter({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 100);
    return () => clearInterval(id);
  }, [since]);
  return (
    <div className="mt-3 flex items-center gap-1.5 text-meta tabular-nums text-ink-dim">
      <HyprlayerMark size={14} animated />
      {formatElapsed(now - since)}
    </div>
  );
}


type ItemKind = TranscriptItem["kind"];

const isQuiet = (kind: ItemKind) => kind === "tool" || kind === "thinking";

function gapForKinds(prev: ItemKind | undefined, cur: ItemKind): string {
  if (!prev) return "";
  if (cur === "user") return "mt-5";
  if (cur === "assistant") return "mt-4";
  if (isQuiet(cur)) return isQuiet(prev) ? "mt-1" : "mt-2";
  return "mt-3";
}

type Unit =
  | { type: "single"; item: TranscriptItem }
  | { type: "group"; id: string; items: TranscriptItem[] };

const unitKind = (u: Unit): ItemKind => (u.type === "group" ? "tool" : u.item.kind);

// File tools and skill chips stay visible rather than folding into a tool group.
const isStandaloneTool = (it: TranscriptItem): boolean =>
  it.kind === "tool" && (it.name === "Skill" || toolFilePath(it.name, it.input) !== null);

const parentIdOf = (it: TranscriptItem): string | null =>
  it.kind === "tool" || it.kind === "assistant" || it.kind === "thinking" || it.kind === "skill"
    ? (it.parentId ?? null)
    : null;

function toUnits(items: TranscriptItem[], keepOut: (it: TranscriptItem) => boolean): Unit[] {
  const units: Unit[] = [];
  let run: TranscriptItem[] = [];
  const flush = () => {
    if (run.length >= 2) units.push({ type: "group", id: run[0].id, items: run });
    else if (run.length === 1) units.push({ type: "single", item: run[0] });
    run = [];
  };
  for (const item of items) {
    const groupable =
      isQuiet(item.kind) &&
      !keepOut(item) &&
      !isStandaloneTool(item);
    if (groupable) {
      run.push(item);
    } else {
      flush();
      units.push({ type: "single", item });
    }
  }
  flush();
  return units;
}

function ToolGroup({ items, busy }: { items: TranscriptItem[]; busy: boolean }) {
  const [open, setOpen] = useState(false);
  const openBg = useContext(OpenBackgroundIds);
  const jump = useContext(JumpTargetId);
  const tools = items.filter((i) => i.kind === "tool");
  const holdsJump =
    jump !== null && tools.some((t) => t.kind === "tool" && t.toolUseId === jump);
  useEffect(() => {
    if (holdsJump) setOpen(true);
  }, [holdsJump]);
  const running = tools.some(
    (t) => t.kind === "tool" && ((busy && t.result === null) || openBg.has(t.toolUseId)),
  );
  const label =
    tools.length > 0
      ? `${tools.length} tool call${tools.length === 1 ? "" : "s"}`
      : `${items.length} steps`;
  return (
    <div className="text-ui">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`${STEP_ROW} text-ink-dim`}
      >
        {running ? <Spinner size="xs" /> : <Icon name="tool" size="xs" className="text-ink-dim" />}
        <span className="font-medium text-ink">{label}</span>
        {running && <span className="text-ink-dim">· working…</span>}
        <StepChevron open={open} />
      </button>
      {open && (
        <div className="mt-1 space-y-1 border-l border-edge pl-3">
          {items.map((item) => {
            if (item.kind === "tool") return <ToolBlock key={item.id} item={item} busy={busy} />;
            if (item.kind === "thinking") return <ThinkingBlock key={item.id} text={item.text} />;
            return null;
          })}
        </div>
      )}
    </div>
  );
}

function renderSingle(
  item: TranscriptItem,
  busy: boolean,
  permission: PermissionHandlers,
  prompt: PromptHandlers,
  selfClientId: string | null,
) {
  switch (item.kind) {
    case "user":
      if (item.promptName === GOAL_PROMPT) return <GoalIterationDivider item={item} />;
      return (
        <UserMessage
          id={item.id}
          text={item.text}
          stage={item.stage}
          attachments={item.attachments}
          createdAt={item.createdAt}
          promptName={item.promptName}
          prompt={prompt}
          author={item.author ?? null}
          selfClientId={selfClientId}
        />
      );
    case "assistant":
      return (
        <div data-item-id={item.id}>
          <AssistantMessage text={item.text} streaming={item.streaming} />
        </div>
      );
    case "thinking":
      return <ThinkingBlock text={item.text} />;
    case "skill":
      return <SkillChip name={item.name} body={item.body} />;
    case "tool":
      return <ToolItem item={item} busy={busy} />;
    case "permission":
      return <PermissionCard item={item} handlers={permission} />;
    case "error":
      return (
        <div data-item-id={item.id}>
          <ProtocolErrorBlock detail={item.detail} raw={item.raw} />
        </div>
      );
    case "compact":
      return <CompactDivider item={item} />;
    case "rewound":
      return (
        <BoundaryDivider
          label="Rewound to here"
          hint="The turns after this point were removed from the transcript."
        />
      );
    case "relaunched":
      return (
        <BoundaryDivider
          label="Agent relaunched"
          hint="The harness process was restarted; the conversation continues on the resumed session."
        />
      );
    case "queuedCommand":
      return <QueuedCommandRow item={item} />;
    case "nativeContinuation":
      return <NativeContinuationRow item={item} />;
    case "exit":
      return (
        <div data-item-id={item.id}>
          <ExitNotice item={item} />
        </div>
      );
    default:
      return null;
  }
}

type ChildMap = Map<string, TranscriptItem[]>;

function SubagentBlock({
  item,
  childItems,
  childrenByParent,
  busy,
  permission,
  prompt,
}: {
  item: Extract<TranscriptItem, { kind: "tool" }>;
  childItems: TranscriptItem[];
  childrenByParent: ChildMap;
  busy: boolean;
  permission: PermissionHandlers;
  prompt: PromptHandlers;
}) {
  const [open, setOpen] = useState(false);
  const type = subagentType(item.name, item.input);
  const summary = toolSummary(item.name, item.input);
  const toolCount = childItems.filter((c) => c.kind === "tool").length;
  const result = item.result;
  const bgRunning = useBackgroundRunning(item.toolUseId);
  const running = bgRunning || (busy && result === null);
  return (
    <div className="text-ui" data-tooluse-id={item.toolUseId}>
      <button type="button" onClick={() => setOpen((v) => !v)} className={STEP_ROW}>
        <ToolStatusDot result={result} busy={busy} running={running} />
        <span className="shrink-0 font-mono font-medium text-ink">
          {item.name === "Workflow" ? "Workflow" : type || "Subagent"}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={summary}>
          {summary !== type && summary}
        </span>
        {toolCount > 0 && (
          <span className="shrink-0 text-ink-dim">
            {toolCount} tool{toolCount === 1 ? "" : "s"}
            {running && " · working…"}
          </span>
        )}
        <StepChevron open={open} />
      </button>
      {open && (
        <div className="mt-1 space-y-2 border-l border-edge pl-3">
          <ItemList
            items={childItems}
            childrenByParent={childrenByParent}
            busy={busy}
            permission={permission}
            prompt={prompt}
          />
          {result && (
            <div>
              <div className="mb-1 text-label text-ink-dim/70">
                Result
              </div>
              <ToolResultOutput result={result} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

const SelfClientId = createContext<string | null>(null);

const unitItems = (u: Unit): TranscriptItem[] => (u.type === "group" ? u.items : [u.item]);

const seqAtLeast = (item: TranscriptItem, seq: number) => item.seq != null && item.seq >= seq;

const goalIterationOf = (u: Unit): string | null =>
  u.type === "single" && u.item.kind === "user" && u.item.promptName === GOAL_PROMPT
    ? (u.item.clientMessageId ?? null)
    : null;

function ItemList({
  items,
  childrenByParent,
  busy,
  permission,
  prompt,
  freshFromSeq = null,
  divider = null,
  goalChecks = false,
}: {
  items: TranscriptItem[];
  childrenByParent: ChildMap;
  busy: boolean;
  permission: PermissionHandlers;
  prompt: PromptHandlers;
  freshFromSeq?: number | null;
  divider?: { seq: number; node: ReactNode } | null;
  /** Top level only: a check card closes each goal iteration. */
  goalChecks?: boolean;
}) {
  const planCards = useContext(PlanCardsContext);
  const selfClientId = useContext(SelfClientId);
  // Subagents and plan cards (a Bash-written plan, a presented one) never fold into a tool group.
  const keepOut = (it: TranscriptItem) =>
    it.kind === "tool" && (childrenByParent.has(it.toolUseId) || Boolean(planCards?.revisions.has(it.toolUseId)));
  const visible = items.filter((it) => it.kind !== "permission" || it.decision !== "allow");
  const units = toUnits(visible, keepOut);

  const renderUnit = (unit: Unit, i: number): ReactNode => {
    const gap = gapForKinds(i > 0 ? unitKind(units[i - 1]) : undefined, unitKind(unit));
    // A group's first item decides, so a group that grows doesn't fade in again.
    const fresh = freshFromSeq !== null && seqAtLeast(unitItems(unit)[0], freshFromSeq);
    const className = cx(gap, fresh && "animate-sync-in");
    if (unit.type === "group") {
      return (
        <div key={`g:${unit.id}`} className={className}>
          <ToolGroup items={unit.items} busy={busy} />
        </div>
      );
    }
    const item = unit.item;
    if (item.kind === "tool" && childrenByParent.has(item.toolUseId)) {
      return (
        <div key={item.id} className={className}>
          <SubagentBlock
            item={item}
            childItems={childrenByParent.get(item.toolUseId)!}
            childrenByParent={childrenByParent}
            busy={busy}
            permission={permission}
            prompt={prompt}
          />
        </div>
      );
    }
    return (
      <div key={item.id} className={className}>
        {renderSingle(item, busy, permission, prompt, selfClientId)}
      </div>
    );
  };

  const dividerAt = divider ? units.findIndex((u) => unitItems(u).some((it) => seqAtLeast(it, divider.seq))) : -1;
  // One flat keyed list, so the divider coming and going never remounts the unit beside it.
  const nodes: ReactNode[] = [];
  let openIteration: string | null = null;
  const closeIteration = () => {
    if (openIteration) nodes.push(<GoalCheckCard key={`goal-check:${openIteration}`} clientMessageId={openIteration} />);
  };
  units.forEach((unit, i) => {
    const iteration = goalChecks ? goalIterationOf(unit) : null;
    if (iteration) {
      closeIteration();
      openIteration = iteration;
    }
    if (divider && i === dividerAt) {
      nodes.push(
        <div key={`divider:${divider.seq}`} className={i > 0 ? "mt-5" : ""}>
          {divider.node}
        </div>,
      );
    }
    nodes.push(renderUnit(unit, i));
  });
  closeIteration();
  return <>{nodes}</>;
}

export type SkeletonTail = {
  phase: SyncPhase;
  turns: PendingTurn[];
  /** Revisions not shown; padded past `turns` when a refusal counted turns no revision named. */
  count: number;
  /** Another device's turn is running; its label replaces the elapsed counter. */
  working: string | null;
};

export type TranscriptSync = {
  tail: SkeletonTail | null;
  divider: { seq: number; device: string } | null;
  freshFromSeq: number | null;
  selfClientId: string | null;
};

const NO_SYNC: TranscriptSync = { tail: null, divider: null, freshFromSeq: null, selfClientId: null };

// One exchange however many turns wait: the count belongs to the composer notice.
function SyncSkeletonTail({ tail }: { tail: SkeletonTail }) {
  if (tail.working || (tail.count === 0 && tail.turns.length === 0)) return null;
  const shimmer = tail.phase === "syncing";
  const avatar = <Skeleton shimmer={shimmer} className="h-[22px] w-[22px] shrink-0 rounded-md" />;
  return (
    <div data-sync-placeholder className="mt-5 space-y-4">
      <div data-sync-row className="flex gap-2.5">
        {avatar}
        <div className="min-w-0 flex-1 space-y-1.5 pt-1">
          <Skeleton shimmer={shimmer} className="h-3 w-32" />
          <Skeleton shimmer={shimmer} className="h-2.5 w-[60%]" />
        </div>
      </div>
      <div data-sync-row className="flex gap-2.5">
        {avatar}
        <div className="min-w-0 flex-1 space-y-1.5">
          <Skeleton shimmer={shimmer} className="h-9 w-full" />
          <Skeleton shimmer={shimmer} className="h-2.5 w-[92%]" />
          <Skeleton shimmer={shimmer} className="h-2.5 w-1/2" />
        </div>
      </div>
    </div>
  );
}

function WorkingSkeleton({ device }: { device: string }) {
  return (
    <div className="mt-3 flex items-center gap-2.5">
      <Skeleton shimmer className="h-[22px] w-[22px] shrink-0 rounded-md" />
      <span className="text-meta text-ink-dim">{device} is working…</span>
    </div>
  );
}

function SyncDivider({
  device,
  rootRef,
  onPassed,
}: {
  device: string;
  rootRef: RefObject<HTMLDivElement | null>;
  onPassed?: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const passed = useRef(onPassed);
  passed.current = onPassed;
  useEffect(() => {
    const el = ref.current;
    const root = rootRef.current;
    if (!el || !root || typeof IntersectionObserver === "undefined") return;
    // A pin to the bottom can carry it off-screen before it was ever shown; that isn't reading past it.
    let seen = false;
    const io = new IntersectionObserver(
      (entries) => {
        const entry = entries[entries.length - 1];
        if (entry.isIntersecting) {
          seen = true;
          return;
        }
        // Only leaving through the top means it was read past; below the fold it is still ahead.
        if (!seen || !entry.rootBounds || entry.boundingClientRect.bottom >= entry.rootBounds.top) return;
        io.disconnect();
        passed.current?.();
      },
      { root },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [rootRef]);
  return (
    <div ref={ref} className="flex items-center gap-2 text-label text-info">
      <span className="h-px flex-1 bg-info/30" aria-hidden />
      <span className="shrink-0">New from {device}</span>
      <span className="h-px flex-1 bg-info/30" aria-hidden />
    </div>
  );
}

type Props = {
  items: TranscriptItem[];
  busy: boolean;
  connecting: boolean;
  harnessLabel: string;
  workingSince: number | null;
  permission: PermissionHandlers;
  prompt: PromptHandlers;
  sync?: TranscriptSync;
  // Set for an implement session; the review feature decides whether the card shows.
  ready?: ReadyCardProps | null;
  goal?: TranscriptGoal | null;
  queued?: QueuedMessage[];
  /** Whether the harness can give back a message it already holds (codex cannot). */
  canWithdrawQueued?: boolean;
  onCancelQueued?: (index: number) => void;
  onBottomChange?: (atBottom: boolean) => void;
  onDividerPassed?: () => void;
};

export type TranscriptHandle = {
  scrollToTool: (toolUseId: string) => void;
  scrollToItem: (itemId: string) => void;
  scrollToBottom: () => void;
};

const NO_QUEUED: QueuedMessage[] = [];

const RESTICK_PX = 24;

const RELOCK_MS = 200;

const JUMP_SETTLE_MS = 150;

const JUMP_MAX_FRAMES = 10;

const SEND_GLIDE_FACTOR = 0.12;
const SEND_GLIDE_MS = 600;

export function stickAfterScroll(prevTop: number, top: number, gap: number, wasStuck: boolean): boolean {
  if (top < prevTop - 1 && gap > 2) return false;
  if (gap < RESTICK_PX) return true;
  return wasStuck;
}

export function releaseOnWheel(deltaX: number, deltaY: number, ctrlKey: boolean, scrollTop: number): boolean {
  if (deltaY >= 0 || ctrlKey) return false;
  if (Math.abs(deltaY) <= Math.abs(deltaX)) return false;
  return scrollTop > 0;
}

export function insideNestedScroller(target: EventTarget | null, root: HTMLElement): boolean {
  for (let el = target instanceof Element ? target : null; el && el !== root; el = el.parentElement) {
    if (el.scrollTop > 0) return true;
    if (el.scrollHeight > el.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(el).overflowY)) return true;
  }
  return false;
}

export const TranscriptView = memo(
  forwardRef<TranscriptHandle, Props>(function TranscriptView(
    {
      items,
      busy,
      connecting,
      harnessLabel,
      workingSince,
      permission,
      prompt,
      sync = NO_SYNC,
      ready = null,
      goal = null,
      queued = NO_QUEUED,
      canWithdrawQueued = true,
      onCancelQueued,
      onBottomChange,
      onDividerPassed,
    },
    ref,
  ) {
    const scrollRef = useRef<HTMLDivElement>(null);
    const contentRef = useRef<HTMLDivElement>(null);
    const atBottom = useRef(true);
    const bottomChanged = useRef(onBottomChange);
    bottomChanged.current = onBottomChange;
    const setAtBottom = useCallback((next: boolean) => {
      if (atBottom.current === next) return;
      atBottom.current = next;
      bottomChanged.current?.(next);
    }, []);
    const lastTop = useRef(0);
    const relockUntil = useRef(0);
    const easeFrame = useRef(0);
    const jumpNonce = useRef(0);
    const jumping = useRef(false);
    const jumpSettle = useRef<ReturnType<typeof setTimeout> | null>(null);
    const [jump, setJump] = useState<{
      selector: string;
      toolUseId: string | null;
      nonce: number;
    } | null>(null);

    const settleJump = useCallback(() => {
      if (jumpSettle.current) clearTimeout(jumpSettle.current);
      jumpSettle.current = setTimeout(() => {
        jumping.current = false;
        jumpSettle.current = null;
      }, JUMP_SETTLE_MS);
    }, []);

    // The group holding the target expands via JumpTargetId in a child effect, which
    // commits a render after this one — so the anchor may not exist yet on the first try.
    useEffect(() => {
      if (!jump) return;
      let frame = 0;
      let tries = 0;
      const attempt = () => {
        const el = scrollRef.current?.querySelector(jump.selector);
        if (!el) {
          if (tries++ < JUMP_MAX_FRAMES) frame = requestAnimationFrame(attempt);
          return;
        }
        setAtBottom(false);
        jumping.current = true;
        settleJump();
        // Centering a target taller than the pane would land mid-message.
        const tall = el.getBoundingClientRect().height > (scrollRef.current?.clientHeight ?? 0);
        el.scrollIntoView({ behavior: "smooth", block: tall ? "start" : "center" });
      };
      attempt();
      return () => {
        if (frame) cancelAnimationFrame(frame);
      };
    }, [jump, settleJump, setAtBottom]);

    useEffect(
      () => () => {
        if (jumpSettle.current) clearTimeout(jumpSettle.current);
      },
      [],
    );

    const pin = useCallback((factor = 0.3) => {
      const el = scrollRef.current;
      if (!el || !atBottom.current || jumping.current) return;
      const target = el.scrollHeight - el.clientHeight;
      const gap = target - el.scrollTop;
      if (gap <= 0) return;
      if (gap > 2 && performance.now() < relockUntil.current) {
        el.scrollTop += Math.max(1, gap * factor);
      } else {
        el.scrollTop = target;
      }
    }, []);

    useImperativeHandle(ref, () => ({
      scrollToTool(toolUseId) {
        setJump({
          selector: `[data-tooluse-id="${CSS.escape(toolUseId)}"]`,
          toolUseId,
          nonce: jumpNonce.current++,
        });
      },
      scrollToItem(itemId) {
        setJump({
          selector: `[data-item-id="${CSS.escape(itemId)}"]`,
          toolUseId: null,
          nonce: jumpNonce.current++,
        });
      },
      scrollToBottom() {
        setAtBottom(true);
        const el = scrollRef.current;
        if (!el || el.scrollHeight - el.clientHeight - el.scrollTop <= 0) return;
        relockUntil.current = performance.now() + SEND_GLIDE_MS;
        if (easeFrame.current) return;
        const tick = () => {
          pin(SEND_GLIDE_FACTOR);
          if (performance.now() < relockUntil.current) {
            easeFrame.current = requestAnimationFrame(tick);
          } else {
            pin(SEND_GLIDE_FACTOR);
            easeFrame.current = 0;
          }
        };
        easeFrame.current = requestAnimationFrame(tick);
      },
    }));

    const onScroll = () => {
      const el = scrollRef.current;
      if (!el) return;
      const top = el.scrollTop;
      // A jump owns the scroll position until it settles. Re-sticking mid-flight would
      // hand it straight back to the pin loop, which drags it to the bottom again.
      if (jumping.current) {
        lastTop.current = top;
        settleJump();
        return;
      }
      const gap = el.scrollHeight - el.clientHeight - top;
      const wasBottom = atBottom.current;
      setAtBottom(stickAfterScroll(lastTop.current, top, gap, wasBottom));
      if (!wasBottom && atBottom.current) relockUntil.current = performance.now() + RELOCK_MS;
      lastTop.current = top;
    };

    const onWheel = (e: WheelEvent<HTMLDivElement>) => {
      const el = scrollRef.current;
      if (!el || !releaseOnWheel(e.deltaX, e.deltaY, e.ctrlKey, el.scrollTop)) return;
      if (insideNestedScroller(e.target, el)) return;
      setAtBottom(false);
    };

    useEffect(() => {
      if (busy) return;
      const el = scrollRef.current;
      if (!el || !atBottom.current) return;
      if (easeFrame.current && performance.now() < relockUntil.current) return;
      el.scrollTop = el.scrollHeight;
    }, [items, busy]);

    useEffect(() => {
      if (!busy) return;
      let frame = requestAnimationFrame(function tick() {
        pin();
        frame = requestAnimationFrame(tick);
      });
      return () => cancelAnimationFrame(frame);
    }, [busy, pin]);

    useEffect(() => {
      const el = scrollRef.current;
      const content = contentRef.current;
      if (!el || !content || typeof ResizeObserver === "undefined") return;
      const ro = new ResizeObserver(() => pin(1));
      ro.observe(el);
      ro.observe(content);
      return () => ro.disconnect();
    }, [pin]);

    useEffect(
      () => () => {
        if (easeFrame.current) cancelAnimationFrame(easeFrame.current);
      },
      [],
    );

    const openBackgroundIds = useMemo(
      () => new Set(backgroundTasksFromItems(items).map((t) => t.toolUseId)),
      [items],
    );

    const childrenByParent: ChildMap = new Map();
    for (const item of items) {
      const parent = parentIdOf(item);
      if (parent) {
        const arr = childrenByParent.get(parent);
        if (arr) arr.push(item);
        else childrenByParent.set(parent, [item]);
      }
    }
    const topLevel = items.filter((item) => parentIdOf(item) === null);
    const { tail, divider } = sync;
    const ReadyCard = ready ? readyCardComponent() : null;

    return (
      <div
        ref={scrollRef}
        onScroll={onScroll}
        onWheel={onWheel}
        className="-mt-9 min-h-0 flex-1 overflow-y-auto overscroll-contain p-5 pt-14 [overflow-anchor:none]"
      >
        <div ref={contentRef} className="mx-auto w-[90%]">
          <OpenBackgroundIds.Provider value={openBackgroundIds}>
            <JumpTargetId.Provider value={jump?.toolUseId ?? null}>
              <SelfClientId.Provider value={sync.selfClientId}>
                <GoalTranscript.Provider value={goal}>
                  <ItemList
                    items={topLevel}
                    goalChecks
                    childrenByParent={childrenByParent}
                    busy={busy}
                    permission={permission}
                    prompt={prompt}
                    freshFromSeq={sync.freshFromSeq}
                    divider={
                      divider && {
                        seq: divider.seq,
                        node: <SyncDivider device={divider.device} rootRef={scrollRef} onPassed={onDividerPassed} />,
                      }
                    }
                  />
                </GoalTranscript.Provider>
              </SelfClientId.Provider>
            </JumpTargetId.Provider>
          </OpenBackgroundIds.Provider>
          {ReadyCard && ready && <ReadyCard sessionId={ready.sessionId} workstreamId={ready.workstreamId} />}
          {goal && <GoalFinishedCard data={goal} />}
          {tail && <SyncSkeletonTail tail={tail} />}
          {connecting && (
            <div className="mt-3 flex items-center gap-2 text-body text-ink-dim">
              <Spinner size="sm" />
              Connecting to {harnessLabel}…
            </div>
          )}
          {tail?.working ? (
            <WorkingSkeleton device={tail.working} />
          ) : (
            workingSince !== null && <ElapsedCounter since={workingSince} />
          )}
          {queued.map((message, i) => (
            <div key={message.clientMessageId ?? `local:${i}`} className={items.length > 0 || i > 0 ? "mt-5" : ""}>
              <QueuedUserMessage
                message={message}
                onCancel={
                  onCancelQueued && (canWithdrawQueued || !message.clientMessageId)
                    ? () => onCancelQueued(i)
                    : null
                }
              />
            </div>
          ))}
        </div>
      </div>
    );
  }),
);
