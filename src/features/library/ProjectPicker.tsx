import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";

import { Icon } from "@/components/Icon";
import { RepoAvatar } from "@/components/RepoAvatar";
import { cx, Kbd, Popover, rowClass, RowSpine } from "@/components/ui";
import { fuzzyScore } from "@/lib/fuzzy";
import { eventToChord } from "@/lib/keybindings/chord";
import { IS_MACOS } from "@/lib/platform";

export type ScopeProject = {
  name: string;
  label: string;
  count: number;
  originUrl: string | null;
};

// null is the "All projects" row.
type Option = ScopeProject | null;

export function ProjectPicker({
  open,
  onClose,
  anchorRef,
  projects,
  current,
  pinned,
  onScope,
  onTogglePin,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  projects: ScopeProject[];
  current: string | undefined;
  pinned: readonly string[];
  onScope: (name: string | undefined) => void;
  onTogglePin: (name: string) => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const activeRef = useRef<HTMLButtonElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
    }
  }, [open]);

  const matches = useMemo(
    () =>
      projects
        .map((p) => ({ p, score: fuzzyScore(p.label, query) }))
        .filter((r): r is { p: ScopeProject; score: number } => r.score !== null)
        .sort((a, b) => b.score - a.score)
        .map((r) => r.p),
    [projects, query],
  );
  const options: Option[] = query ? matches : [null, ...matches];

  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const scope = (opt: Option) => {
    onScope(opt?.name);
    onClose();
  };

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (options.length === 0) return;
    const opt = options[Math.min(active, options.length - 1)];
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => (a + 1) % options.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => (a - 1 + options.length) % options.length);
    } else if (eventToChord(e, IS_MACOS) === "mod+enter") {
      e.preventDefault();
      if (opt) onTogglePin(opt.name);
    } else if (e.key === "Enter") {
      e.preventDefault();
      scope(opt);
    }
  }

  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      trapFocus
      initialFocusRef={inputRef}
      aria-label="Scope to project"
      className="flex max-h-80 w-[300px] flex-col overflow-y-auto p-0"
    >
      <div className="shrink-0 border-b border-edge-2 p-2.5">
        <label className="group/search flex h-7 items-center gap-1.5 rounded-control border border-edge px-2 focus-within:border-info">
          <Icon name="search" size="xs" className="text-ink-4 group-focus-within/search:text-ink" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
            placeholder="Find a project"
            aria-label="Find a project"
            className="min-w-0 flex-1 bg-transparent text-ui text-ink placeholder:text-ink-4 focus:outline-none"
          />
        </label>
      </div>
      <p className="shrink-0 px-3 pb-1 pt-2.5 text-ui-sm font-semibold text-ink-dim">
        Matches · {matches.length} of {projects.length}
      </p>
      <ul role="listbox" aria-label="Projects" className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-1.5">
        {options.map((opt, i) => {
          const isActive = i === active;
          const isCurrent = opt ? opt.name === current : !current;
          const isPinned = !!opt && pinned.includes(opt.name);
          return (
            <li key={opt?.name ?? ""} className="group relative">
              <button
                ref={isActive ? activeRef : undefined}
                type="button"
                role="option"
                aria-selected={isActive}
                onMouseEnter={() => setActive(i)}
                onClick={() => scope(opt)}
                className={rowClass({
                  tier: "compact",
                  selected: isActive ? "spine" : "none",
                  className: cx(
                    "flex w-full gap-2 pl-2.5 text-left text-ui",
                    opt ? "pr-8" : "pr-2.5",
                    isCurrent ? "font-semibold text-ink" : "text-ink-2",
                  ),
                })}
              >
                {isActive && <RowSpine />}
                {opt ? (
                  <RepoAvatar source={opt.originUrl} name={opt.label} size={14} className="rounded" />
                ) : (
                  <Icon name="folder" size="sm" className="text-ink-dim" />
                )}
                <span className="min-w-0 truncate">
                  {opt ? <Highlighted text={opt.label} query={query} /> : "All projects"}
                </span>
                <span className="ml-auto shrink-0 text-meta text-ink-4">{opt?.count}</span>
              </button>
              {opt && (
                // A sibling, not a child: a button can't nest inside the option button.
                <button
                  type="button"
                  aria-label={isPinned ? `Unpin ${opt.label}` : `Pin ${opt.label}`}
                  aria-pressed={isPinned}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => {
                    onTogglePin(opt.name);
                    inputRef.current?.focus();
                  }}
                  className={cx(
                    "absolute right-1.5 top-1/2 flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded hover:bg-edge/70 hover:text-ink focus:opacity-100",
                    isPinned ? "text-ink-dim" : "text-ink-4 opacity-0 group-hover:opacity-100",
                  )}
                >
                  <Icon name="star" size="xs" />
                </button>
              )}
            </li>
          );
        })}
      </ul>
      <div className="flex shrink-0 items-center gap-2.5 border-t border-edge-2 bg-panel-2 px-3 py-2 text-ui-sm text-ink-4">
        <span>
          <Kbd chord="enter" /> scope to project
        </span>
        <span>
          <Kbd chord="mod+enter" /> pin
        </span>
      </div>
    </Popover>
  );
}

// Mirrors fuzzyScore's greedy left-to-right scan so the bold letters are the ones that matched.
function Highlighted({ text, query }: { text: string; query: string }) {
  const lower = text.toLowerCase();
  const hits = new Set<number>();
  let from = 0;
  for (const ch of query.toLowerCase()) {
    const i = lower.indexOf(ch, from);
    if (i === -1) return text;
    hits.add(i);
    from = i + 1;
  }
  const runs: { bold: boolean; text: string }[] = [];
  for (let i = 0; i < text.length; i++) {
    const bold = hits.has(i);
    const last = runs[runs.length - 1];
    if (last && last.bold === bold) last.text += text[i];
    else runs.push({ bold, text: text[i] });
  }
  return runs.map((r, i) =>
    r.bold ? (
      <b key={i} className="font-bold text-ink">
        {r.text}
      </b>
    ) : (
      <span key={i}>{r.text}</span>
    ),
  );
}
