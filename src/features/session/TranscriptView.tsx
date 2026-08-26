import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type WheelEvent,
} from "react";

import { Button, Spinner } from "@/components/ui";
import { useCopyFeedback } from "@/lib/clipboard";
import { HyprlayerMark } from "@/components/Logo";
import {
  ChevronRightIcon,
  ClipboardIcon,
  PuzzleIcon,
  RestartIcon,
  RewindIcon,
  ShieldIcon,
  WrenchIcon,
} from "@/components/icons";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { stageLabel } from "@/lib/domain/stages";
import { exitLabel, type TranscriptItem } from "./transcript";
import { AttachmentCards } from "./AttachmentCard";
import { FileCard } from "./FileCard";
import { PromptCard } from "./PromptCard";
import { STEP_ROW, STEP_ROW_STATIC, StepChevron } from "./stepRow";
import type { Attachment } from "./attachments";
import { formatElapsed, formatRelativeTime } from "@/lib/format";
import {
  baseName,
  editDiffFromTool,
  formatTokens,
  prettyToolInput,
  resultPreview,
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


function useTickingNow(epochMs: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (epochMs === null) return;
    let id: number;
    const schedule = () => {
      const ageMs = Date.now() - epochMs;
      const delay = ageMs < 60_000 ? 10_000 : ageMs < 3_600_000 ? 30_000 : 300_000;
      id = window.setTimeout(() => {
        setNow(Date.now());
        schedule();
      }, delay);
    };
    schedule();
    return () => window.clearTimeout(id);
  }, [epochMs]);
  return now;
}

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
    "flex h-6 w-6 cursor-pointer items-center justify-center rounded text-ink-dim opacity-0 transition hover:bg-edge hover:text-ink group-hover:opacity-100";
  return (
    <div className="mt-1 flex items-center justify-end gap-1 text-[11px] text-ink-dim">
      <button
        type="button"
        onClick={() => trigger(text)}
        className={btn}
        title={copied ? "Copied" : "Copy prompt"}
        aria-label="Copy prompt"
      >
        <ClipboardIcon size={12} />
      </button>
      <button
        type="button"
        onClick={() => onRewind(itemId, text)}
        className={btn}
        title="Rewind to here"
        aria-label="Rewind to here"
      >
        <RewindIcon size={12} />
      </button>
      {rel && <span className="ml-1 tabular-nums">{rel}</span>}
    </div>
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
}: {
  id: string;
  text: string;
  stage: string | null;
  attachments: Attachment[];
  createdAt: number | null;
  promptName: string | null;
  prompt: PromptHandlers;
}) {
  return (
    <div className="group flex flex-col items-end">
      <div className="max-w-[85%] rounded-lg border border-edge bg-panel-2 px-3.5 py-2.5">
        {stage && stage !== "general" && (
          <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-dim">
            {stageLabel(stage)}
          </span>
        )}
        {text &&
          (promptName ? (
            <PromptCard name={promptName} body={text} />
          ) : (
            <p className="selectable whitespace-pre-wrap break-words text-sm text-ink">{text}</p>
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
          <span className="text-sm leading-relaxed text-ink-dim">…</span>
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
      className={`selectable max-h-96 overflow-auto rounded p-2 font-mono text-[11px] leading-relaxed ${
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
        <ChevronRightIcon
          size={12}
          className={`ml-auto shrink-0 text-ink-dim transition-transform ${open ? "rotate-90" : ""}`}
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
    <div className="text-xs">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`${STEP_ROW} text-ink-dim`}
      >
        <span className="shrink-0 font-medium">Thinking</span>
        {!open && preview && <span className="min-w-0 flex-1 truncate italic">{preview}</span>}
        <StepChevron open={open} reveal="hover" size={11} />
      </button>
      {open && (
        <div className="selectable mt-1 whitespace-pre-wrap border-l border-edge pl-3 italic leading-relaxed text-ink-dim">
          {text}
        </div>
      )}
    </div>
  );
});

const SkillBlock = memo(function SkillBlock({ name, body }: { name: string; body: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="text-xs">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`${STEP_ROW} text-ink-dim`}
      >
        <PuzzleIcon size={12} className="shrink-0 text-ink-dim" />
        <span className="shrink-0 font-medium text-ink">Skill</span>
        <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={name}>
          {name}
        </span>
        <StepChevron open={open} reveal="hover" size={11} />
      </button>
      {open && (
        <div className="selectable mt-1 border-l border-edge pl-3">
          <MarkdownPreview body={body} />
        </div>
      )}
    </div>
  );
});

function ToolStatusDot({
  result,
  busy,
}: {
  result: { isError: boolean } | null;
  busy: boolean;
}) {
  if (result) {
    return (
      <span
        className={`h-2 w-2 shrink-0 rounded-full ${result.isError ? "bg-bad" : "bg-ok"}`}
        aria-hidden="true"
      />
    );
  }
  if (busy) return <Spinner size={12} />;
  return <span className="h-2 w-2 shrink-0 rounded-full bg-ink-dim/50" aria-hidden="true" />;
}

function FileChip({ path }: { path: string }) {
  return (
    <span
      className="min-w-0 shrink truncate rounded bg-edge/60 px-1.5 py-0.5 font-mono text-[11px] text-ink-dim"
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
  if (!summary) return null;
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
  const isFileTool = toolFilePath(item.name, item.input) !== null;
  const isSkill = item.name === "Skill";
  const preview =
    result && !((isFileTool || isSkill) && !result.isError)
      ? resultPreview(result.content, result.isError)
      : null;

  return (
    <div className="text-xs" data-tooluse-id={item.toolUseId}>
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
        <ToolStatusDot result={result} busy={busy} />
        <WrenchIcon className="shrink-0 text-ink-dim" />
        <span className="shrink-0 font-mono font-medium text-ink">{item.name}</span>
        <ToolArg name={item.name} input={item.input} />
        <StepChevron open={open} reveal="hover" size={11} />
      </div>

      {!open && preview && (
        <div className="flex gap-1.5 pl-[1.45rem] text-ink-dim">
          <span aria-hidden="true" className="select-none text-ink-dim/50">
            ⎿
          </span>
          <span
            className={`truncate font-mono text-[11px] ${result?.isError ? "text-bad/90" : ""}`}
          >
            {preview}
          </span>
        </div>
      )}

      {open && (
        <div className="mt-1 space-y-2 border-l border-edge pl-3">
          {item.input && item.input !== "{}" && (
            <pre className="selectable max-h-48 overflow-auto rounded bg-surface/60 p-2 font-mono text-[11px] leading-relaxed text-ink-dim">
              {prettyToolInput(item.input)}
            </pre>
          )}
          {item.skillBody && (
            <div className="selectable">
              <div className="mb-1 text-[10px] font-medium uppercase tracking-wide text-ink-dim/70">
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

const PermissionCard = memo(function PermissionCard({
  item,
  handlers,
}: {
  item: Extract<TranscriptItem, { kind: "permission" }>;
  handlers: PermissionHandlers;
}) {
  const filePath = toolFilePath(item.toolName, item.input);
  const summary = toolSummary(item.toolName, item.input);

  if (item.decision !== null) {
    if (item.decision === "allow") return null;
    return (
      <div className="flex items-center gap-2 text-xs text-ink-dim">
        <ShieldIcon size={12} />
        <span className="font-mono text-ink">{item.toolName}</span>
        <span>· denied</span>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-warn/40 bg-warn/10 p-3">
      <div className="flex items-center gap-2 text-warn">
        <ShieldIcon />
        <span className="text-sm font-medium">Permission required</span>
      </div>
      <div className="mt-2 flex items-center gap-1.5 rounded-md border border-warn/20 bg-surface/40 px-2.5 py-2">
        <span className="shrink-0 font-mono text-xs font-medium text-ink">{item.toolName}</span>
        {filePath ? (
          <FileChip path={filePath} />
        ) : (
          summary && (
            <span className="min-w-0 break-all font-mono text-xs text-ink-dim">{summary}</span>
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
      header={<span className="text-xs text-ink-dim">⚠ {detail}</span>}
    >
      <pre className="selectable max-h-48 overflow-auto px-3 py-2 font-mono text-[11px] text-ink-dim">
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
    <div className="text-[11px] text-ink-dim/70">
      <div className="flex items-center gap-2 uppercase tracking-wide">
        <span className="h-px flex-1 bg-edge" aria-hidden />
        {summary ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            className="flex shrink-0 items-center gap-1 rounded px-1 py-0.5 uppercase tracking-wide transition hover:bg-panel-2 hover:text-ink"
            title="claude summarized the prior turns; expand to read the summary."
          >
            <span>
              {label}
              {tokens}
            </span>
            <ChevronRightIcon
              size={11}
              className={`shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
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
        <div className="selectable mt-2 max-h-96 overflow-auto rounded-md border border-edge bg-panel-2 p-3 normal-case tracking-normal text-ink-dim">
          <MarkdownPreview body={summary} />
        </div>
      )}
    </div>
  );
});

const QueuedCommandRow = memo(function QueuedCommandRow({
  item,
}: {
  item: Extract<TranscriptItem, { kind: "queuedCommand" }>;
}) {
  return (
    <div className="text-xs">
      <div className={STEP_ROW_STATIC}>
        <span className="h-2 w-2 shrink-0 rounded-full bg-accent" aria-hidden="true" />
        <RestartIcon size={13} className="shrink-0 text-ink-dim" />
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
      className={`rounded-md border px-3 py-2 ${
        bad ? "border-bad/30 bg-bad/10" : "border-edge bg-panel-2"
      }`}
    >
      <p className={`text-center text-xs font-medium ${bad ? "text-bad" : "text-ink-dim"}`}>
        {exitLabel(item.outcome)}
        {item.code !== null && item.code !== 0 ? ` (exit ${item.code})` : ""}
      </p>
      {remedy && <p className="mt-1 text-center text-xs text-ink-dim">{remedy}</p>}
      {bad && !remedy && item.stderrTail && (
        <pre className="selectable mt-2 max-h-40 overflow-auto rounded bg-surface/60 p-2 font-mono text-[11px] text-ink-dim">
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
    <div className="mt-3 flex items-center gap-1.5 font-mono text-xs tabular-nums text-ink-dim">
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

const isFileUnit = (it: TranscriptItem): boolean =>
  it.kind === "tool" && toolFilePath(it.name, it.input) !== null;

const parentIdOf = (it: TranscriptItem): string | null =>
  it.kind === "tool" || it.kind === "assistant" || it.kind === "thinking" || it.kind === "skill"
    ? (it.parentId ?? null)
    : null;

function toUnits(items: TranscriptItem[], hasChildren: (it: TranscriptItem) => boolean): Unit[] {
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
      !(item.kind === "tool" && hasChildren(item)) &&
      !isFileUnit(item);
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
  const tools = items.filter((i) => i.kind === "tool");
  const running = busy && tools.some((t) => t.kind === "tool" && t.result === null);
  const label =
    tools.length > 0
      ? `${tools.length} tool call${tools.length === 1 ? "" : "s"}`
      : `${items.length} steps`;
  return (
    <div className="text-xs">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`${STEP_ROW} text-ink-dim`}
      >
        {running ? <Spinner size={12} /> : <WrenchIcon className="shrink-0 text-ink-dim" />}
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
) {
  switch (item.kind) {
    case "user":
      return (
        <UserMessage
          id={item.id}
          text={item.text}
          stage={item.stage}
          attachments={item.attachments}
          createdAt={item.createdAt}
          promptName={item.promptName}
          prompt={prompt}
        />
      );
    case "assistant":
      return <AssistantMessage text={item.text} streaming={item.streaming} />;
    case "thinking":
      return <ThinkingBlock text={item.text} />;
    case "skill":
      return <SkillBlock name={item.name} body={item.body} />;
    case "tool":
      return <ToolBlock item={item} busy={busy} />;
    case "permission":
      return <PermissionCard item={item} handlers={permission} />;
    case "error":
      return <ProtocolErrorBlock detail={item.detail} raw={item.raw} />;
    case "compact":
      return <CompactDivider item={item} />;
    case "queuedCommand":
      return <QueuedCommandRow item={item} />;
    case "exit":
      return <ExitNotice item={item} />;
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
  const summary = toolSummary(item.name, item.input);
  const toolCount = childItems.filter((c) => c.kind === "tool").length;
  const result = item.result;
  const running = busy && result === null;
  return (
    <div className="text-xs">
      <button type="button" onClick={() => setOpen((v) => !v)} className={STEP_ROW}>
        <ToolStatusDot result={result} busy={busy} />
        <WrenchIcon className="shrink-0 text-ink-dim" />
        <span className="shrink-0 font-mono font-medium text-ink">
          {item.name === "Workflow" ? "Workflow" : "Subagent"}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={summary}>
          {summary}
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
              <div className="mb-1 text-[10px] font-medium uppercase tracking-wide text-ink-dim/70">
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

function ItemList({
  items,
  childrenByParent,
  busy,
  permission,
  prompt,
}: {
  items: TranscriptItem[];
  childrenByParent: ChildMap;
  busy: boolean;
  permission: PermissionHandlers;
  prompt: PromptHandlers;
}) {
  const hasChildren = (it: TranscriptItem) =>
    it.kind === "tool" && childrenByParent.has(it.toolUseId);
  const visible = items.filter((it) => it.kind !== "permission" || it.decision !== "allow");
  const units = toUnits(visible, hasChildren);
  return (
    <>
      {units.map((unit, i) => {
        const gap = gapForKinds(i > 0 ? unitKind(units[i - 1]) : undefined, unitKind(unit));
        if (unit.type === "group") {
          return (
            <div key={`g:${unit.id}`} className={gap}>
              <ToolGroup items={unit.items} busy={busy} />
            </div>
          );
        }
        const item = unit.item;
        if (item.kind === "tool" && childrenByParent.has(item.toolUseId)) {
          return (
            <div key={item.id} className={gap}>
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
          <div key={item.id} className={gap}>
            {renderSingle(item, busy, permission, prompt)}
          </div>
        );
      })}
    </>
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
};

export type TranscriptHandle = {
  scrollToTool: (toolUseId: string) => void;
  scrollToBottom: () => void;
};

const RESTICK_PX = 24;

const RELOCK_MS = 200;

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
    { items, busy, connecting, harnessLabel, workingSince, permission, prompt },
    ref,
  ) {
    const scrollRef = useRef<HTMLDivElement>(null);
    const contentRef = useRef<HTMLDivElement>(null);
    const atBottom = useRef(true);
    const lastTop = useRef(0);
    const relockUntil = useRef(0);
    const easeFrame = useRef(0);

    const pin = useCallback((factor = 0.3) => {
      const el = scrollRef.current;
      if (!el || !atBottom.current) return;
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
        const container = scrollRef.current;
        if (!container) return;
        const el = container.querySelector(`[data-tooluse-id="${toolUseId}"]`);
        if (el) el.scrollIntoView({ behavior: "smooth", block: "nearest" });
      },
      scrollToBottom() {
        atBottom.current = true;
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
      const gap = el.scrollHeight - el.clientHeight - top;
      const wasBottom = atBottom.current;
      atBottom.current = stickAfterScroll(lastTop.current, top, gap, wasBottom);
      if (!wasBottom && atBottom.current) relockUntil.current = performance.now() + RELOCK_MS;
      lastTop.current = top;
    };

    const onWheel = (e: WheelEvent<HTMLDivElement>) => {
      const el = scrollRef.current;
      if (!el || !releaseOnWheel(e.deltaX, e.deltaY, e.ctrlKey, el.scrollTop)) return;
      if (insideNestedScroller(e.target, el)) return;
      atBottom.current = false;
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
      const ro = new ResizeObserver(() => pin());
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

    return (
      <div
        ref={scrollRef}
        onScroll={onScroll}
        onWheel={onWheel}
        className="-mt-9 min-h-0 flex-1 overflow-y-auto overscroll-contain p-5 pt-14 [overflow-anchor:none]"
      >
        <div ref={contentRef} className="mx-auto w-[90%]">
          <ItemList
            items={topLevel}
            childrenByParent={childrenByParent}
            busy={busy}
            permission={permission}
            prompt={prompt}
          />
          {connecting && (
            <div className="mt-3 flex items-center gap-2 text-sm text-ink-dim">
              <Spinner size={14} />
              Connecting to {harnessLabel}…
            </div>
          )}
          {workingSince !== null && <ElapsedCounter since={workingSince} />}
        </div>
      </div>
    );
  }),
);
