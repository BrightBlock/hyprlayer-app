import { memo, useEffect, useRef, useState } from "react";

import { Button, Spinner } from "../../components/ui";
import { ChevronRightIcon, ShieldIcon, WrenchIcon } from "../../components/icons";
import { MarkdownPreview } from "../library/MarkdownPreview";
import { stageLabel } from "./stages";
import { exitLabel, type TranscriptItem } from "./transcript";
import { prettyToolInput, toolSummary } from "./toolDisplay";

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
  return (
    <Collapsible
      open={open}
      onToggle={() => setOpen((v) => !v)}
      header={<span className="text-xs italic text-ink-dim">Thought for a moment</span>}
    >
      <div className="selectable whitespace-pre-wrap px-3 py-2 text-xs leading-relaxed text-ink-dim">
        {text}
      </div>
    </Collapsible>
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

  return (
    <div className="overflow-hidden rounded-md border border-edge bg-panel-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-edge/40"
      >
        <ToolStatusDot result={result} busy={busy} />
        <WrenchIcon className="shrink-0 text-ink-dim" />
        <span className="shrink-0 font-mono text-xs font-medium text-ink">{item.name}</span>
        {summary && (
          <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-dim" title={summary}>
            {summary}
          </span>
        )}
        <ChevronRightIcon
          size={12}
          className={`ml-auto shrink-0 text-ink-dim transition-transform ${open ? "rotate-90" : ""}`}
        />
      </button>
      {!open && result && (
        <div className="truncate border-t border-edge px-3 py-1.5 font-mono text-[11px] text-ink-dim">
          {result.content.split("\n")[0] || (result.isError ? "(error)" : "(no output)")}
        </div>
      )}
      {open && (
        <div className="space-y-2 border-t border-edge p-2">
          {item.input && item.input !== "{}" && (
            <pre className="selectable max-h-48 overflow-auto rounded bg-surface/60 p-2 font-mono text-[11px] leading-relaxed text-ink-dim">
              {prettyToolInput(item.input)}
            </pre>
          )}
          {result && (
            <pre
              className={`selectable max-h-96 overflow-auto rounded p-2 font-mono text-[11px] leading-relaxed ${
                result.isError ? "bg-bad/10 text-rose-300" : "bg-surface/60 text-ink-dim"
              }`}
            >
              {result.content || (result.isError ? "(error, no output)" : "(no output)")}
            </pre>
          )}
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

/* ------------------------------- transcript ------------------------------ */

type Props = {
  items: TranscriptItem[];
  /** A turn is in flight — drives the running-tool spinner. */
  busy: boolean;
  /** True before the first event lands, to show a connecting spinner. */
  connecting: boolean;
  permission: PermissionHandlers;
};

export const TranscriptView = memo(function TranscriptView({
  items,
  busy,
  connecting,
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

  return (
    <div ref={scrollRef} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto p-5">
      <div className="mx-auto max-w-3xl space-y-4">
        {items.map((item) => {
          switch (item.kind) {
            case "user":
              return <UserMessage key={item.id} text={item.text} stage={item.stage} />;
            case "assistant":
              return <AssistantMessage key={item.id} text={item.text} streaming={item.streaming} />;
            case "thinking":
              return <ThinkingBlock key={item.id} text={item.text} />;
            case "tool":
              return <ToolBlock key={item.id} item={item} busy={busy} />;
            case "permission":
              return <PermissionCard key={item.id} item={item} handlers={permission} />;
            case "error":
              return <ProtocolErrorBlock key={item.id} detail={item.detail} raw={item.raw} />;
            case "exit":
              return <ExitNotice key={item.id} item={item} />;
            default:
              return null;
          }
        })}
        {connecting && (
          <div className="flex items-center gap-2 text-sm text-ink-dim">
            <Spinner size={14} />
            Connecting to claude…
          </div>
        )}
      </div>
    </div>
  );
});
