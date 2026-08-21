import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import type { CustomPrompt, WorkstreamView } from "@/lib/bindings";
import { useBootstrap } from "@/lib/queries";
import { IS_MACOS } from "@/lib/platform";
import { ChordHint, rowClass } from "@/components/ui";
import { effectiveChord, type Overrides } from "@/lib/keybindings/resolve";
import { GitCommitIcon, GitPullRequestIcon, PencilIcon, ZapIcon } from "@/components/icons";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";
import { closePromptPalette, usePromptPaletteStore } from "./promptsStore";
import {
  BUILTIN_PROMPTS,
  customPrompts,
  effectiveBuiltinBody,
  effectiveCustomBody,
  isRunnablePrompt,
  promptFirstLine,
  type BuiltinPromptId,
} from "./prompts";
import { runCustomPrompt } from "./runPrompt";

type Row = {
  key: string;
  name: string;
  desc: string;
  chord: string | null;
  icon: React.ReactNode;
  /** Why the row can't run right now (rendered as the row's title); null = runnable. */
  disabled: string | null;
  run: () => void;
};

const EMPTY_OVERRIDES: Overrides = {};

/**
 * The prompt palette (⌘K): every prompt in the library, filtered as you type,
 * Enter runs the selection in the focused workstream. The layout mirrors
 * `SearchPalette` (⌘P) so the two read as one family of overlays.
 */
export function PromptPalette({ activeWorkstream }: { activeWorkstream: WorkstreamView | null }) {
  const open = usePromptPaletteStore((s) => s.open);
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const gitActions = useWorkstreamGitActions(activeWorkstream);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActiveIndex(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  const settings = bootstrap.data?.settings;
  const defaults = bootstrap.data?.promptDefaults;
  const overrides = settings?.keybindings?.overrides ?? EMPTY_OVERRIDES;
  const projectPath = activeWorkstream?.item.projectPath ?? null;

  const rows = useMemo<Row[]>(() => {
    if (!open) return [];
    const q = query.trim().toLowerCase();
    const noWs = activeWorkstream ? null : "Open a workstream to run prompts";

    const builtinRow = (id: BuiltinPromptId): Row | null => {
      // Commit/Ship can never apply to a folder workstream, hide, don't gray.
      if (activeWorkstream && isFolderWorkstream(activeWorkstream)) return null;
      const def = BUILTIN_PROMPTS.find((b) => b.id === id)!;
      const body = effectiveBuiltinBody(settings, defaults, id);
      if (q && !def.name.toLowerCase().includes(q) && !body.toLowerCase().includes(q)) {
        return null;
      }
      const gate =
        id === "commit"
          ? gitActions.canCommit && !gitActions.committing
            ? null
            : "Nothing to commit right now"
          : gitActions.canShip && !gitActions.shipping
            ? null
            : "Nothing to ship right now";
      return {
        key: `builtin:${id}`,
        name: def.name,
        desc: promptFirstLine(body),
        chord: effectiveChord(overrides, def.commandId, IS_MACOS),
        icon:
          id === "commit" ? (
            <GitCommitIcon size={14} className="shrink-0 text-accent" />
          ) : (
            <GitPullRequestIcon size={14} className="shrink-0 text-accent" />
          ),
        disabled: noWs ?? gate,
        run: () => {
          if (id === "commit") gitActions.doCommit();
          else gitActions.doShip();
        },
      };
    };

    const customRow = (p: CustomPrompt): Row | null => {
      const name = p.name || "Prompt";
      const body = effectiveCustomBody(p, projectPath);
      if (q && !name.toLowerCase().includes(q) && !body.toLowerCase().includes(q)) return null;
      return {
        key: `custom:${p.id}`,
        name,
        desc: promptFirstLine(body),
        chord: p.chord ?? null,
        icon: <ZapIcon size={14} className="shrink-0 text-accent" />,
        disabled: noWs ?? (isRunnablePrompt(p, projectPath) ? null : "This prompt has no text yet"),
        run: () => {
          if (activeWorkstream) void runCustomPrompt(activeWorkstream, p);
        },
      };
    };

    const out: Row[] = [
      ...BUILTIN_PROMPTS.map((b) => builtinRow(b.id)),
      ...customPrompts(settings).map(customRow),
    ].filter((r): r is Row => r !== null);

    if (!q || "manage prompts".includes(q)) {
      out.push({
        key: "manage",
        name: "Manage prompts…",
        desc: "Edit defaults, add custom prompts, assign shortcuts",
        chord: null,
        icon: <PencilIcon size={13} className="shrink-0 text-ink-dim" />,
        disabled: null,
        run: () => void navigate({ to: "/prompts" }),
      });
    }
    return out;
  }, [open, query, settings, defaults, overrides, projectPath, activeWorkstream, gitActions, navigate]);

  const clampedActive = rows.length > 0 ? Math.min(activeIndex, rows.length - 1) : 0;

  const select = (row: Row) => {
    if (row.disabled) return;
    row.run();
    closePromptPalette();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (rows.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => (i + 1) % rows.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => (i - 1 + rows.length) % rows.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (rows[clampedActive]) select(rows[clampedActive]);
    }
  };

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-surface/60 pt-[12vh] backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) closePromptPalette();
      }}
    >
      <div
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={true}
        aria-label="Prompt library"
        className="w-full max-w-xl overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl"
        onKeyDown={handleKeyDown}
      >
        <div className="flex items-center gap-2 border-b border-edge px-4 py-3">
          <ZapIcon size={15} className="shrink-0 text-ink-dim" />
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
                closePromptPalette();
              }
            }}
            placeholder="Run a prompt…"
            className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-dim/50"
            aria-autocomplete="list"
            aria-controls="prompt-palette-results"
          />
          <kbd className="shrink-0 rounded bg-edge/60 px-1.5 py-0.5 text-[10px] font-medium text-ink-dim">
            esc
          </kbd>
        </div>

        <div
          id="prompt-palette-results"
          role="listbox"
          aria-label="Prompts"
          className="max-h-80 overflow-y-auto py-1"
        >
          {rows.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-ink-dim">
              No prompts match “{query}”.
            </div>
          ) : (
            rows.map((row, i) => (
              <button
                key={row.key}
                type="button"
                role="option"
                aria-selected={clampedActive === i}
                aria-disabled={row.disabled !== null}
                title={row.disabled ?? undefined}
                onMouseEnter={() => setActiveIndex(i)}
                onClick={() => select(row)}
                className={rowClass({
                  tier: "standard",
                  // Rounded corners would show mid-list, clipped only by the
                  // palette's own `rounded-lg`, so cancel the tier's default
                  // radius. The keyboard-active state is the palette's own
                  // highlight, not Row's `selected` tint.
                  className: `flex w-full gap-2 rounded-none px-4 text-left text-sm transition-colors ${
                    clampedActive === i ? "bg-edge/60 text-ink" : "text-ink hover:bg-edge/50"
                  } ${row.disabled ? "opacity-50" : ""}`,
                })}
              >
                {row.icon}
                <span className="shrink-0 font-medium">{row.name}</span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-dim/70">{row.desc}</span>
                {row.chord && (
                  <ChordHint
                    chord={row.chord}
                    className="shrink-0 rounded bg-edge/60 px-1.5 py-0.5 text-xs text-ink-dim"
                  />
                )}
              </button>
            ))
          )}
        </div>

        <div className="flex items-center gap-3 border-t border-edge px-4 py-2 text-[11px] text-ink-dim">
          <span>
            <kbd className="rounded bg-edge/60 px-1 py-0.5 text-[10px]">↑↓</kbd> navigate
          </span>
          <span>
            <kbd className="rounded bg-edge/60 px-1 py-0.5 text-[10px]">↵</kbd> run
          </span>
          <span className="ml-auto text-ink-dim/60">
            {activeWorkstream
              ? "Prompts run in the focused workstream"
              : "Open a workstream to run prompts"}
          </span>
        </div>
      </div>
    </div>
  );
}
