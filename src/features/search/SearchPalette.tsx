import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";

import { closeSearch, useSearchStore } from "./searchStore";
import { groupResults, flattenGroups, type SearchResult, type SearchGroup } from "./search";
import { useSearchSources } from "./useSearchSources";
import { selectAgentTab } from "./selectAgentTab";
import { openDocPreviewTab } from "@/features/session/workstreamTabs";
import { BookIcon, FolderIcon, TerminalIcon } from "@/components/icons";
import { ActivityDot } from "@/components/ActivityDot";
import { rowClass } from "@/components/ui";


function KindIcon({ kind }: { kind: SearchResult["kind"] }) {
  const cls = "shrink-0 text-ink-dim";
  switch (kind) {
    case "workstream":
      return (
        <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" className={cls}>
          <circle cx="6" cy="5" r="2.3" stroke="currentColor" strokeWidth="1.8" />
          <circle cx="6" cy="19" r="2.3" stroke="currentColor" strokeWidth="1.8" />
          <circle cx="18" cy="8" r="2.3" stroke="currentColor" strokeWidth="1.8" />
          <path d="M6 7.5v9" stroke="currentColor" strokeWidth="1.8" />
          <path d="M18 10.5c0 3-2.5 4.5-5.5 4.5H9.5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      );
    case "agent":
      return <TerminalIcon size={13} className={cls} />;
    case "doc":
      return <BookIcon size={13} />;
    case "file":
      return <FolderIcon size={13} className={cls} />;
  }
}


function dispatchNav(
  result: SearchResult,
  navigate: ReturnType<typeof useNavigate>,
  activeWorkstreamId: string | undefined,
): void {
  const nav = result.nav;
  switch (nav.type) {
    case "workstream":
      void navigate({
        to: "/projects/$projectPath",
        params: { projectPath: nav.projectPath },
        search: { item: nav.workstreamId },
      });
      break;
    case "agent":
      void navigate({
        to: "/projects/$projectPath",
        params: { projectPath: nav.projectPath },
        search: { item: nav.workstreamId },
      });
      selectAgentTab(nav.workstreamId, nav.sessionId);
      break;
    case "doc":
      if (activeWorkstreamId) {
        openDocPreviewTab(activeWorkstreamId, nav.id, result.title);
      } else {
        void navigate({ to: "/library", search: { doc: nav.id } });
      }
      break;
    case "file":
      break;
  }
  closeSearch();
}


function EmptyHint() {
  return (
    <div className="px-4 py-8 text-center text-sm text-ink-dim">
      Type to search workstreams, documents, and agents
    </div>
  );
}


function ResultRow({
  result,
  active,
  onMouseEnter,
  onClick,
}: {
  result: SearchResult;
  active: boolean;
  onMouseEnter: () => void;
  onClick: () => void;
}) {
  const rowRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (active) rowRef.current?.scrollIntoView({ block: "nearest" });
  }, [active]);

  return (
    <button
      ref={rowRef}
      type="button"
      role="option"
      aria-selected={active}
      onMouseEnter={onMouseEnter}
      onClick={onClick}
      className={rowClass({
        tier: "standard",
        className: `flex w-full gap-2 rounded-none px-4 text-left text-sm transition-colors ${
          active ? "bg-edge/60 text-ink" : "text-ink hover:bg-edge/50"
        }`,
      })}
    >
      <KindIcon kind={result.kind} />
      <span className="min-w-0 flex-1 truncate font-medium">{result.title}</span>
      {result.subtitle && (
        <span className="shrink-0 truncate text-xs text-ink-dim/70">{result.subtitle}</span>
      )}
      {result.activity && result.activity !== "idle" && <ActivityDot activity={result.activity} />}
      {result.tag && (
        <span className="shrink-0 rounded bg-edge/60 px-1.5 py-0.5 text-[10px] text-ink-dim">
          {result.tag}
        </span>
      )}
    </button>
  );
}


function Section({
  group,
  activeIndex,
  flatOffset,
  onHover,
  onSelect,
}: {
  group: SearchGroup;
  activeIndex: number;
  flatOffset: number;
  onHover: (i: number) => void;
  onSelect: (result: SearchResult) => void;
}) {
  return (
    <div>
      <div className="px-4 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-ink-dim/60">
        {group.label}
      </div>
      {group.results.map((result, i) => (
        <ResultRow
          key={result.key}
          result={result}
          active={activeIndex === flatOffset + i}
          onMouseEnter={() => onHover(flatOffset + i)}
          onClick={() => onSelect(result)}
        />
      ))}
    </div>
  );
}


function useActiveWorkstreamId(): string | undefined {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const locationSearch = useRouterState({ select: (s) => s.location.search }) as {
    item?: string;
  };
  if (!pathname.startsWith("/projects/")) return undefined;
  return locationSearch.item;
}

export function SearchPalette() {
  const open = useSearchStore((s) => s.open);
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const activeWorkstreamId = useActiveWorkstreamId();
  const allResults = useSearchSources(query, activeWorkstreamId);
  const groups = groupResults(allResults);
  const flat = flattenGroups(groups);

  const clampedActive = flat.length > 0 ? Math.min(activeIndex, flat.length - 1) : 0;

  useEffect(() => {
    if (open) {
      setQuery("");
      setActiveIndex(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  const handleSelect = useCallback(
    (result: SearchResult) => {
      dispatchNav(result, navigate, activeWorkstreamId);
    },
    [navigate, activeWorkstreamId],
  );

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (flat.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => (i + 1) % flat.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => (i - 1 + flat.length) % flat.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (flat[clampedActive]) handleSelect(flat[clampedActive]);
    }
  };

  if (!open) return null;

  let offset = 0;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-surface/60 pt-[12vh] backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) closeSearch();
      }}
    >
      <div
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={true}
        aria-label="Search everything"
        className="w-full max-w-xl overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl"
        onKeyDown={handleKeyDown}
      >
        <div className="flex items-center gap-2 border-b border-edge px-4 py-3">
          <svg
            aria-hidden="true"
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            className="shrink-0 text-ink-dim"
          >
            <circle cx="11" cy="11" r="7" stroke="currentColor" strokeWidth="1.8" />
            <path d="m16.5 16.5 4 4" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActiveIndex(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                closeSearch();
              }
            }}
            placeholder="Search workstreams, documents, agents…"
            className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-dim/50"
            aria-autocomplete="list"
            aria-controls="search-palette-results"
          />
          <kbd className="shrink-0 rounded bg-edge/60 px-1.5 py-0.5 text-[10px] font-medium text-ink-dim">
            esc
          </kbd>
        </div>

        <div
          id="search-palette-results"
          role="listbox"
          aria-label="Search results"
          className="max-h-80 overflow-y-auto py-1"
        >
          {flat.length === 0 ? (
            <EmptyHint />
          ) : (
            groups.map((group) => {
              const groupOffset = offset;
              offset += group.results.length;
              return (
                <Section
                  key={group.kind}
                  group={group}
                  activeIndex={clampedActive}
                  flatOffset={groupOffset}
                  onHover={setActiveIndex}
                  onSelect={handleSelect}
                />
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
