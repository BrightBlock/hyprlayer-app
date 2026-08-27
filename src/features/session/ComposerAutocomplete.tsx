import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";

import { commands as ipc, type SlashCommand } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { basename } from "@/lib/path";
import { useBootstrap } from "@/lib/queries";
import {
  customPrompts,
  effectiveCustomBody,
  matchComposerPrompt,
  offeredPrompts,
  promptFirstLine,
  type ComposerPromptMatch,
} from "@/features/prompts/prompts";
import type { CustomPrompt } from "@/lib/bindings";

type Trigger = { type: "command" | "file"; start: number; query: string };

export type Suggestion = {
  value: string;
  label: string;
  detail: string;
  badge?: string;
  sigil: string;
};

const FILE_LIMIT = 20;
const FILE_DEBOUNCE_MS = 100;

export function detectTrigger(text: string, caret: number): Trigger | null {
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

export function isClaudeFamilyModel(model: string | null | undefined): boolean {
  if (!model) return false;
  const id = model.toLowerCase();
  return id.startsWith("anthropic/") || id.includes("claude");
}

export function skillSigilFor(harness: string, model?: string | null): string {
  if (harness === "claude") return "/";
  if (harness === "opencode" && isClaudeFamilyModel(model)) return "/";
  return "$";
}

export function resolvesInHarness(command: SlashCommand, harness: string): boolean {
  return harness !== "codex" || command.kind === "skill";
}

function dirname(path: string): string {
  const i = path.lastIndexOf("/");
  return i >= 0 ? path.slice(0, i) : "";
}

export type ComposerAutocomplete = {
  open: boolean;
  mode: "command" | "file" | null;
  suggestions: Suggestion[];
  activeIndex: number;
  setActiveIndex: (i: number) => void;
  loading: boolean;
  commandNames: Set<string>;
  skillSigil: string;
  matchPrompt: (draft: string) => ComposerPromptMatch | null;
  offeredPrompts: ReadonlyMap<string, CustomPrompt>;
  apply: (index: number) => void;
  handleKeyDown: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
  syncCaret: () => void;
};

export function useComposerAutocomplete(opts: {
  workstreamId: string;
  projectPath: string;
  value: string;
  onChange: (next: string) => void;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  enabled: boolean;
  harness: string;
  model?: string | null;
  onPickFile?: (path: string) => void;
}): ComposerAutocomplete {
  const {
    workstreamId,
    projectPath,
    value,
    onChange,
    textareaRef,
    enabled,
    harness,
    model,
    onPickFile,
  } = opts;

  const [slashCommands, setSlashCommands] = useState<SlashCommand[] | null>(null);
  const [caret, setCaret] = useState(0);
  const [activeIndex, setActiveIndex] = useState(0);
  const [files, setFiles] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const dismissed = useRef<string | null>(null);

  useEffect(() => {
    let alive = true;
    unwrap(ipc.listSlashCommands(workstreamId))
      .then((c) => alive && setSlashCommands(c))
      .catch(() => alive && setSlashCommands([]));
    return () => {
      alive = false;
    };
  }, [workstreamId]);

  const settings = useBootstrap().data?.settings;
  const prompts = useMemo(() => customPrompts(settings), [settings]);
  const offered = useMemo(
    () =>
      slashCommands === null
        ? new Map<string, CustomPrompt>()
        : offeredPrompts(prompts, projectPath, new Set(slashCommands.map((c) => c.name))),
    [prompts, slashCommands, projectPath],
  );

  const skillSigil = skillSigilFor(harness, model);

  const allCommands = useMemo<SlashCommand[]>(
    () => [
      ...[...offered].map(([slug, p]) => ({
        name: slug,
        description: promptFirstLine(effectiveCustomBody(p, projectPath)),
        source: "prompt" as const,
        kind: "skill" as const,
      })),
      ...(slashCommands ?? []).filter((c) => resolvesInHarness(c, harness)),
    ],
    [offered, slashCommands, projectPath, harness],
  );

  const matchPrompt = useCallback((draft: string) => matchComposerPrompt(draft, offered), [offered]);

  const trigger = useMemo(
    () => (enabled ? detectTrigger(value, caret) : null),
    [enabled, value, caret],
  );
  const triggerKey = trigger ? `${trigger.type}:${trigger.start}:${trigger.query}` : null;

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
      return filterCommands(allCommands, trigger.query).map((c) => ({
        value: c.name,
        label: c.name,
        detail: c.description,
        badge: c.source === "prompt" ? "prompt" : undefined,
        sigil: c.kind === "skill" ? skillSigil : "/",
      }));
    }
    if (trigger?.type === "file") {
      return files.map((path) => ({
        value: path,
        label: basename(path),
        detail: dirname(path),
        sigil: "@",
      }));
    }
    return [];
  }, [trigger, allCommands, files, skillSigil]);

  useEffect(() => {
    setActiveIndex(0);
  }, [triggerKey]);

  const dismissedHere = dismissed.current !== null && dismissed.current === triggerKey;
  const open =
    enabled &&
    trigger !== null &&
    !dismissedHere &&
    (suggestions.length > 0 || (trigger.type === "file" && loading));

  const commandNames = useMemo(() => new Set(allCommands.map((c) => c.name)), [allCommands]);

  const apply = useCallback(
    (index: number) => {
      const item = suggestions[index];
      if (!item || !trigger) return;

      if (trigger.type === "file" && onPickFile) {
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

      const insert = `${item.sigil}${item.value} `;
      const next = value.slice(0, trigger.start) + insert + value.slice(caret);
      onChange(next);
      const newCaret = trigger.start + insert.length;
      dismissed.current = null;
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
    skillSigil,
    matchPrompt,
    offeredPrompts: offered,
    apply,
    handleKeyDown,
    syncCaret,
  };
}

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
                onMouseDown={(e) => {
                  e.preventDefault();
                  state.apply(i);
                }}
                onMouseEnter={() => state.setActiveIndex(i)}
                className={`flex w-full items-baseline gap-2 px-3 py-1.5 text-left ${
                  active ? "bg-accent/15" : "hover:bg-edge/50"
                }`}
              >
                <span className="shrink-0 font-medium text-ink">
                  {state.mode === "command" ? `${s.sigil}${s.label}` : s.label}
                </span>
                {s.detail && (
                  <span className="truncate text-xs text-ink-dim">{s.detail}</span>
                )}
                {s.badge && (
                  <span className="ml-auto shrink-0 rounded bg-edge/60 px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide text-ink-dim">
                    {s.badge}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export function highlightSegments(
  text: string,
  commandNames: Set<string>,
  sigil = "/",
): { text: string; skill: boolean }[] {
  const segments: { text: string; skill: boolean }[] = [];
  const re = new RegExp(`(^|\\s)(\\${sigil}[\\w:-]+)`, "g");
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

export function HighlightBackdrop({
  value,
  commandNames,
  sigil,
  scrollTop,
  argHint,
}: {
  value: string;
  commandNames: Set<string>;
  sigil: string;
  scrollTop: number;
  argHint?: string | null;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = scrollTop;
  }, [scrollTop, value]);

  const segments = useMemo(
    () => highlightSegments(value, commandNames, sigil),
    [value, commandNames, sigil],
  );

  return (
    <div
      ref={ref}
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-3.5 py-4 text-sm text-transparent"
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
      {argHint && (
        <span className="text-ink-dim/70">{/\s$/.test(value) ? argHint : ` ${argHint}`}</span>
      )}
      {"\n"}
    </div>
  );
}
