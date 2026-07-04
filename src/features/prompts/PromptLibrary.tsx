import { useEffect, useMemo, useRef, useState } from "react";
import { confirm, open, save as saveFileDialog } from "@tauri-apps/plugin-dialog";

import { commands } from "../../lib/bindings";
import type { AppSettings, CustomPrompt, PromptSettings } from "../../lib/bindings";
import { useBootstrap, useSetAppSettings } from "../../lib/queries";
import { IS_MACOS } from "../../lib/platform";
import { useToast } from "../../lib/toast";
import { startEdgeDrag } from "../../lib/resize";
import { PROMPTS_SIDEBAR_MAX, PROMPTS_SIDEBAR_MIN, useUiStore } from "../../lib/uiStore";
import { Badge, Button, Card, ChordHint, ErrorNote, TextInput, Toggle } from "../../components/ui";
import {
  BracesIcon,
  ExportIcon,
  GitCommitIcon,
  GitPullRequestIcon,
  ImportIcon,
  PlusIcon,
  ZapIcon,
} from "../../components/icons";
import {
  eventToChord,
  formatChord,
  normalizeChord,
  type Chord,
} from "../../lib/keybindings/chord";
import { commandById } from "../../lib/keybindings/commands";
import { conflictsFor, effectiveChord, type Overrides } from "../../lib/keybindings/resolve";
import { isReserved } from "../../lib/keybindings/reserved";
import {
  BUILTIN_PROMPTS,
  PROMPT_VARS,
  effectiveBuiltinBody,
  isBuiltinModified,
  type BuiltinPromptId,
} from "./prompts";
import { ExportPromptsDialog } from "./ExportPromptsDialog";
import { ImportPromptsDialog } from "./ImportPromptsDialog";
import {
  PROMPTS_FILE_EXT,
  applyImport,
  buildPromptsFile,
  parsePromptsFile,
  planImport,
  type ExportEntry,
  type ExportOptions,
  type ImportCandidate,
  type ImportDecision,
  type LibrarySnapshot,
} from "./promptsIo";

const EMPTY_OVERRIDES: Overrides = {};
const SAVE_DEBOUNCE_MS = 600;

/** What actually goes to disk: a blank built-in override means "stock text",
 *  so drop the key rather than persisting an empty body that would run as an
 *  empty prompt. The LOCAL draft keeps the empty string while editing — the
 *  textarea must not snap back to the stock text mid-rewrite. */
function sanitizedForSave(prompts: PromptSettings): PromptSettings {
  const bodies = Object.fromEntries(
    Object.entries(prompts.builtinBodies ?? {}).filter(([, body]) => body.trim() !== ""),
  );
  return { ...prompts, builtinBodies: bodies };
}

/** How long the recorder pattern below matches KeyboardSettings: capture-phase
 *  listener beats the global dispatcher; Escape cancels, Enter commits. */
function useChordRecorder(
  recording: boolean,
  onCommit: (chord: Chord) => void,
  onCancel: () => void,
): Chord | null {
  const [candidate, setCandidate] = useState<Chord | null>(null);
  useEffect(() => {
    if (!recording) {
      setCandidate(null);
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        onCancel();
        return;
      }
      if (e.key === "Enter" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && candidate) {
        onCommit(candidate);
        return;
      }
      const chord = eventToChord(e, IS_MACOS);
      if (chord) setCandidate(chord);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, candidate, onCancel, onCommit]);
  return candidate;
}

function ChordPill({ chord }: { chord: Chord | null }) {
  if (!chord) return <span className="text-xs italic text-ink-dim">Not set</span>;
  return (
    <kbd
      className="inline-flex items-center rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-sm font-medium text-ink"
      style={IS_MACOS ? { letterSpacing: "0.1em", paddingRight: "calc(0.625rem - 0.1em)" } : undefined}
    >
      {formatChord(chord, IS_MACOS)}
    </kbd>
  );
}

type Selection =
  | { kind: "builtin"; id: BuiltinPromptId }
  | { kind: "custom"; prompt: CustomPrompt };

function newPromptId(): string {
  return typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `p-${Math.random().toString(36).slice(2)}`;
}

/**
 * The Prompt library — its own top-level view, opened from the nav rail's ⚡
 * entry (route `/prompts`, see `PromptsPage`). A "Prompts" master panel lists
 * built-in (Commit, Ship) and custom prompts; the selected prompt's editor
 * fills the content area — body text with `{{variable}}` insertion,
 * per-project overrides (custom only), keyboard shortcut, and pin-to-composer.
 * Selection lives in the URL (the page owns it); edits persist debounced
 * through the whole-settings mutation.
 */
export function PromptLibrary({
  settings,
  selectedId,
  onSelect,
}: {
  settings: AppSettings;
  selectedId: string;
  onSelect: (id: string) => void;
}) {
  const save = useSetAppSettings();
  const bootstrap = useBootstrap();
  const defaults = bootstrap.data?.promptDefaults ?? null;
  const projects = useMemo(() => bootstrap.data?.projects ?? [], [bootstrap.data?.projects]);

  // Drag-resizable master-panel width, in the UI store so it survives leaving
  // and re-opening the route (session-only, like the Library's list panel).
  const promptsWidth = useUiStore((s) => s.promptsWidth);
  const setPromptsWidth = useUiStore((s) => s.setPromptsWidth);

  // ── Draft state: the single source of truth while this page is open. ──────
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const [prompts, setPrompts] = useState<PromptSettings>(() => ({
    builtinBodies: { ...(settings.prompts?.builtinBodies ?? {}) },
    custom: (settings.prompts?.custom ?? []).map((p) => ({ ...p })),
  }));
  const promptsRef = useRef(prompts);
  promptsRef.current = prompts;
  const timer = useRef<number | null>(null);

  const persistNow = (extra?: Partial<AppSettings>) => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    save.mutate({ ...settingsRef.current, prompts: sanitizedForSave(promptsRef.current), ...extra });
  };
  const persistSoon = () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      save.mutate({ ...settingsRef.current, prompts: sanitizedForSave(promptsRef.current) });
    }, SAVE_DEBOUNCE_MS);
  };
  /** Flush a debounce-pending save immediately (blur); no-op when idle. */
  const flushPending = () => {
    if (timer.current !== null) persistNow();
  };
  // Flush a pending debounced save on unmount so no keystrokes are lost.
  useEffect(
    () => () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
        save.mutate({ ...settingsRef.current, prompts: sanitizedForSave(promptsRef.current) });
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const update = (fn: (p: PromptSettings) => PromptSettings, opts?: { immediate?: boolean }) => {
    const next = fn(promptsRef.current);
    promptsRef.current = next;
    setPrompts(next);
    if (opts?.immediate) persistNow();
    else persistSoon();
  };
  const updateCustom = (
    id: string,
    fn: (p: CustomPrompt) => CustomPrompt,
    opts?: { immediate?: boolean },
  ) =>
    update(
      (p) => ({ ...p, custom: (p.custom ?? []).map((c) => (c.id === id ? fn(c) : c)) }),
      opts,
    );

  // ── Selection (owned by the page via the URL). ────────────────────────────
  const custom = useMemo(() => prompts.custom ?? [], [prompts.custom]);
  const [scope, setScope] = useState<string>("global"); // "global" | project path
  const [recording, setRecording] = useState(false);
  const [varPickerOpen, setVarPickerOpen] = useState(false);
  const [projPickerOpen, setProjPickerOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const [exportOpen, setExportOpen] = useState(false);
  const [importFile, setImportFile] = useState<{
    fileName: string;
    source: string | null;
    candidates: ImportCandidate[];
  } | null>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);

  // Switching prompts resets the per-prompt editor state (scope tab, an
  // in-progress chord recording, open pickers) regardless of who navigated.
  useEffect(() => {
    setScope("global");
    setRecording(false);
    setVarPickerOpen(false);
    setProjPickerOpen(false);
  }, [selectedId]);

  const select = (id: string) => onSelect(id);

  const selection: Selection = useMemo(() => {
    const builtin = BUILTIN_PROMPTS.find((b) => b.id === selectedId);
    if (builtin) return { kind: "builtin", id: builtin.id };
    const prompt = custom.find((c) => c.id === selectedId);
    if (prompt) return { kind: "custom", prompt };
    return { kind: "builtin", id: "commit" };
  }, [selectedId, custom]);

  const kbOverrides = settings.keybindings?.overrides ?? EMPTY_OVERRIDES;
  const asSettings = useMemo(() => ({ ...settings, prompts }), [settings, prompts]);

  // ── Selected prompt's derived fields. ─────────────────────────────────────
  const isBuiltin = selection.kind === "builtin";
  const builtinDef = isBuiltin
    ? BUILTIN_PROMPTS.find((b) => b.id === (selection as { id: BuiltinPromptId }).id)!
    : null;
  const selPrompt = selection.kind === "custom" ? selection.prompt : null;
  const selName = builtinDef ? builtinDef.name : (selPrompt?.name ?? "");
  const isProjScope = scope !== "global";
  const hasScopeOverride = Boolean(
    selPrompt && isProjScope && selPrompt.overrides?.[scope] !== undefined,
  );
  const selBody = builtinDef
    ? effectiveBuiltinBody(asSettings, defaults, builtinDef.id)
    : isProjScope
      ? (selPrompt?.overrides?.[scope] ?? "")
      : (selPrompt?.body ?? "");
  const selModified = builtinDef ? isBuiltinModified(asSettings, defaults, builtinDef.id) : false;
  const selChord: Chord | null = builtinDef
    ? effectiveChord(kbOverrides, builtinDef.commandId, IS_MACOS)
    : selPrompt?.chord
      ? normalizeChord(selPrompt.chord)
      : null;

  const projectName = (path: string) =>
    projects.find((p) => p.path === path)?.mappedName ?? path.split("/").pop() ?? path;

  // ── Edit actions. ──────────────────────────────────────────────────────────
  const writeBody = (next: string) => {
    if (builtinDef) {
      update((p) => ({
        ...p,
        builtinBodies: { ...(p.builtinBodies ?? {}), [builtinDef.id]: next },
      }));
    } else if (selPrompt) {
      if (isProjScope) {
        updateCustom(selPrompt.id!, (c) => ({
          ...c,
          overrides: { ...(c.overrides ?? {}), [scope]: next },
        }));
      } else {
        updateCustom(selPrompt.id!, (c) => ({ ...c, body: next }));
      }
    }
  };

  const insertVariable = (token: string) => {
    const el = bodyRef.current;
    const cur = selBody;
    const start = el?.selectionStart ?? cur.length;
    const end = el?.selectionEnd ?? cur.length;
    writeBody(cur.slice(0, start) + token + cur.slice(end));
    setVarPickerOpen(false);
    // Put the caret after the inserted token (post-commit) so consecutive
    // inserts land where the user expects instead of at the end of the body.
    const caret = start + token.length;
    requestAnimationFrame(() => {
      const t = bodyRef.current;
      if (!t) return;
      t.focus();
      t.setSelectionRange(caret, caret);
    });
  };

  const addPrompt = () => {
    const id = newPromptId();
    update(
      (p) => ({
        ...p,
        custom: [
          ...(p.custom ?? []),
          { id, name: "Untitled prompt", body: "", overrides: {}, chord: null, pinned: false },
        ],
      }),
      { immediate: true },
    );
    setFilter("");
    select(id);
  };

  const deleteSelected = async () => {
    if (!selPrompt) return;
    const ok = await confirm(`Delete “${selPrompt.name ?? "this prompt"}”?`, {
      title: "Delete prompt?",
      kind: "warning",
    });
    if (!ok) return;
    update(
      (p) => ({ ...p, custom: (p.custom ?? []).filter((c) => c.id !== selPrompt.id) }),
      { immediate: true },
    );
    select("commit");
  };

  const resetBuiltin = () => {
    if (!builtinDef) return;
    update(
      (p) => {
        const bodies = { ...(p.builtinBodies ?? {}) };
        delete bodies[builtinDef.id];
        return { ...p, builtinBodies: bodies };
      },
      { immediate: true },
    );
  };

  const commitChord = (chord: Chord) => {
    const normalized = normalizeChord(chord);
    if (builtinDef) {
      // A built-in's shortcut IS its catalog command's binding — write the same
      // override Settings → Keyboard edits.
      persistNow({
        keybindings: { overrides: { ...kbOverrides, [builtinDef.commandId]: normalized } },
      });
    } else if (selPrompt) {
      updateCustom(selPrompt.id!, (c) => ({ ...c, chord: normalized }), { immediate: true });
    }
    setRecording(false);
  };

  const clearChord = () => {
    if (builtinDef) {
      persistNow({
        keybindings: { overrides: { ...kbOverrides, [builtinDef.commandId]: null } },
      });
    } else if (selPrompt) {
      updateCustom(selPrompt.id!, (c) => ({ ...c, chord: null }), { immediate: true });
    }
  };

  const candidate = useChordRecorder(recording, commitChord, () => setRecording(false));

  // Conflicts: catalog commands (excluding this built-in's own command) plus
  // other prompts' chords. Warn, don't block — matching KeyboardSettings.
  const conflictLabels = useMemo(() => {
    if (!candidate) return [];
    const selfCommandId = builtinDef?.commandId ?? "";
    const labels = conflictsFor(candidate, selfCommandId, kbOverrides, IS_MACOS).map(
      (id) => commandById(id)?.label ?? id,
    );
    for (const p of custom) {
      if (selPrompt && p.id === selPrompt.id) continue;
      if (p.chord && normalizeChord(p.chord) === candidate) labels.push(p.name ?? "a prompt");
    }
    return labels;
  }, [candidate, builtinDef, kbOverrides, custom, selPrompt]);
  const reserved = candidate ? isReserved(candidate, IS_MACOS) : false;

  // ── List rows. ─────────────────────────────────────────────────────────────
  const fq = filter.trim().toLowerCase();
  const matches = (name: string, body: string) =>
    !fq || name.toLowerCase().includes(fq) || body.toLowerCase().includes(fq);

  const listRow = (opts: {
    key: string;
    icon: React.ReactNode;
    name: string;
    chord: Chord | null;
    modified?: boolean;
    hasOverrides?: boolean;
    selected: boolean;
    onSelect: () => void;
  }) => (
    <button
      key={opts.key}
      type="button"
      onClick={opts.onSelect}
      aria-current={opts.selected}
      className={`flex w-full items-center gap-2 rounded-md border px-2.5 py-2 text-left text-sm transition-colors ${
        opts.selected
          ? "border-accent/30 bg-accent/10 text-ink"
          : "border-transparent text-ink hover:bg-edge/40"
      }`}
    >
      <span className={`shrink-0 ${opts.selected ? "text-accent" : "text-ink-dim"}`}>
        {opts.icon}
      </span>
      <span className="min-w-0 flex-1 truncate">{opts.name}</span>
      {opts.modified && (
        <span
          title="Modified from default"
          className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
        />
      )}
      {opts.hasOverrides && (
        <span className="shrink-0 rounded bg-brand/15 px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-brand">
          proj
        </span>
      )}
      {opts.chord && <ChordHint chord={opts.chord} className="shrink-0 text-[13px] text-ink-dim" />}
    </button>
  );

  const availableOverrideProjects = projects.filter(
    (p) => selPrompt && selPrompt.overrides?.[p.path] === undefined,
  );

  // ── Import / export ────────────────────────────────────────────────────────
  const chordLabel = (c: string) => formatChord(normalizeChord(c), IS_MACOS);
  const basename = (p: string) => p.split(/[\\/]/).pop() || p;
  const slugify = (name: string) =>
    name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "prompt";

  // Everything exportable, resolved to effective bodies/chords. Built-ins first,
  // then custom — the order the sidebar and the export dialog both show.
  const exportEntries: ExportEntry[] = useMemo(() => {
    const builtins = BUILTIN_PROMPTS.map((b) => ({
      id: b.id,
      name: b.name,
      builtin: true,
      body: effectiveBuiltinBody(asSettings, defaults, b.id),
      chord: effectiveChord(kbOverrides, b.commandId, IS_MACOS),
      overrides: {},
    }));
    const customs = custom.map((c) => ({
      id: c.id ?? "",
      name: c.name ?? "",
      builtin: false,
      body: c.body ?? "",
      chord: c.chord ? normalizeChord(c.chord) : null,
      overrides: c.overrides ?? {},
    }));
    return [...builtins, ...customs];
  }, [asSettings, defaults, kbOverrides, custom]);

  // The export dialog omits stock built-ins — an unmodified Commit/Ship is
  // identical to what every recipient already has, so it's just noise. Custom
  // prompts always list; a built-in only earns a row once it's been edited.
  // (The editor's own Export button still exports whatever prompt is open.)
  const exportListEntries = useMemo(
    () =>
      exportEntries.filter(
        (e) => !e.builtin || isBuiltinModified(asSettings, defaults, e.id as BuiltinPromptId),
      ),
    [exportEntries, asSettings, defaults],
  );

  // What an incoming file is matched against: existing names, bound chords, and
  // the project name→path map used to re-home overrides.
  const librarySnapshot: LibrarySnapshot = useMemo(() => {
    const takenChords = new Set<string>();
    for (const b of BUILTIN_PROMPTS) {
      const ch = effectiveChord(kbOverrides, b.commandId, IS_MACOS);
      if (ch) takenChords.add(ch);
    }
    for (const c of custom) if (c.chord) takenChords.add(normalizeChord(c.chord));
    const projectPathByName = new Map<string, string>();
    for (const p of projects) projectPathByName.set(p.mappedName, p.path);
    return {
      builtins: BUILTIN_PROMPTS.map((b) => ({ id: b.id, name: b.name })),
      custom,
      takenChords,
      projectPathByName,
    };
  }, [kbOverrides, custom, projects]);

  /** Write `content` to a user-chosen path, forcing the `.hlprompts` suffix.
   *  Returns the final basename on success, null if cancelled, or throws. */
  const writeToChosenPath = async (defaultName: string, content: string): Promise<string | null> => {
    const chosen = await saveFileDialog({
      title: "Export prompts",
      defaultPath: defaultName,
      filters: [{ name: "Hyprlayer prompts", extensions: [PROMPTS_FILE_EXT] }],
    });
    if (!chosen) return null;
    const path = chosen.endsWith(`.${PROMPTS_FILE_EXT}`) ? chosen : `${chosen}.${PROMPTS_FILE_EXT}`;
    const res = await commands.writePromptsFile(path, content);
    if (res.status === "error") {
      useToast.getState().show("Export failed", res.error.message || "Couldn't write the file.", {
        tone: "bad",
      });
      return null;
    }
    return basename(path);
  };

  const openImportDialog = async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      title: "Import prompts",
      filters: [{ name: "Hyprlayer prompts", extensions: [PROMPTS_FILE_EXT] }],
    });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return;
    const res = await commands.readPromptsFile(path);
    if (res.status === "error") {
      useToast.getState().show("Import failed", res.error.message || "Couldn't read that file.", {
        tone: "bad",
      });
      return;
    }
    const parsed = parsePromptsFile(res.data);
    if (!parsed.ok) {
      useToast.getState().show("Import failed", parsed.error, { tone: "bad" });
      return;
    }
    setImportFile({
      fileName: basename(path),
      source: parsed.file.source ?? null,
      candidates: planImport(parsed.file, librarySnapshot),
    });
  };

  const runImport = (decisions: Record<string, ImportDecision>) => {
    if (!importFile) return;
    const outcome = applyImport(
      importFile.candidates,
      decisions,
      promptsRef.current,
      librarySnapshot,
      newPromptId,
    );
    const from = importFile.fileName;
    setImportFile(null);
    if (!outcome.imported) return;
    update(() => outcome.prompts, { immediate: true });
    if (outcome.firstId) select(outcome.firstId);
    useToast.getState().show(
      "Prompts imported",
      `Imported ${outcome.imported} prompt${outcome.imported === 1 ? "" : "s"} from ${from}.`,
    );
  };

  const runExport = async (selected: ExportEntry[], opts: ExportOptions) => {
    const content = buildPromptsFile(selected, { ...opts, projectName });
    const name = await writeToChosenPath(`hyprlayer-prompts.${PROMPTS_FILE_EXT}`, content);
    if (name === null) return;
    setExportOpen(false);
    useToast.getState().show(
      "Prompts exported",
      `Exported ${selected.length} prompt${selected.length === 1 ? "" : "s"} → ${name}.`,
    );
  };

  // The editor's one-click export of the selected prompt — no options, carries
  // its chord and any overrides as-is.
  const exportSelected = async () => {
    const entry = exportEntries.find((e) => e.id === selectedId);
    if (!entry) return;
    const content = buildPromptsFile([entry], {
      includeChords: true,
      includeOverrides: true,
      projectName,
    });
    const name = await writeToChosenPath(`${slugify(entry.name)}.${PROMPTS_FILE_EXT}`, content);
    if (name === null) return;
    useToast.getState().show("Prompt exported", `Exported “${entry.name}” → ${name}.`);
  };

  return (
    <div className="flex h-full min-h-0">
      {/* ── "Prompts" master panel — the mock's sidebar, styled like the
             project/Library panels (h-9 header band, bg-panel, border seam). ── */}
      <aside
        className="flex shrink-0 flex-col border-r border-edge bg-panel"
        style={{ width: promptsWidth }}
      >
        <div className="flex h-9 shrink-0 items-center justify-between border-b border-edge pl-4 pr-3">
          <p className="text-xs font-medium uppercase tracking-wider text-ink-dim">Prompts</p>
          <div className="flex items-center gap-0.5">
            <button
              type="button"
              onClick={() => void openImportDialog()}
              className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
              title="Import prompts from a .hlprompts file"
              aria-label="Import prompts"
            >
              <ImportIcon size={14} />
            </button>
            <button
              type="button"
              onClick={() => setExportOpen(true)}
              className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
              title="Export prompts to share"
              aria-label="Export prompts"
            >
              <ExportIcon size={14} />
            </button>
            <button
              type="button"
              onClick={addPrompt}
              className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
              title="New prompt"
              aria-label="New prompt"
            >
              <PlusIcon size={14} />
            </button>
          </div>
        </div>
        <div className="px-3 pb-1 pt-3">
          <TextInput
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter…"
            aria-label="Filter prompts"
          />
        </div>
        <nav className="flex-1 space-y-4 overflow-y-auto px-2 pb-3 pt-2" aria-label="Prompts">
          <div>
            <div className="px-1.5 pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
              Built-in
            </div>
            <div className="space-y-0.5">
              {BUILTIN_PROMPTS.filter((b) =>
                matches(b.name, effectiveBuiltinBody(asSettings, defaults, b.id)),
              ).map((b) =>
                listRow({
                  key: b.id,
                  icon:
                    b.id === "commit" ? <GitCommitIcon size={14} /> : <GitPullRequestIcon size={14} />,
                  name: b.name,
                  chord: effectiveChord(kbOverrides, b.commandId, IS_MACOS),
                  modified: isBuiltinModified(asSettings, defaults, b.id),
                  selected: isBuiltin && builtinDef?.id === b.id,
                  onSelect: () => select(b.id),
                }),
              )}
            </div>
          </div>
          <div>
            <div className="px-1.5 pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
              Custom
            </div>
            <div className="space-y-0.5">
              {custom
                .filter((p) =>
                  matches(
                    p.name ?? "",
                    [p.body ?? "", ...Object.values(p.overrides ?? {})].join("\n"),
                  ),
                )
                .map((p) =>
                  listRow({
                    key: p.id ?? "",
                    icon: <ZapIcon size={14} />,
                    name: p.name ?? "",
                    chord: p.chord ? normalizeChord(p.chord) : null,
                    hasOverrides: Object.keys(p.overrides ?? {}).length > 0,
                    selected: selPrompt?.id === p.id,
                    onSelect: () => select(p.id ?? ""),
                  }),
                )}
              {custom.length === 0 && (
                <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-xs leading-relaxed text-ink-dim">
                  No custom prompts yet.
                </div>
              )}
            </div>
          </div>
        </nav>
        {/* Footer: where prompts can be invoked from — the mock's bottom band. */}
        <div className="shrink-0 border-t border-edge px-4 py-3">
          <p className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
            Invoke anywhere
          </p>
          <p className="mt-1 text-xs leading-relaxed text-ink-dim">
            Composer chips · the{" "}
            <ChordHint
              chord={effectiveChord(kbOverrides, "prompts.palette", IS_MACOS) ?? "mod+k"}
              className="rounded bg-edge/60 px-1 py-0.5"
            />{" "}
            palette · assigned shortcuts. Prompts run in the focused workstream.
          </p>
        </div>
      </aside>

      {/* Drag separator between the master panel and the editor. */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize prompts sidebar"
        className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
        onPointerDown={(e) =>
          startEdgeDrag(e, {
            startWidth: promptsWidth,
            min: PROMPTS_SIDEBAR_MIN,
            max: PROMPTS_SIDEBAR_MAX,
            direction: 1,
            onResize: setPromptsWidth,
          })
        }
      />

      {/* ── Editor ── */}
      <section className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl space-y-4 p-8">
          {save.isError && <ErrorNote error={save.error} />}
          <Card>
          <div className="flex items-center gap-2.5">
            <span className="shrink-0 text-accent">
              {builtinDef ? (
                builtinDef.id === "commit" ? (
                  <GitCommitIcon size={16} />
                ) : (
                  <GitPullRequestIcon size={16} />
                )
              ) : (
                <ZapIcon size={16} />
              )}
            </span>
            {builtinDef ? (
              <>
                <h2 className="text-sm font-semibold text-ink">{selName}</h2>
                <Badge tone="dim">Built-in</Badge>
              </>
            ) : (
              <TextInput
                value={selName}
                onChange={(e) =>
                  selPrompt && updateCustom(selPrompt.id!, (c) => ({ ...c, name: e.target.value }))
                }
                placeholder="Prompt name"
                aria-label="Prompt name"
                className="max-w-60 font-medium"
              />
            )}
            {selModified && <Badge tone="warn">Modified</Badge>}
            <div className="ml-auto flex shrink-0 items-center gap-2">
              <Button onClick={() => void exportSelected()} title="Save this prompt as a .hlprompts file">
                <ExportIcon size={13} />
                Export
              </Button>
              {builtinDef && selModified && (
                <Button onClick={resetBuiltin} title="Restore the stock prompt text">
                  Reset to default
                </Button>
              )}
              {selPrompt && (
                <Button variant="danger" onClick={() => void deleteSelected()}>
                  Delete
                </Button>
              )}
            </div>
          </div>

          {/* Scope */}
          <div className="mt-4 space-y-1.5">
            <div className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
              Scope
            </div>
            {builtinDef ? (
              <p className="text-xs text-ink-dim">
                Built-in prompts are global — they apply to every project and can't be scoped.
              </p>
            ) : (
              <>
                <div className="relative flex flex-wrap items-center gap-1.5">
                  <button
                    type="button"
                    onClick={() => {
                      setScope("global");
                      setProjPickerOpen(false);
                    }}
                    className={`rounded-md border px-2.5 py-1 text-xs font-medium ${
                      !isProjScope
                        ? "border-accent/40 bg-accent/15 text-accent"
                        : "border-edge bg-panel-2 text-ink-dim hover:text-ink"
                    }`}
                  >
                    Global
                  </button>
                  {Object.keys(selPrompt?.overrides ?? {}).map((path) => (
                    <button
                      key={path}
                      type="button"
                      onClick={() => {
                        setScope(path);
                        setProjPickerOpen(false);
                      }}
                      className={`rounded-md border px-2.5 py-1 text-xs font-medium ${
                        scope === path
                          ? "border-accent/40 bg-accent/15 text-accent"
                          : "border-edge bg-panel-2 text-ink-dim hover:text-ink"
                      }`}
                    >
                      {projectName(path)}
                    </button>
                  ))}
                  <button
                    type="button"
                    onClick={() => setProjPickerOpen((v) => !v)}
                    className="inline-flex items-center gap-1 rounded-md border border-dashed border-edge px-2 py-1 text-xs text-ink-dim hover:border-accent/50 hover:text-ink"
                  >
                    <PlusIcon size={11} />
                    Project override…
                  </button>
                  {projPickerOpen && (
                    <>
                      <button
                        type="button"
                        aria-label="Close"
                        className="fixed inset-0 z-20 cursor-default"
                        onClick={() => setProjPickerOpen(false)}
                      />
                      <div className="absolute left-0 top-8 z-40 w-60 rounded-md border border-edge bg-panel p-1 shadow-2xl">
                        <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                          Add override for…
                        </div>
                        {availableOverrideProjects.map((p) => (
                          <button
                            key={p.path}
                            type="button"
                            onClick={() => {
                              if (!selPrompt) return;
                              // Seed the override from the global body so editing
                              // starts from something, not a blank page.
                              updateCustom(
                                selPrompt.id!,
                                (c) => ({
                                  ...c,
                                  overrides: { ...(c.overrides ?? {}), [p.path]: c.body ?? "" },
                                }),
                                { immediate: true },
                              );
                              setScope(p.path);
                              setProjPickerOpen(false);
                            }}
                            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm text-ink hover:bg-edge/50"
                          >
                            {p.mappedName}
                          </button>
                        ))}
                        {availableOverrideProjects.length === 0 && (
                          <div className="px-2 py-1.5 text-xs text-ink-dim">
                            Every project already has an override.
                          </div>
                        )}
                      </div>
                    </>
                  )}
                </div>
                <p className="text-xs text-ink-dim">
                  {isProjScope
                    ? `Editing the override used only in ${projectName(scope)}. The global prompt stays untouched.`
                    : "Used by every project unless a project override exists."}
                </p>
              </>
            )}
          </div>

          {/* Body */}
          <div className="mt-4 space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                Prompt text
              </span>
              {selPrompt && isProjScope && hasScopeOverride && (
                <>
                  <span className="rounded bg-brand/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-brand">
                    Overrides global in {projectName(scope)}
                  </span>
                  <button
                    type="button"
                    onClick={() => {
                      if (!selPrompt) return;
                      const path = scope;
                      updateCustom(
                        selPrompt.id!,
                        (c) => {
                          const overrides = { ...(c.overrides ?? {}) };
                          delete overrides[path];
                          return { ...c, overrides };
                        },
                        { immediate: true },
                      );
                      setScope("global");
                    }}
                    className="text-[11px] text-ink-dim underline underline-offset-2 hover:text-bad"
                  >
                    Remove override
                  </button>
                </>
              )}
            </div>
            <textarea
              ref={bodyRef}
              value={selBody}
              onChange={(e) => writeBody(e.target.value)}
              onBlur={flushPending}
              rows={8}
              spellCheck={false}
              placeholder="What should the agent do when this prompt runs?"
              className="selectable min-h-40 w-full resize-y rounded-md border border-edge bg-panel-2 px-3 py-2.5 font-mono text-xs leading-relaxed text-ink focus:border-accent focus:outline-none"
            />
            <div className="relative flex items-center gap-2.5">
              <button
                type="button"
                onClick={() => setVarPickerOpen((v) => !v)}
                className="inline-flex items-center gap-1.5 rounded-md border border-edge bg-panel-2 px-2 py-1 text-xs text-brand hover:border-brand/60"
              >
                <BracesIcon size={12} />
                Insert variable
              </button>
              <span className="text-xs text-ink-dim">
                Variables resolve from the worktree when the prompt runs.
              </span>
              {varPickerOpen && (
                <>
                  <button
                    type="button"
                    aria-label="Close"
                    className="fixed inset-0 z-20 cursor-default"
                    onClick={() => setVarPickerOpen(false)}
                  />
                  <div className="absolute bottom-8 left-0 z-40 w-80 rounded-md border border-edge bg-panel p-1 shadow-2xl">
                    {PROMPT_VARS.map((v) => (
                      <button
                        key={v.token}
                        type="button"
                        onClick={() => insertVariable(v.token)}
                        className="flex w-full items-center gap-2.5 rounded px-2 py-1.5 text-left hover:bg-edge/50"
                      >
                        <code className="shrink-0 rounded bg-brand/15 px-1.5 py-0.5 text-[11px] text-brand">
                          {v.token}
                        </code>
                        <span className="min-w-0 flex-1 truncate text-xs text-ink-dim">
                          {v.desc}
                        </span>
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          </div>

          {/* Shortcut + availability/pin */}
          <div className="mt-4 grid grid-cols-2 gap-5 border-t border-edge/50 pt-4">
            <div className="space-y-1.5">
              <div className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                Keyboard shortcut
              </div>
              {recording ? (
                <div className="space-y-1.5">
                  <div className="flex items-center gap-2">
                    {candidate ? (
                      <ChordPill chord={candidate} />
                    ) : (
                      <span className="text-xs text-ink-dim">Press a shortcut…</span>
                    )}
                    <Button
                      variant="primary"
                      disabled={!candidate}
                      onClick={() => candidate && commitChord(candidate)}
                    >
                      Save
                    </Button>
                    <Button onClick={() => setRecording(false)}>Cancel</Button>
                  </div>
                  {conflictLabels.length > 0 && (
                    <p className="text-xs text-warn">
                      Already used by {conflictLabels.join(", ")}
                    </p>
                  )}
                  {reserved && (
                    <p className="text-xs text-warn">
                      This is a system shortcut — the OS may intercept it.
                    </p>
                  )}
                </div>
              ) : (
                <div className="flex items-center gap-2">
                  <ChordPill chord={selChord} />
                  <Button onClick={() => setRecording(true)}>Edit</Button>
                  <Button onClick={clearChord} disabled={selChord === null}>
                    Clear
                  </Button>
                </div>
              )}
              <p className="text-xs leading-relaxed text-ink-dim">
                Works anywhere in the app; runs in the focused workstream.
              </p>
            </div>
            <div className="space-y-1.5">
              <div className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                {builtinDef ? "Availability" : "Pin to composer"}
              </div>
              {builtinDef ? (
                <>
                  <p className="text-sm text-ink">Available in every project</p>
                  <p className="text-xs leading-relaxed text-ink-dim">
                    Built-ins are always at hand — run them from the git panel, the ⌘K palette, or
                    their shortcut.
                  </p>
                </>
              ) : (
                <>
                  <div className="flex items-center gap-2.5">
                    <Toggle
                      checked={Boolean(selPrompt?.pinned)}
                      onChange={(next) =>
                        selPrompt &&
                        updateCustom(selPrompt.id!, (c) => ({ ...c, pinned: next }), {
                          immediate: true,
                        })
                      }
                      aria-label="Pin to composer"
                    />
                    <span className="text-sm text-ink">
                      {selPrompt?.pinned ? "Shown in the composer bar" : "Hidden from the composer bar"}
                    </span>
                  </div>
                  <p className="text-xs leading-relaxed text-ink-dim">
                    Pinned prompts show as one-click chips above the message box in every session.
                  </p>
                </>
              )}
            </div>
          </div>
          </Card>
        </div>
      </section>

      {exportOpen && (
        <ExportPromptsDialog
          entries={exportListEntries}
          chordLabel={chordLabel}
          onExport={(sel, opts) => void runExport(sel, opts)}
          onClose={() => setExportOpen(false)}
        />
      )}
      {importFile && (
        <ImportPromptsDialog
          fileName={importFile.fileName}
          source={importFile.source}
          candidates={importFile.candidates}
          chordLabel={chordLabel}
          onConfirm={runImport}
          onChooseAnother={() => {
            setImportFile(null);
            void openImportDialog();
          }}
          onClose={() => setImportFile(null)}
        />
      )}
    </div>
  );
}
