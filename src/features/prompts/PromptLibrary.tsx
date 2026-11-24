import { useEffect, useMemo, useRef, useState } from "react";
import { open, save as saveFileDialog } from "@tauri-apps/plugin-dialog";

import { commands } from "@/lib/bindings";
import type { AppSettings, CustomPrompt, PromptSettings } from "@/lib/bindings";
import { invalidateBootstrap, useBootstrap, useSetAppSettings } from "@/lib/queries";
import { plural } from "@/lib/format";
import { IS_MACOS } from "@/lib/platform";
import { useToast } from "@/lib/toast";
import {
  Badge,
  Button,
  Card,
  ChordHint,
  ChordPill,
  ErrorNote,
  MenuItem,
  MenuSeparator,
  Popover,
  TextInput,
  Toggle,
  useConfirm,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
import {
  ListCardRow,
  ListHeader,
  NavFilterRow,
  NavGroupLabel,
  WorkshopFrame,
  WorkshopNav,
  type WorkshopChrome,
} from "@/components/workshop";
import {
  eventToChord,
  formatChord,
  normalizeChord,
  type Chord,
} from "@/lib/keybindings/chord";
import { commandById } from "@/lib/keybindings/commands";
import { conflictsFor, effectiveChord, type Overrides } from "@/lib/keybindings/resolve";
import { isReserved } from "@/lib/keybindings/reserved";
import {
  BUILTIN_PROMPTS,
  PROMPT_INPUT_TOKEN,
  PROMPT_VARS,
  PROMPT_FILTERS,
  effectiveBuiltinBody,
  filterCustomPrompts,
  isBuiltinModified,
  promptFirstLine,
  promptInputs,
  promptSlug,
  type BuiltinPromptId,
  type PromptFilter,
} from "./prompts";
import { ExportPromptsDialog } from "./ExportPromptsDialog";
import { ImportPromptsDialog } from "./ImportPromptsDialog";
import { PromptPreview } from "./PromptPreview";
import { PromptRunInfo } from "./PromptRunInfo";
import {
  DEFAULT_EXPORT_NAME,
  PROMPTS_FILE_EXT,
  buildExportRequest,
  conflictActions,
  exportFileName,
  importRows,
  type ExportEntry,
  type ExportOptions,
  type ImportDecision,
  type ImportRow,
} from "./promptsIo";

const EMPTY_OVERRIDES: Overrides = {};
const SAVE_DEBOUNCE_MS = 600;

function sanitizedForSave(prompts: PromptSettings): PromptSettings {
  const bodies = Object.fromEntries(
    Object.entries(prompts.builtinBodies ?? {}).filter(([, body]) => body.trim() !== ""),
  );
  return { ...prompts, builtinBodies: bodies };
}

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


type Selection =
  | { kind: "builtin"; id: BuiltinPromptId }
  | { kind: "custom"; prompt: CustomPrompt };

function newPromptId(): string {
  return typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `p-${Math.random().toString(36).slice(2)}`;
}

export function PromptLibrary({
  chrome,
  settings,
  selectedId,
  onSelect,
  filter,
  onFilter,
}: {
  chrome: WorkshopChrome;
  settings: AppSettings;
  selectedId: string;
  onSelect: (id: string) => void;
  filter: PromptFilter;
  onFilter: (filter: PromptFilter) => void;
}) {
  const save = useSetAppSettings();
  const bootstrap = useBootstrap();
  const defaults = bootstrap.data?.promptDefaults ?? null;
  const projects = useMemo(() => bootstrap.data?.projects ?? [], [bootstrap.data?.projects]);

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
  const flushPending = () => {
    if (timer.current !== null) persistNow();
  };
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

  const custom = useMemo(() => prompts.custom ?? [], [prompts.custom]);
  const [scope, setScope] = useState<string>("global");
  const [previewTarget, setPreviewTarget] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const [varPickerOpen, setVarPickerOpen] = useState(false);
  const varPickerTriggerRef = useRef<HTMLButtonElement>(null);
  const [projPickerOpen, setProjPickerOpen] = useState(false);
  const projPickerTriggerRef = useRef<HTMLButtonElement>(null);
  const [query, setQuery] = useState("");
  const [exportOpen, setExportOpen] = useState(false);
  const [confirmDialog, confirm] = useConfirm();
  const [importFile, setImportFile] = useState<{
    fileName: string;
    /** The file's own contents, carried back to `applyPromptImport`. */
    contents: string;
    sourceSha256: string;
    source: string | null;
    rows: ImportRow[];
  } | null>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);

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

  const isBuiltin = selection.kind === "builtin";
  const builtinDef = isBuiltin
    ? BUILTIN_PROMPTS.find((b) => b.id === selection.id)!
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
    setQuery("");
    onFilter("all");
    select(id);
  };

  const deleteSelected = async () => {
    if (!selPrompt) return;
    const ok = await confirm({
      title: `Delete “${selPrompt.name ?? "this prompt"}”?`,
      confirmLabel: "Delete",
      confirmVariant: "danger",
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

  const fq = query.trim().toLowerCase();
  const matches = (name: string, body: string) =>
    !fq || name.toLowerCase().includes(fq) || body.toLowerCase().includes(fq);

  const availableOverrideProjects = projects.filter(
    (p) => selPrompt && selPrompt.overrides?.[p.path] === undefined,
  );

  const chordLabel = (c: string) => formatChord(normalizeChord(c), IS_MACOS);
  const basename = (p: string) => p.split(/[\\/]/).pop() || p;

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

  const exportListEntries = useMemo(
    () =>
      exportEntries.filter(
        (e) => !e.builtin || isBuiltinModified(asSettings, defaults, e.id as BuiltinPromptId),
      ),
    [exportEntries, asSettings, defaults],
  );

  const takenChords = useMemo(() => {
    const taken = new Set<string>();
    for (const b of BUILTIN_PROMPTS) {
      const ch = effectiveChord(kbOverrides, b.commandId, IS_MACOS);
      if (ch) taken.add(ch);
    }
    for (const c of custom) if (c.chord) taken.add(normalizeChord(c.chord));
    return [...taken];
  }, [kbOverrides, custom]);

  const adopt = (next: PromptSettings) => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    promptsRef.current = next;
    setPrompts(next);
    invalidateBootstrap();
  };

  const exportTo = async (
    defaultName: string,
    entries: ExportEntry[],
    opts: ExportOptions,
  ): Promise<string | null> => {
    const chosen = await saveFileDialog({
      title: "Export prompts",
      defaultPath: defaultName,
      filters: [{ name: "Hyprlayer prompts", extensions: [PROMPTS_FILE_EXT] }],
    });
    if (!chosen) return null;
    const res = await commands.exportPrompts(buildExportRequest(entries, opts));
    if (res.status === "error") {
      useToast.getState().show("Export failed", res.error.message || "Couldn't write the file.", {
        tone: "bad",
      });
      return null;
    }
    const written = await commands.writePromptExport(chosen, res.data.content);
    if (written.status === "error") {
      useToast
        .getState()
        .show("Export failed", written.error.message || "Couldn't write the file.", {
          tone: "bad",
        });
      return null;
    }
    return basename(written.data);
  };

  const openImportDialog = async () => {
    flushPending();
    const selected = await open({
      multiple: false,
      directory: false,
      title: "Import prompts",
      filters: [{ name: "Hyprlayer prompts", extensions: [PROMPTS_FILE_EXT] }],
    });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return;
    const read = await commands.readPromptImport(path);
    if (read.status === "error") {
      useToast
        .getState()
        .show("Import failed", read.error.message || "Couldn't read that file.", {
          tone: "bad",
        });
      return;
    }
    const res = await commands.previewPromptImport(read.data, takenChords);
    if (res.status === "error") {
      useToast.getState().show("Import failed", res.error.message || "Couldn't read that file.", {
        tone: "bad",
      });
      return;
    }
    setImportFile({
      fileName: basename(path),
      contents: read.data,
      sourceSha256: res.data.sourceSha256,
      source: res.data.source,
      rows: importRows(res.data),
    });
  };

  const runImport = async (decisions: Record<string, ImportDecision>) => {
    if (!importFile) return;
    const from = importFile.fileName;
    const res = await commands.applyPromptImport({
      source: importFile.contents,
      expectedSourceSha256: importFile.sourceSha256,
      mode: "merge",
      conflictActions: conflictActions(importFile.rows, decisions),
      takenChords,
    });
    setImportFile(null);
    if (res.status === "error") {
      useToast.getState().show("Import failed", res.error.message || "Couldn't import that file.", {
        tone: "bad",
      });
      return;
    }
    const outcome = res.data;
    adopt(outcome.settings);
    const imported = outcome.added + outcome.replaced + outcome.renamed;
    if (!imported) return;
    if (outcome.firstId) select(outcome.firstId);
    useToast.getState().show(
      "Prompts imported",
      `Imported ${imported} prompt${imported === 1 ? "" : "s"} from ${from}.`,
    );
  };

  const runExport = async (selected: ExportEntry[], opts: ExportOptions) => {
    const name = await exportTo(DEFAULT_EXPORT_NAME, selected, opts);
    if (name === null) return;
    setExportOpen(false);
    useToast.getState().show(
      "Prompts exported",
      `Exported ${selected.length} prompt${selected.length === 1 ? "" : "s"} → ${name}.`,
    );
  };

  const exportSelected = async () => {
    const entry = exportEntries.find((e) => e.id === selectedId);
    if (!entry) return;
    const name = await exportTo(exportFileName(entry.name), [entry], {
      includeChords: true,
      includeOverrides: true,
    });
    if (name === null) return;
    useToast.getState().show("Prompt exported", `Exported “${entry.name}” → ${name}.`);
  };

  const builtinRows =
    filter === "all"
      ? BUILTIN_PROMPTS.filter((b) =>
          matches(b.name, effectiveBuiltinBody(asSettings, defaults, b.id)),
        )
      : [];
  const customRows = filterCustomPrompts(custom, filter).filter((p) =>
    matches(p.name ?? "", [p.body ?? "", ...Object.values(p.overrides ?? {})].join("\n")),
  );
  const filterLabel = PROMPT_FILTERS.find((f) => f.key === filter)?.label ?? "All prompts";

  return (
    <>
      <WorkshopFrame
        navKey={`prompts:${filter}`}
        nav={
          <WorkshopNav
            chrome={chrome}
            action={
              <Button variant="primary" className="w-full justify-center" onClick={addPrompt}>
                New prompt
              </Button>
            }
            filters={
              <>
                <div className="space-y-0.5">
                  {PROMPT_FILTERS.map((f) => (
                    <NavFilterRow
                      key={f.key}
                      label={f.label}
                      count={
                        f.key === "all"
                          ? BUILTIN_PROMPTS.length + custom.length
                          : filterCustomPrompts(custom, f.key).length
                      }
                      selected={filter === f.key}
                      onClick={() => onFilter(f.key)}
                    />
                  ))}
                </div>
              </>
            }
          />
        }
        list={
          <>
            <ListHeader
              actions={
                <>
                  <button
                    type="button"
                    onClick={() => void openImportDialog()}
                    className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
                    title="Import prompts from a .hlprompts file"
                    aria-label="Import prompts"
                  >
                    <Icon name="import" size="sm" />
                  </button>
                  <button
                    type="button"
                    onClick={() => setExportOpen(true)}
                    className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
                    title="Export prompts to share"
                    aria-label="Export prompts"
                  >
                    <Icon name="export" size="sm" />
                  </button>
                </>
              }
            >
              {`${filterLabel} · ${plural(builtinRows.length + customRows.length, "prompt")}`}
            </ListHeader>
            <div className="shrink-0 border-b border-edge p-3">
              <TextInput
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter…"
                aria-label="Filter prompts"
              />
            </div>
            <nav
              className="min-h-0 flex-1 space-y-4 overflow-y-auto px-2 pb-3 pt-2"
              aria-label="Prompts"
            >
              {filter === "all" && (
                <div>
                  <NavGroupLabel>Built-in</NavGroupLabel>
                  <div className="space-y-0.5">
                    {builtinRows.map((b) => {
                      const chord = effectiveChord(kbOverrides, b.commandId, IS_MACOS);
                      return (
                        <ListCardRow
                          key={b.id}
                          title={b.name}
                          badges={
                            <>
                              <Badge tone="dim">Built-in</Badge>
                              {isBuiltinModified(asSettings, defaults, b.id) && (
                                <Badge tone="warn">Modified</Badge>
                              )}
                              {chord && <ChordHint chord={chord} className="text-ui text-ink-dim" />}
                            </>
                          }
                          description={b.description}
                          meta={`${b.id} stage`}
                          selected={isBuiltin && builtinDef?.id === b.id}
                          onClick={() => select(b.id)}
                        />
                      );
                    })}
                  </div>
                </div>
              )}
              <div>
                <NavGroupLabel>Custom</NavGroupLabel>
                <div className="space-y-0.5">
                  {customRows.map((p) => {
                    const body = p.body ?? "";
                    const overrides = Object.keys(p.overrides ?? {}).length;
                    return (
                      <ListCardRow
                        key={p.id ?? ""}
                        title={p.name ?? ""}
                        badges={
                          (p.chord || p.pinned) && (
                            <>
                              {p.chord && (
                                <ChordHint
                                  chord={normalizeChord(p.chord)}
                                  className="text-ui text-ink-dim"
                                />
                              )}
                              {p.pinned && <Badge tone="dim">Pinned</Badge>}
                            </>
                          )
                        }
                        description={promptFirstLine(body)}
                        meta={`/${promptSlug(p.name ?? "")} · ${plural(promptInputs(body).length, "input")}${
                          overrides ? ` · ${plural(overrides, "override")}` : ""
                        }`}
                        selected={selPrompt?.id === p.id}
                        onClick={() => select(p.id ?? "")}
                      />
                    );
                  })}
                  {custom.length === 0 && (
                    <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
                      No custom prompts yet.
                    </div>
                  )}
                </div>
              </div>
            </nav>
          </>
        }
        detail={
          <div className="mx-auto max-w-5xl space-y-4 p-8">
            {save.isError && <ErrorNote error={save.error} onDismiss={() => save.reset()} />}
            <Card>
            <div className="flex items-center gap-2.5">
              <span className="shrink-0 text-accent">
                <Icon
                  name={builtinDef ? (builtinDef.id === "commit" ? "commit" : "pullRequest") : "prompt"}
                />
              </span>
              {builtinDef ? (
                <>
                  <h2 className="text-body font-semibold text-ink">{selName}</h2>
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
              {selPrompt && promptSlug(selName) && (
                <code
                  className="shrink-0 rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-ui-sm text-ink-dim"
                  title="Type this in a session composer to run the prompt. Text after it fills {{input}}."
                >
                  /{promptSlug(selName)}
                </code>
              )}
              {selModified && <Badge tone="warn">Modified</Badge>}
              <div className="ml-auto flex shrink-0 items-center gap-2">
                <Button onClick={() => void exportSelected()} title="Save this prompt as a .hlprompts file">
                  <Icon name="export" size="sm" />
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

            <div className="mt-4 space-y-1.5">
              <div className="text-label text-ink-dim">
                Scope
              </div>
              {builtinDef ? (
                <p className="text-ui text-ink-dim">
                  Built-in prompts are global and can't be scoped.
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
                      className={`rounded-md border px-2.5 py-1 text-ui font-medium ${
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
                        className={`rounded-md border px-2.5 py-1 text-ui font-medium ${
                          scope === path
                            ? "border-accent/40 bg-accent/15 text-accent"
                            : "border-edge bg-panel-2 text-ink-dim hover:text-ink"
                        }`}
                      >
                        {projectName(path)}
                      </button>
                    ))}
                    <button
                      ref={projPickerTriggerRef}
                      type="button"
                      onClick={() => setProjPickerOpen((v) => !v)}
                      className="inline-flex items-center gap-1 rounded-md border border-dashed border-edge px-2 py-1 text-ui text-ink-dim hover:border-accent/50 hover:text-ink"
                    >
                      <Icon name="add" size="xs" />
                      Project override…
                    </button>
                    <Popover
                      open={projPickerOpen}
                      onClose={() => setProjPickerOpen(false)}
                      anchorRef={projPickerTriggerRef}
                      className="min-w-44"
                    >
                      <div className="px-2 py-1 text-label text-ink-dim">
                        Add override for…
                      </div>
                      {availableOverrideProjects.map((p) => (
                        <MenuItem
                          key={p.path}
                          onClick={() => {
                            if (!selPrompt) return;
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
                        >
                          {p.mappedName}
                        </MenuItem>
                      ))}
                      {availableOverrideProjects.length === 0 && (
                        <div className="px-2 py-1.5 text-ui text-ink-dim">
                          Every project already has an override.
                        </div>
                      )}
                    </Popover>
                  </div>
                  <p className="text-ui text-ink-dim">
                    {isProjScope
                      ? `Editing the override used only in ${projectName(scope)}. The global prompt stays untouched.`
                      : "Used by every project unless a project override exists."}
                  </p>
                </>
              )}
            </div>

            <div className="mt-4 space-y-1.5">
              <div className="flex items-center gap-2">
                <span className="text-label text-ink-dim">
                  Prompt text
                </span>
                {selPrompt && isProjScope && hasScopeOverride && (
                  <>
                    <span className="rounded bg-brand/15 px-1.5 py-0.5 text-tag text-brand">
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
                      className="text-ui-sm text-ink-dim underline underline-offset-2 hover:text-bad"
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
                className="selectable min-h-40 w-full resize-y rounded-md border border-edge bg-panel-2 px-3 py-2.5 text-meta leading-prose text-ink focus:border-accent focus:outline-none"
              />
              <div className="flex items-center gap-2.5">
                <button
                  ref={varPickerTriggerRef}
                  type="button"
                  onClick={() => setVarPickerOpen((v) => !v)}
                  className="inline-flex items-center gap-1.5 rounded-md border border-edge bg-panel-2 px-2 py-1 text-ui text-brand hover:border-brand/60"
                >
                  <Icon name="braces" size="xs" />
                  Insert variable
                </button>
                <span className="text-ui text-ink-dim">
                  Variables resolve from the worktree; <code>{"{{input}}"}</code> asks you when
                  the prompt runs.
                </span>
                <Popover
                  open={varPickerOpen}
                  onClose={() => setVarPickerOpen(false)}
                  anchorRef={varPickerTriggerRef}
                  side="top"
                  className="min-w-44"
                >
                  {PROMPT_VARS.map((v) => (
                    <MenuItem key={v.token} onClick={() => insertVariable(v.token)}>
                      <span className="flex w-full items-center gap-2.5">
                        <code className="shrink-0 rounded bg-brand/15 px-1.5 py-0.5 text-ui-sm text-brand">
                          {v.token}
                        </code>
                        <span className="min-w-0 flex-1 truncate text-ui text-ink-dim">{v.desc}</span>
                      </span>
                    </MenuItem>
                  ))}
                  <MenuSeparator />
                  <MenuItem onClick={() => insertVariable(PROMPT_INPUT_TOKEN)}>
                    <span className="flex w-full items-center gap-2.5">
                      <code className="shrink-0 rounded bg-accent/15 px-1.5 py-0.5 text-ui-sm text-accent">
                        {PROMPT_INPUT_TOKEN}
                      </code>
                      <span className="min-w-0 flex-1 truncate text-ui text-ink-dim">
                        Ask me for a value when it runs
                      </span>
                    </span>
                  </MenuItem>
                  <p className="px-2.5 pb-1 pt-0.5 text-ui-sm leading-prose text-ink-dim/80">
                    Name the field with <code>{"{{input:which module}}"}</code>. Repeat a token to
                    fill it once and use it twice.
                  </p>
                </Popover>
              </div>
            </div>

            <div className="mt-4 grid grid-cols-2 gap-5 border-t border-edge/50 pt-4">
              <div className="space-y-1.5">
                <div className="text-label text-ink-dim">
                  Keyboard shortcut
                </div>
                {recording ? (
                  <div className="space-y-1.5">
                    <div className="flex items-center gap-2">
                      {candidate ? (
                        <ChordPill chord={candidate} unsetLabel="Not set" />
                      ) : (
                        <span className="text-ui text-ink-dim">Press a shortcut…</span>
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
                      <p className="text-ui text-warn-ink">
                        Already used by {conflictLabels.join(", ")}
                      </p>
                    )}
                    {reserved && (
                      <p className="text-ui text-warn-ink">
                        This is a system shortcut, so the OS may intercept it.
                      </p>
                    )}
                  </div>
                ) : (
                  <div className="flex items-center gap-2">
                    <ChordPill chord={selChord} unsetLabel="Not set" />
                    <Button onClick={() => setRecording(true)}>Edit</Button>
                    <Button onClick={clearChord} disabled={selChord === null}>
                      Clear
                    </Button>
                  </div>
                )}
                <p className="text-ui leading-prose text-ink-dim">
                  Works anywhere in the app; runs in the focused workstream.
                </p>
              </div>
              <div className="space-y-1.5">
                <div className="text-label text-ink-dim">
                  {builtinDef ? "Availability" : "Pin to composer"}
                </div>
                {builtinDef ? (
                  <p className="text-body text-ink">Available in every project</p>
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
                      <span className="text-body text-ink">
                        {selPrompt?.pinned ? "Shown in the composer bar" : "Hidden from the composer bar"}
                      </span>
                    </div>
                  </>
                )}
              </div>
            </div>
            </Card>
            <PromptPreview
              kind={selection.kind}
              builtinId={builtinDef?.id}
              customPrompt={selPrompt ?? undefined}
              builtinBody={selBody}
              stock={!selModified}
              scopeProject={isProjScope ? scope : null}
              projects={projects}
              target={previewTarget}
              onTarget={setPreviewTarget}
            />
            <PromptRunInfo
              body={selBody}
              builtinName={builtinDef?.name ?? null}
              slug={promptSlug(selName)}
            />
          </div>
        }
      />

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
          rows={importFile.rows}
          chordLabel={chordLabel}
          onConfirm={(decisions) => void runImport(decisions)}
          onChooseAnother={() => {
            setImportFile(null);
            void openImportDialog();
          }}
          onClose={() => setImportFile(null)}
        />
      )}
      {confirmDialog}
    </>
  );
}
