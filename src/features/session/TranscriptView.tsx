import { memo, useEffect, useRef, useState } from "react";

import { Button, Spinner } from "../../components/ui";
import { ChevronRightIcon, ShieldIcon, WrenchIcon } from "../../components/icons";
import { MarkdownPreview } from "../library/MarkdownPreview";
import { stageLabel } from "./stages";
import { exitLabel, type TranscriptItem } from "./transcript";
import { formatElapsed, prettyToolInput, resultPreview, toolSummary } from "./toolDisplay";

export type PermissionHandlers = {
  onAllow: (requestId: string) => void;
  onAllowForSession: (requestId: string, toolName: string) => void;
  onDeny: (requestId: string) => void;
};

/* -------------------------------- pieces --------------------------------- */

function UserMessage({ text, stage }: { text: string; stage: string | null }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] rounded-lg border border-edge bg-panel-2 px-3.5 py-2.5">
        {stage && stage !== "general" && (
          <span className="mb-1 block text-[11px] font-medium uppercase tracking-wide text-ink-dim">
            {stageLabel(stage)}
          </span>
        )}
        <p className="selectable whitespace-pre-wrap break-words text-sm text-ink">{text}</p>
      </div>
    </div>
  );
}

/**
 * Smoothly reveal text as it streams in. claude sends `partialText` in bursts;
 * this drains the backlog at a steady per-frame rate (ease-out, so it catches
 * up when far behind) so the user sees fluid typing instead of chunks popping
 * in. Snaps to the full text the moment the block finishes streaming.
 */
function useSmoothText(text: string, streaming: boolean): string {
  const [revealed, setRevealed] = useState(() => (streaming ? 0 : text.length));
  const targetLen = useRef(text.length);
  targetLen.current = text.length;

  useEffect(() => {
    if (!streaming) {
      setRevealed(targetLen.current);
      return;
    }
    let frame = requestAnimationFrame(function tick() {
      setRevealed((cur) => {
        const target = targetLen.current;
        if (cur >= target) return cur; // caught up; idle until more arrives
        return Math.min(target, cur + Math.max(2, Math.ceil((target - cur) / 6)));
      });
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [streaming]);

  return text.slice(0, Math.min(revealed, text.length));
}

const StreamCursor = () => (
  <span className="ml-0.5 inline-block h-3.5 w-1.5 translate-y-0.5 animate-pulse rounded-sm bg-ink-dim align-baseline" />
);

function AssistantMessage({ text, streaming }: { text: string; streaming: boolean }) {
  const shown = useSmoothText(text, streaming);

  // While streaming, render plain text (cheap, and avoids markdown structures
  // "snapping" as their closing tokens arrive); format on completion.
  if (streaming) {
    return (
      <div className="selectable text-sm leading-relaxed text-ink">
        {shown ? (
          <span className="whitespace-pre-wrap break-words">{shown}</span>
        ) : (
          <span className="text-ink-dim">…</span>
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
}

/** The shared button shell for a transcript working-step row (thinking, tool,
 *  tool group, subagent) — a tight, full-bleed hover target. */
const STEP_ROW =
  "group -ml-1.5 flex w-[calc(100%+0.375rem)] items-center gap-2 rounded px-1.5 py-1 text-left hover:bg-panel-2";

/** The disclosure chevron for a step row. `reveal="hover"` keeps it hidden until
 *  the row is hovered or open (thinking/tool); `"always"` shows it (group/subagent). */
function StepChevron({
  open,
  reveal = "always",
  size = 12,
}: {
  open: boolean;
  reveal?: "hover" | "always";
  size?: number;
}) {
  const visibility =
    reveal === "hover"
      ? open
        ? "rotate-90 text-ink-dim"
        : "text-ink-dim/0 group-hover:text-ink-dim"
      : `${open ? "rotate-90" : ""} text-ink-dim`;
  return (
    <ChevronRightIcon size={size} className={`ml-auto shrink-0 transition-all ${visibility}`} />
  );
}

/** The expanded `<pre>` for a tool/subagent result — shared so error coloring,
 *  max-height, and the empty-output fallback stay identical across both. */
function ToolResultOutput({ result }: { result: { content: string; isError: boolean } }) {
  return (
    <pre
      className={`selectable max-h-96 overflow-auto rounded p-2 font-mono text-[11px] leading-relaxed ${
        result.isError ? "bg-bad/10 text-rose-300" : "bg-surface/60 text-ink-dim"
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

function ThinkingBlock({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  // A one-line preview of the thought (Claude-Code / Conductor style), so a
  // collapsed thinking block still hints at what the agent reasoned about.
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
}

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

/**
 * One tool call as a lightweight action row (not a boxed card) — the thing that
 * keeps the transcript reading top-to-bottom like Claude Code: a compact
 * `● Name  summary` line with a dimmed `⎿ result` line beneath it. Click to
 * expand the full input/output.
 */
function ToolBlock({
  item,
  busy,
}: {
  item: Extract<TranscriptItem, { kind: "tool" }>;
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const summary = toolSummary(item.name, item.input);
  const result = item.result;
  const preview = result ? resultPreview(result.content, result.isError) : null;

  return (
    <div className="text-xs">
      <button type="button" onClick={() => setOpen((v) => !v)} className={STEP_ROW}>
        <ToolStatusDot result={result} busy={busy} />
        <WrenchIcon className="shrink-0 text-ink-dim" />
        <span className="shrink-0 font-mono font-medium text-ink">{item.name}</span>
        {summary && (
          <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={summary}>
            {summary}
          </span>
        )}
        <StepChevron open={open} reveal="hover" size={11} />
      </button>

      {!open && preview && (
        <div className="flex gap-1.5 pl-[1.45rem] text-ink-dim">
          <span aria-hidden="true" className="select-none text-ink-dim/50">
            ⎿
          </span>
          <span
            className={`truncate font-mono text-[11px] ${result?.isError ? "text-rose-300/90" : ""}`}
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
          {result && <ToolResultOutput result={result} />}
        </div>
      )}
    </div>
  );
}

function PermissionCard({
  item,
  handlers,
}: {
  item: Extract<TranscriptItem, { kind: "permission" }>;
  handlers: PermissionHandlers;
}) {
  const summary = toolSummary(item.toolName, item.input);

  if (item.decision !== null) {
    return (
      <div className="flex items-center gap-2 text-xs text-ink-dim">
        <ShieldIcon size={12} />
        <span className="font-mono text-ink">{item.toolName}</span>
        <span>{item.decision === "allow" ? "— allowed" : "— denied"}</span>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-amber-400/40 bg-amber-400/10 p-3">
      <div className="flex items-center gap-2 text-amber-200">
        <ShieldIcon />
        <span className="text-sm font-medium">Permission required</span>
      </div>
      <div className="mt-2 rounded-md border border-amber-400/20 bg-surface/40 px-2.5 py-2">
        <span className="font-mono text-xs font-medium text-ink">{item.toolName}</span>
        {summary && (
          <span className="ml-1.5 break-all font-mono text-xs text-ink-dim">{summary}</span>
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
}

function ProtocolErrorBlock({ detail, raw }: { detail: string; raw: string }) {
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
}

/** A crash/spawn-failure stderr that mentions login → an actionable remedy. */
function exitRemedy(stderrTail: string): string | null {
  if (/\/login\b|not logged in|unauthor/i.test(stderrTail)) {
    return "claude isn't logged in. Run `claude /login` in a terminal, then start a new session.";
  }
  return null;
}

function ExitNotice({ item }: { item: Extract<TranscriptItem, { kind: "exit" }> }) {
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
}

/**
 * The live "time working" readout pinned to the bottom-left of the transcript
 * (Conductor-style) — it sits at the end of the scrolling content, so it rides
 * the bottom of the chat. Ticks ~10×/s so the tenths advance smoothly; it
 * unmounts the moment the turn ends (`since` goes null), which clears the timer.
 */
function ElapsedCounter({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 100);
    return () => clearInterval(id);
  }, [since]);
  return (
    <div className="mt-3 font-mono text-xs tabular-nums text-ink-dim">{formatElapsed(now - since)}</div>
  );
}

/* ------------------------------- transcript ------------------------------ */

type ItemKind = TranscriptItem["kind"];

/** "Quiet" items are the agent's working steps; they cluster/collapse together. */
const isQuiet = (kind: ItemKind) => kind === "tool" || kind === "thinking";

/**
 * Vertical gap before a unit, by what it is and what precedes it — the rhythm
 * that makes the stream read linearly: working steps pack tightly, while prose
 * and new turns get room to breathe.
 */
function gapForKinds(prev: ItemKind | undefined, cur: ItemKind): string {
  if (!prev) return "";
  if (cur === "user") return "mt-5"; // a new turn
  if (cur === "assistant") return "mt-4"; // a new block of prose
  if (isQuiet(cur)) return isQuiet(prev) ? "mt-1" : "mt-2";
  return "mt-3"; // permission / exit / error
}

/** A render unit: a standalone item, or a run of ≥2 working steps to collapse. */
type Unit =
  | { type: "single"; item: TranscriptItem }
  | { type: "group"; id: string; items: TranscriptItem[] };

const unitKind = (u: Unit): ItemKind => (u.type === "group" ? "tool" : u.item.kind);

/** The subagent `Task` id an item belongs to (only assistant/thinking/tool carry it). */
const parentIdOf = (it: TranscriptItem): string | null =>
  it.kind === "tool" || it.kind === "assistant" || it.kind === "thinking"
    ? (it.parentId ?? null)
    : null;

/** Fold a list into units: consecutive tool/thinking steps (≥2) become one
 *  collapsible group; everything else stays inline. A tool with subagent
 *  children stays standalone (`hasChildren`) so it renders as its own block. */
function toUnits(items: TranscriptItem[], hasChildren: (it: TranscriptItem) => boolean): Unit[] {
  const units: Unit[] = [];
  let run: TranscriptItem[] = [];
  const flush = () => {
    if (run.length >= 2) units.push({ type: "group", id: run[0].id, items: run });
    else if (run.length === 1) units.push({ type: "single", item: run[0] });
    run = [];
  };
  for (const item of items) {
    const groupable = isQuiet(item.kind) && !(item.kind === "tool" && hasChildren(item));
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

/**
 * A collapsed run of working steps — "N tool calls", expand for the detail.
 * Collapsed by default so a long research burst reads as one line; while the
 * turn is live the header shows a spinner and the count grows.
 */
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

/** Render one standalone item (no grouping / nesting). */
function renderSingle(item: TranscriptItem, busy: boolean, permission: PermissionHandlers) {
  switch (item.kind) {
    case "user":
      return <UserMessage text={item.text} stage={item.stage} />;
    case "assistant":
      return <AssistantMessage text={item.text} streaming={item.streaming} />;
    case "thinking":
      return <ThinkingBlock text={item.text} />;
    case "tool":
      return <ToolBlock item={item} busy={busy} />;
    case "permission":
      return <PermissionCard item={item} handlers={permission} />;
    case "error":
      return <ProtocolErrorBlock detail={item.detail} raw={item.raw} />;
    case "exit":
      return <ExitNotice item={item} />;
    default:
      return null;
  }
}

type ChildMap = Map<string, TranscriptItem[]>;

/**
 * A subagent (`Task`) and all of its work, collapsed by default — expand to see
 * the nested transcript. Keeps subagent activity out of the main thread.
 */
function SubagentBlock({
  item,
  childItems,
  childrenByParent,
  busy,
  permission,
}: {
  item: Extract<TranscriptItem, { kind: "tool" }>;
  childItems: TranscriptItem[];
  childrenByParent: ChildMap;
  busy: boolean;
  permission: PermissionHandlers;
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
        <span className="shrink-0 font-mono font-medium text-ink">Subagent</span>
        {summary && (
          <span className="min-w-0 flex-1 truncate font-mono text-ink-dim" title={summary}>
            {summary}
          </span>
        )}
        {toolCount > 0 && (
          <span className="ml-auto shrink-0 text-ink-dim">
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

/** Render a list of items: groups working steps, nests subagents, spaces units. */
function ItemList({
  items,
  childrenByParent,
  busy,
  permission,
}: {
  items: TranscriptItem[];
  childrenByParent: ChildMap;
  busy: boolean;
  permission: PermissionHandlers;
}) {
  const hasChildren = (it: TranscriptItem) =>
    it.kind === "tool" && childrenByParent.has(it.toolUseId);
  const units = toUnits(items, hasChildren);
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
              />
            </div>
          );
        }
        return (
          <div key={item.id} className={gap}>
            {renderSingle(item, busy, permission)}
          </div>
        );
      })}
    </>
  );
}

type Props = {
  items: TranscriptItem[];
  /** A turn is in flight — drives the running-tool spinner. */
  busy: boolean;
  /** True before the first event lands, to show a connecting spinner. */
  connecting: boolean;
  /** Epoch ms when the current working stretch began, or null when idle —
   *  drives the bottom-left elapsed counter. */
  workingSince: number | null;
  permission: PermissionHandlers;
};

export const TranscriptView = memo(function TranscriptView({
  items,
  busy,
  connecting,
  workingSince,
  permission,
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };

  // Stick to the bottom on discrete updates (new items, tool results).
  useEffect(() => {
    const el = scrollRef.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [items]);

  // While a turn is in flight the visible text grows smoothly between store
  // updates (the typewriter reveal), so pin to the bottom every frame too —
  // unless the user has scrolled up.
  useEffect(() => {
    if (!busy) return;
    let frame = requestAnimationFrame(function tick() {
      const el = scrollRef.current;
      if (el && atBottom.current) el.scrollTop = el.scrollHeight;
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [busy]);

  // Subagent items nest under their `Task`; the main thread shows the rest.
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
    <div ref={scrollRef} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto p-5">
      <div className="mx-auto w-4/5">
        <ItemList
          items={topLevel}
          childrenByParent={childrenByParent}
          busy={busy}
          permission={permission}
        />
        {connecting && (
          <div className="mt-3 flex items-center gap-2 text-sm text-ink-dim">
            <Spinner size={14} />
            Connecting to claude…
          </div>
        )}
        {workingSince !== null && <ElapsedCounter since={workingSince} />}
      </div>
    </div>
  );
});
