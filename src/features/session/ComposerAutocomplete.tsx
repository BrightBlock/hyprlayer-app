import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";

import { commands as ipc, type SlashCommand } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { basename } from "../../lib/path";

/** A live autocomplete trigger detected at the caret: a `/command` or `@file`
 *  token, either of which can open any whitespace-delimited token in the draft.
 *  `start` is the index of the `/`/`@` itself, so applying a pick can splice
 *  from there. */
type Trigger = { type: "command" | "file"; start: number; query: string };

/** What the dropdown renders for one row, normalized across command/file modes. */
export type Suggestion = {
  /** The text inserted after the trigger char (command name or file path). */
  value: string;
  /** Primary label (command name / file basename). */
  label: string;
  /** Secondary detail (description / directory), shown dimmed. */
  detail: string;
};

const FILE_LIMIT = 20;
const FILE_DEBOUNCE_MS = 100;

/** Detect whether the caret sits inside an autocomplete trigger. Either a `/`
 *  or `@` that opens the current whitespace-delimited token fires — anywhere
 *  in the draft, matching the in-prompt skill highlight. */
export function detectTrigger(text: string, caret: number): Trigger | null {
  // Walk back to the start of the whitespace-delimited token under the caret.
  let i = caret;
  while (i > 0 && !/\s/.test(text[i - 1])) i -= 1;
  const token = text.slice(i, caret);

  if (token.startsWith("/")) {
    return { type: "command", start: i, query: token.slice(1) };
  }
  if (token.startsWith("@")) {
    return { type: "file", start: i, query: token.slice(1) };
  }
  return null;
}

/** Rank a command against the typed query: prefix matches first, then any
 *  substring, dropping non-matches. Keeps the original order within a tier. */
function filterCommands(all: SlashCommand[], query: string): SlashCommand[] {
  const q = query.toLowerCase();
  if (!q) return all;
  const prefix: SlashCommand[] = [];
  const substr: SlashCommand[] = [];
  for (const c of all) {
    const name = c.name.toLowerCase();
    if (name.startsWith(q)) prefix.push(c);
    else if (name.includes(q)) substr.push(c);
  }
  return [...prefix, ...substr];
}

function dirname(path: string): string {
  const i = path.lastIndexOf("/");
  return i >= 0 ? path.slice(0, i) : "";
}

export type ComposerAutocomplete = {
  /** Whether the dropdown should be visible. */
  open: boolean;
  mode: "command" | "file" | null;
  suggestions: Suggestion[];
  activeIndex: number;
  setActiveIndex: (i: number) => void;
  /** A file search is in flight (shows a hint instead of "no matches"). */
  loading: boolean;
  /** Names of every known command — drives the in-prompt skill highlight. */
  commandNames: Set<string>;
  /** Apply the suggestion at `index`, splicing it into the draft. */
  apply: (index: number) => void;
  /** Call FIRST inside the textarea's onKeyDown. Returns true when it consumed
   *  the key (navigation / accept / dismiss) so the caller skips its own
   *  handling (e.g. Enter-to-send). */
  handleKeyDown: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
  /** Keep the tracked caret in sync after any selection change. */
  syncCaret: () => void;
};

/** Drives `/command` and `@file` autocomplete plus the recognized-skill
 *  highlight for the composer textarea. The caller owns the `value`/`onChange`
 *  of the (controlled) textarea and the ref; this hook only reads the caret and
 *  splices accepted picks back through `onChange`.
 *
 *  When `onPickFile` is provided, an accepted `@file` pick is promoted to an
 *  attachment card instead of being spliced back into the textarea: the
 *  `@query` token is removed from the draft and the worktree-relative path is
 *  handed to the host. Falls back to the inline `@path ` splice when omitted. */
export function useComposerAutocomplete(opts: {
  workstreamId: string;
  value: string;
  onChange: (next: string) => void;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  enabled: boolean;
  onPickFile?: (path: string) => void;
}): ComposerAutocomplete {
  const { workstreamId, value, onChange, textareaRef, enabled, onPickFile } = opts;

  const [slashCommands, setSlashCommands] = useState<SlashCommand[]>([]);
  const [caret, setCaret] = useState(0);
  const [activeIndex, setActiveIndex] = useState(0);
  const [files, setFiles] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  // Trigger signature the user dismissed with Escape; suppresses the dropdown
  // until the query changes (so Escape doesn't fight the live trigger).
  const dismissed = useRef<string | null>(null);

  // Discover available slash commands once per worktree (and on focus changes
  // the worktree id is stable, so this is effectively load-once).
  useEffect(() => {
    let alive = true;
    unwrap(ipc.listSlashCommands(workstreamId))
      .then((c) => alive && setSlashCommands(c))
      .catch(() => alive && setSlashCommands([]));
    return () => {
      alive = false;
    };
  }, [workstreamId]);

  const trigger = useMemo(
    () => (enabled ? detectTrigger(value, caret) : null),
    [enabled, value, caret],
  );
  const triggerKey = trigger ? `${trigger.type}:${trigger.start}:${trigger.query}` : null;

  // Debounced file search whenever an `@file` trigger is active.
  useEffect(() => {
    if (trigger?.type !== "file") {
      setFiles([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    const query = trigger.query;
    let alive = true;
    const handle = setTimeout(() => {
      unwrap(ipc.worktreeSearchFiles(workstreamId, query, FILE_LIMIT))
        .then((r) => {
          if (alive) {
            setFiles(r);
            setLoading(false);
          }
        })
        .catch(() => {
          if (alive) {
            setFiles([]);
            setLoading(false);
          }
        });
    }, FILE_DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(handle);
    };
  }, [trigger?.type, trigger?.query, workstreamId]);

  const suggestions = useMemo<Suggestion[]>(() => {
    if (trigger?.type === "command") {
      return filterCommands(slashCommands, trigger.query).map((c) => ({
        value: c.name,
        label: c.name,
        detail: c.description,
      }));
    }
    if (trigger?.type === "file") {
      return files.map((path) => ({
        value: path,
        label: basename(path),
        detail: dirname(path),
      }));
    }
    return [];
  }, [trigger, slashCommands, files]);

  // Reset the highlighted row whenever the candidate set changes.
  useEffect(() => {
    setActiveIndex(0);
  }, [triggerKey]);

  const dismissedHere = dismissed.current !== null && dismissed.current === triggerKey;
  const open =
    enabled &&
    trigger !== null &&
    !dismissedHere &&
    (suggestions.length > 0 || (trigger.type === "file" && loading));

  const commandNames = useMemo(
    () => new Set(slashCommands.map((c) => c.name)),
    [slashCommands],
  );

  const apply = useCallback(
    (index: number) => {
      const item = suggestions[index];
      if (!item || !trigger) return;

      if (trigger.type === "file" && onPickFile) {
        // Promote the @file pick to an attachment card: delete the `@query`
        // from the draft and hand the path off to the host. If the deletion
        // would leave a doubled space, eat one to keep the draft tidy.
        let removeTo = caret;
        if (value[trigger.start - 1] === " " && value[removeTo] === " ") {
          removeTo += 1;
        }
        const next = value.slice(0, trigger.start) + value.slice(removeTo);
        onChange(next);
        onPickFile(item.value);
        const newCaret = trigger.start;
        dismissed.current = null;
        requestAnimationFrame(() => {
          const el = textareaRef.current;
          if (el) {
            el.focus();
            el.setSelectionRange(newCaret, newCaret);
          }
          setCaret(newCaret);
        });
        return;
      }

      const insert =
        trigger.type === "command" ? `/${item.value} ` : `@${item.value} `;
      const next = value.slice(0, trigger.start) + insert + value.slice(caret);
      onChange(next);
      const newCaret = trigger.start + insert.length;
      dismissed.current = null;
      // The textarea re-renders from the new value first; restore the caret on
      // the next frame so it lands after the inserted token.
      requestAnimationFrame(() => {
        const el = textareaRef.current;
        if (el) {
          el.focus();
          el.setSelectionRange(newCaret, newCaret);
        }
        setCaret(newCaret);
      });
    },
    [suggestions, trigger, value, caret, onChange, textareaRef, onPickFile],
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>): boolean => {
      if (!open) return false;
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setActiveIndex((i) => (i + 1) % suggestions.length);
          return true;
        case "ArrowUp":
          e.preventDefault();
          setActiveIndex((i) => (i - 1 + suggestions.length) % suggestions.length);
          return true;
        case "Enter":
        case "Tab":
          if (suggestions.length > 0) {
            e.preventDefault();
            apply(activeIndex);
            return true;
          }
          return false;
        case "Escape":
          e.preventDefault();
          dismissed.current = triggerKey;
          return true;
        default:
          return false;
      }
    },
    [open, suggestions.length, apply, activeIndex, triggerKey],
  );

  const syncCaret = useCallback(() => {
    const el = textareaRef.current;
    if (el) setCaret(el.selectionStart);
  }, [textareaRef]);

  return {
    open,
    mode: trigger?.type ?? null,
    suggestions,
    activeIndex,
    setActiveIndex,
    loading,
    commandNames,
    apply,
    handleKeyDown,
    syncCaret,
  };
}

/** The popover list of `/command` or `@file` suggestions, anchored above the
 *  composer (which sits at the bottom of the pane). */
export function AutocompleteDropdown({
  state,
  emptyHint,
}: {
  state: ComposerAutocomplete;
  emptyHint?: string;
}) {
  const activeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest" });
  }, [state.activeIndex]);

  if (!state.open) return null;

  const showLoading = state.suggestions.length === 0 && state.loading;

  return (
    <div className="absolute bottom-full left-0 z-50 mb-1 w-full max-w-md overflow-hidden rounded-lg border border-edge bg-panel text-sm shadow-lg">
      <div className="border-b border-edge/60 px-3 py-1.5 text-[11px] font-medium uppercase tracking-wide text-ink-dim/70">
        {state.mode === "command" ? "Skills & commands" : "Files"}
      </div>
      <ul className="max-h-64 overflow-y-auto py-1">
        {showLoading && (
          <li className="px-3 py-2 text-sm text-ink-dim">Searching…</li>
        )}
        {!showLoading && state.suggestions.length === 0 && (
          <li className="px-3 py-2 text-sm text-ink-dim">{emptyHint ?? "No matches"}</li>
        )}
        {state.suggestions.map((s, i) => {
          const active = i === state.activeIndex;
          return (
            <li key={`${s.value}-${i}`}>
              <button
                ref={active ? activeRef : undefined}
                type="button"
                // Keep the textarea focused: prevent the mousedown from blurring it.
                onMouseDown={(e) => {
                  e.preventDefault();
                  state.apply(i);
                }}
                onMouseEnter={() => state.setActiveIndex(i)}
                className={`flex w-full items-baseline gap-2 px-3 py-1.5 text-left ${
                  active ? "bg-accent/15" : "hover:bg-edge/40"
                }`}
              >
                <span className="shrink-0 font-medium text-ink">
                  {state.mode === "command" ? `/${s.label}` : s.label}
                </span>
                {s.detail && (
                  <span className="truncate text-xs text-ink-dim">{s.detail}</span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** Tokenize `text` into plain runs and recognized `/skill` runs (a slash token,
 *  bounded by start/whitespace, whose name is a known command). */
export function highlightSegments(
  text: string,
  commandNames: Set<string>,
): { text: string; skill: boolean }[] {
  const segments: { text: string; skill: boolean }[] = [];
  const re = /(^|\s)(\/[\w:-]+)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const name = m[2].slice(1);
    if (!commandNames.has(name)) continue;
    const tokenStart = m.index + m[1].length;
    if (tokenStart > last) segments.push({ text: text.slice(last, tokenStart), skill: false });
    segments.push({ text: m[2], skill: true });
    last = tokenStart + m[2].length;
  }
  if (last < text.length) segments.push({ text: text.slice(last), skill: false });
  return segments;
}

/** A transparent-text mirror of the textarea, sitting behind it, that paints a
 *  highlight pill behind any recognized `/skill` token. The textarea's own text
 *  renders on top (its background is transparent), so only the pill shows
 *  through. Typography/padding/scroll must match the textarea exactly. */
export function HighlightBackdrop({
  value,
  commandNames,
  scrollTop,
}: {
  value: string;
  commandNames: Set<string>;
  scrollTop: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = scrollTop;
  }, [scrollTop, value]);

  const segments = useMemo(
    () => highlightSegments(value, commandNames),
    [value, commandNames],
  );

  return (
    <div
      ref={ref}
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-3.5 py-3 text-sm text-transparent"
    >
      {segments.map((seg, i) =>
        seg.skill ? (
          <mark
            key={i}
            className="rounded bg-accent/20 text-transparent ring-1 ring-inset ring-accent/30"
          >
            {seg.text}
          </mark>
        ) : (
          <span key={i}>{seg.text}</span>
        ),
      )}
      {/* Trailing newline guard so the box height matches the textarea's. */}
      {"\n"}
    </div>
  );
}
