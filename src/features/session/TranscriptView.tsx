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

import { Button, Spinner } from "../../components/ui";
import {
  ChevronRightIcon,
  ClipboardIcon,
  RewindIcon,
  ShieldIcon,
  SparkleIcon,
  WrenchIcon,
} from "../../components/icons";
import { MarkdownPreview } from "../library/MarkdownPreview";
import { stageLabel } from "./stages";
import { exitLabel, type TranscriptItem } from "./transcript";
import { AttachmentCards } from "./AttachmentCard";
import { FileCard } from "./FileCard";
import type { Attachment } from "./attachments";
import {
  baseName,
  editDiffFromTool,
  formatElapsed,
  formatRelativeTime,
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

/** Actions reachable from the per-user-message toolbar (copy/rewind).
 *  Plumbed alongside `PermissionHandlers` so the rendering stays prop-driven
 *  (no zustand access from the view) and tests can drive the buttons. Rewind
 *  carries the bubble's text so the host can repopulate the composer with it
 *  (Claude-Desktop-style: rewinding lands you back on that prompt to edit and
 *  re-send). */
export type PromptHandlers = {
  onRewind: (itemId: string, text: string) => void;
};

/* -------------------------------- pieces --------------------------------- */

/** A short-lived "Copied!" confirmation under the copy button (Claude-Code
 *  style). Re-renders the toolbar on tick to fade the label after 1.2s. */
function useCopyFeedback() {
  const [copied, setCopied] = useState(false);
  const trigger = (text: string) => {
    if (!text) return;
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    });
  };
  return { copied, trigger };
}

/**
 * Re-render a relative-time label on a schedule that matches its precision:
 * every 10s under a minute, every 30s under an hour, every 5m thereafter.
 * Avoids waking the React tree on a 1s tick when the label says "2w ago".
 */
function useTickingNow(epochMs: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (epochMs === null) return;
    // A self-rescheduling timeout: each run picks the next delay from the
    // message's current age, so the cadence widens as it ages past each
    // boundary (a fresh "5s ago" runs at 10s, a "2w ago" at 5m).
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

/** Hover-revealed actions for a user turn — copy text, rewind to here, fork,
 *  and a relative timestamp — mirroring Claude Desktop's prompt toolbar. */
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
  prompt,
}: {
  id: string;
  text: string;
  stage: string | null;
  attachments: Attachment[];
  createdAt: number | null;
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
        {text && (
          <p className="selectable whitespace-pre-wrap break-words text-sm text-ink">{text}</p>
        )}
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
    // Throttle the reveal (and thus the markdown re-parse it drives) to ~30fps:
    // the rAF still ticks each frame, but we only advance the revealed length
    // every ~33ms. The CSS-animated cursor keeps motion fluid between steps, so
    // the cost of re-parsing the growing block during a live turn roughly halves.
    let last = 0;
    let frame = requestAnimationFrame(function tick(ts) {
      if (ts - last >= 33) {
        last = ts;
        setRevealed((cur) => {
          const target = targetLen.current;
          if (cur >= target) return cur; // caught up; idle until more arrives
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

  // Render markdown live as the text streams in (react-markdown handles
  // partial/unterminated structures gracefully), so the user sees formatted
  // output instead of raw markdown until the turn completes.
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

const ThinkingBlock = memo(function ThinkingBlock({ text }: { text: string }) {
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
});

/** A launched skill, collapsed to a one-line chip — Claude Code injects the
 *  skill's full SKILL.md as an assistant block; we keep it out of the thread but
 *  expandable so the loaded instructions are still inspectable. */
const SkillBlock = memo(function SkillBlock({ name, body }: { name: string; body: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="text-xs">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`${STEP_ROW} text-ink-dim`}
      >
        <SparkleIcon size={12} className="shrink-0 text-ink-dim" />
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

/** A compact filename chip for file-targeting tools (Read/Write/Edit/…): shows
 *  just the basename, with the full absolute path on hover. */
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

/** The one-line argument shown after a tool's name: a filename chip for file
 *  tools, otherwise the plain Claude-Code-style summary text. */
function ToolArg({ name, input }: { name: string; input: string }) {
  const filePath = toolFilePath(name, input);
  if (filePath) {
    // Every file tool gets the same pin card; Edit/MultiEdit also carry a diff
    // the card previews on hover (other file tools pass diff=null → no popover).
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

/**
 * One tool call as a lightweight action row (not a boxed card) — the thing that
 * keeps the transcript reading top-to-bottom like Claude Code: a compact
 * `● Name  summary` line with a dimmed `⎿ result` line beneath it. Click to
 * expand the full input/output.
 */
const ToolBlock = memo(function ToolBlock({
  item,
  busy,
}: {
  item: Extract<TranscriptItem, { kind: "tool" }>;
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const result = item.result;
  // Edit/MultiEdit already show the change in the card (basename + hover diff),
  // so their "file … updated successfully" confirmation is redundant noise — and
  // it re-leaks the full path. Suppress that preview on success; keep it on error.
  const isEdit = item.name === "Edit" || item.name === "MultiEdit";
  // A Skill row already names the skill in its header, so the "Launching skill: X"
  // result is redundant noise — and its body folds into the expanded view below.
  const isSkill = item.name === "Skill";
  const preview =
    result && !((isEdit || isSkill) && !result.isError)
      ? resultPreview(result.content, result.isError)
      : null;

  return (
    <div className="text-xs" data-tooluse-id={item.toolUseId}>
      <button type="button" onClick={() => setOpen((v) => !v)} className={STEP_ROW}>
        <ToolStatusDot result={result} busy={busy} />
        <WrenchIcon className="shrink-0 text-ink-dim" />
        <span className="shrink-0 font-mono font-medium text-ink">{item.name}</span>
        <ToolArg name={item.name} input={item.input} />
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
    // An approved permission is immediately followed by the actual tool call row,
    // so re-showing "Tool — allowed" is redundant. Only a denial (where the tool
    // never runs, so there's no tool row) is worth surfacing.
    if (item.decision === "allow") return null;
    return (
      <div className="flex items-center gap-2 text-xs text-ink-dim">
        <ShieldIcon size={12} />
        <span className="font-mono text-ink">{item.toolName}</span>
        <span>— denied</span>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-amber-400/40 bg-amber-400/10 p-3">
      <div className="flex items-center gap-2 text-amber-200">
        <ShieldIcon />
        <span className="text-sm font-medium">Permission required</span>
      </div>
      <div className="mt-2 flex items-center gap-1.5 rounded-md border border-amber-400/20 bg-surface/40 px-2.5 py-2">
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

/** A crash/spawn-failure stderr that mentions login → an actionable remedy. */
function exitRemedy(stderrTail: string): string | null {
  if (/\/login\b|not logged in|unauthor/i.test(stderrTail)) {
    return "claude isn't logged in. Run `claude /login` in a terminal, then start a new session.";
  }
  return null;
}

/** A thin horizontal divider marking where claude paused to compact the
 *  conversation — auto- or manually. Without this row a long compact looks like
 *  the session went silent (no events fire while the summary is generated). When
 *  claude's post-compact summary is folded in, the label becomes a toggle that
 *  expands it — kept collapsed by default so the 10–20KB summary never floods the
 *  thread (the same treatment a launched skill's injected body gets). */
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

/** A file-targeting step (Read/Write/Edit/…) renders a per-file chip or card, so
 *  it never folds into a collapsed "N tool calls" group — every file the agent
 *  touches stays its own visible row. */
const isFileUnit = (it: TranscriptItem): boolean =>
  it.kind === "tool" && toolFilePath(it.name, it.input) !== null;

/** The subagent `Task` id an item belongs to (only assistant/thinking/tool/skill carry it). */
const parentIdOf = (it: TranscriptItem): string | null =>
  it.kind === "tool" || it.kind === "assistant" || it.kind === "thinking" || it.kind === "skill"
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

/** Render a list of items: groups working steps, nests subagents, spaces units. */
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
  // An approved permission renders nothing (the tool row that follows conveys it),
  // so drop it before layout — otherwise its empty wrapper leaves a phantom gap.
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
  /** claude is actively computing (`isWorking`) — drives the running-tool
   *  spinner and the smooth scroll-to-bottom. False while parked on the user. */
  busy: boolean;
  /** True before the first event lands, to show a connecting spinner. */
  connecting: boolean;
  /** Epoch ms when the current working stretch began, or null when idle —
   *  drives the bottom-left elapsed counter. */
  workingSince: number | null;
  permission: PermissionHandlers;
  prompt: PromptHandlers;
};

/** Imperative handle exposed via ref so the parent can scroll to a tool row. */
export type TranscriptHandle = {
  /** Scroll the transcript to the tool row with the given `toolUseId`. */
  scrollToTool: (toolUseId: string) => void;
};

/** How close to the bottom (px) counts as "stuck to the bottom" — the gate for
 *  *resuming* auto-follow. Forgiving on purpose: while streaming, the bottom
 *  keeps growing, so a too-tight gate means scrolling "to the bottom" lands a few
 *  px short and never re-locks. The matching re-lock is eased (see RELOCK_MS), so
 *  a generous gate doesn't reintroduce the snap a wide threshold used to cause. */
const RESTICK_PX = 24;

/** When the view re-sticks from a gap, ease the catch-up to the exact bottom
 *  shut over roughly this long instead of jumping — keeps the re-lock smooth. */
const RELOCK_MS = 200;

export const TranscriptView = memo(
  forwardRef<TranscriptHandle, Props>(function TranscriptView(
    { items, busy, connecting, workingSince, permission, prompt },
    ref,
  ) {
    const scrollRef = useRef<HTMLDivElement>(null);
    const atBottom = useRef(true);
    // Previous scrollTop + scrollHeight, to read scroll *direction* and to tell a
    // real user scroll-up apart from a content-shrink clamp (both lower scrollTop,
    // but only the former should release the stick).
    const lastTop = useRef(0);
    const lastHeight = useRef(0);
    // performance.now() until which a freshly re-stuck view eases the remaining
    // gap to the bottom instead of snapping (a smooth "catch up" on re-lock).
    const relockUntil = useRef(0);

    useImperativeHandle(ref, () => ({
      scrollToTool(toolUseId) {
        const container = scrollRef.current;
        if (!container) return;
        const el = container.querySelector(`[data-tooluse-id="${toolUseId}"]`);
        if (el) el.scrollIntoView({ behavior: "smooth", block: "nearest" });
      },
    }));

    // Keep the view at the bottom while stuck. A fresh re-lock (see `relockUntil`)
    // eases the gap shut over a few frames so it glides rather than jumps; steady
    // streaming pins hard so it tracks the live output tightly. No-ops the instant
    // the user scrolls up (atBottom cleared), so it never fights them.
    const pin = useCallback(() => {
      const el = scrollRef.current;
      if (!el || !atBottom.current) return;
      const target = el.scrollHeight - el.clientHeight;
      const gap = target - el.scrollTop;
      if (gap <= 0) return;
      if (gap > 2 && performance.now() < relockUntil.current) {
        el.scrollTop += Math.max(1, gap * 0.3);
      } else {
        el.scrollTop = target;
      }
    }, []);

    const onScroll = () => {
      const el = scrollRef.current;
      if (!el) return;
      const top = el.scrollTop;
      const height = el.scrollHeight;
      const wasBottom = atBottom.current;
      // A drop in scrollTop while the content height held is the user scrolling
      // up — release at once so streaming stops yanking them back. (A drop the
      // content *shrinking* forces — scrollTop clamps down on its own — is not a
      // gesture, so it must not release.) Otherwise, re-stick once they're back
      // within reach of the bottom.
      if (top < lastTop.current - 1 && height >= lastHeight.current - 1) {
        atBottom.current = false;
      } else if (height - top - el.clientHeight < RESTICK_PX) {
        atBottom.current = true;
      }
      // On the transition back to stuck, glide the catch-up to the exact bottom.
      if (!wasBottom && atBottom.current) relockUntil.current = performance.now() + RELOCK_MS;
      lastTop.current = top;
      lastHeight.current = height;
    };

    // A wheel/trackpad scroll-up releases synchronously, before the next frame's
    // pin can fire — the scroll event alone can land after that frame's rAF.
    const onWheel = (e: WheelEvent<HTMLDivElement>) => {
      if (e.deltaY < 0) atBottom.current = false;
    };

    // Discrete updates while idle (a queued message renders, a tool result lands):
    // snap to the bottom if stuck. While a turn streams the rAF below owns pinning.
    useEffect(() => {
      if (busy) return;
      const el = scrollRef.current;
      if (el && atBottom.current) el.scrollTop = el.scrollHeight;
    }, [items, busy]);

    // While a turn is in flight the visible text grows between store updates, so
    // keep the view pinned (and glide a re-lock) every frame.
    useEffect(() => {
      if (!busy) return;
      let frame = requestAnimationFrame(function tick() {
        pin();
        frame = requestAnimationFrame(tick);
      });
      return () => cancelAnimationFrame(frame);
    }, [busy, pin]);

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
      <div
        ref={scrollRef}
        onScroll={onScroll}
        onWheel={onWheel}
        // `overscroll-contain`: keep the wheel inside the transcript. Without it,
        // scrolling past the top/bottom edge chains to the app-level <main>
        // scroller and drags the whole window — the agent pane must own its
        // scroll. (Newly reachable now that scroll-up isn't pinned mid-stream.)
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-5"
      >
        <div className="mx-auto w-[90%]">
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
              Connecting to claude…
            </div>
          )}
          {workingSince !== null && <ElapsedCounter since={workingSince} />}
        </div>
      </div>
    );
  }),
);
