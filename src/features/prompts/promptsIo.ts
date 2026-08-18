/**
 * Pure core behind the `.hlprompts` file format (a versioned JSON bundle of
 * prompt entries) and the merge that folds an import into settings. On import
 * every entry becomes a custom prompt unless its name collides, which the user
 * resolves (keep-both/replace/skip). Overrides travel keyed by project NAME,
 * not path. They only re-attach where a same-named project exists, else drop.
 * File I/O lives in Rust (`read_prompts_file`/`write_prompts_file`); this is
 * pure and unit-tested. Chords are the frontend's canonical form.
 */

import type { CustomPrompt, PromptSettings } from "@/lib/bindings";
import { normalizeChord } from "@/lib/keybindings/chord";
import type { BuiltinPromptId } from "./prompts";

/** Current on-disk format version. Bump when the shape changes incompatibly. */
export const PROMPTS_FILE_VERSION = 1;
/** The file extension (no dot) used by the open/save dialogs and Rust guard. */
export const PROMPTS_FILE_EXT = "hlprompts";

/** One prompt as it travels inside a `.hlprompts` file. */
export interface PromptFileEntry {
  name: string;
  body: string;
  /** Canonical chord; absent = unbound or shortcuts were excluded on export. */
  chord?: string;
  /** Per-project body overrides, keyed by project **name** (not path). */
  overrides?: Record<string, string>;
}

/** The `.hlprompts` document. */
export interface PromptsFile {
  hlprompts: number;
  /** Free-text attribution shown in the import review ("from …"); optional. */
  source?: string | null;
  prompts: PromptFileEntry[];
}

// ── Export ──────────────────────────────────────────────────────────────────

/** A library prompt gathered for export (built-in or custom), already resolved
 *  to its effective body and chord. `overrides` is keyed by project **path**
 *  (as stored); {@link buildPromptsFile} maps those to names on the way out. */
export interface ExportEntry {
  /** Stable selection id: a built-in id (`"commit"`) or a custom prompt id. */
  id: string;
  name: string;
  builtin: boolean;
  body: string;
  chord: string | null;
  overrides: Record<string, string>;
}

export interface ExportOptions {
  includeChords: boolean;
  includeOverrides: boolean;
}

/** Serialize the selected entries into `.hlprompts` JSON text. `projectName`
 *  maps a stored project path to its display name (overrides travel by name so
 *  they can re-attach on a different machine). */
export function buildPromptsFile(
  entries: ExportEntry[],
  opts: ExportOptions & {
    source?: string | null;
    projectName: (path: string) => string;
  },
): string {
  const prompts: PromptFileEntry[] = entries.map((e) => {
    const entry: PromptFileEntry = { name: e.name, body: e.body };
    if (opts.includeChords && e.chord) entry.chord = e.chord;
    if (opts.includeOverrides) {
      const byName: Record<string, string> = {};
      for (const [path, body] of Object.entries(e.overrides)) {
        byName[opts.projectName(path)] = body;
      }
      if (Object.keys(byName).length > 0) entry.overrides = byName;
    }
    return entry;
  });
  const doc: PromptsFile = {
    hlprompts: PROMPTS_FILE_VERSION,
    ...(opts.source ? { source: opts.source } : {}),
    prompts,
  };
  return JSON.stringify(doc, null, 2) + "\n";
}

// ── Parse ───────────────────────────────────────────────────────────────────

export type ParseResult =
  | { ok: true; file: PromptsFile }
  | { ok: false; error: string };

/** Parse and validate `.hlprompts` text. Tolerant of extra fields and sparse
 *  entries; rejects non-JSON, the wrong shape, a newer format version, or a
 *  bundle with no usable (named) prompts. */
export function parsePromptsFile(text: string): ParseResult {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return {
      ok: false,
      error: "This file isn't valid JSON. It may be corrupted or not a prompt export.",
    };
  }
  if (typeof raw !== "object" || raw === null) {
    return { ok: false, error: "This doesn't look like a Hyprlayer prompt file." };
  }
  const obj = raw as Record<string, unknown>;
  if (!Array.isArray(obj.prompts)) {
    return { ok: false, error: "This doesn't look like a Hyprlayer prompt file." };
  }
  if (typeof obj.hlprompts === "number" && obj.hlprompts > PROMPTS_FILE_VERSION) {
    return {
      ok: false,
      error: "This file was exported by a newer version of Hyprlayer. Update to import it.",
    };
  }
  const prompts: PromptFileEntry[] = [];
  for (const item of obj.prompts) {
    if (typeof item !== "object" || item === null) continue;
    const p = item as Record<string, unknown>;
    const name = typeof p.name === "string" ? p.name.trim() : "";
    if (!name) continue; // unnamed prompts can't be listed or name-matched
    const entry: PromptFileEntry = {
      name,
      body: typeof p.body === "string" ? p.body : "",
    };
    if (typeof p.chord === "string" && p.chord.trim()) entry.chord = p.chord.trim();
    if (p.overrides && typeof p.overrides === "object") {
      const overrides: Record<string, string> = {};
      for (const [proj, body] of Object.entries(p.overrides as Record<string, unknown>)) {
        if (typeof body === "string") overrides[proj] = body;
      }
      if (Object.keys(overrides).length > 0) entry.overrides = overrides;
    }
    prompts.push(entry);
  }
  if (prompts.length === 0) {
    return { ok: false, error: "This prompt file has no prompts to import." };
  }
  return {
    ok: true,
    file: {
      hlprompts: typeof obj.hlprompts === "number" ? obj.hlprompts : PROMPTS_FILE_VERSION,
      source: typeof obj.source === "string" ? obj.source : null,
      prompts,
    },
  };
}

// ── Import planning + merge ───────────────────────────────────────────────────

export type ImportResolution = "keep-both" | "replace" | "skip";

/** A planned import row: the file entry plus how it relates to this library. */
export interface ImportCandidate {
  /** Stable key (the file index) for check/decision maps. */
  key: string;
  name: string;
  body: string;
  /** Canonical chord as it appears in the file (may be dropped on apply). */
  chord: string | null;
  /** Overrides from the file, keyed by project **name**. */
  overridesByName: Record<string, string>;
  /** An existing prompt of the same name, if any. */
  conflict: { kind: "builtin" | "custom" } | null;
  /** True when {@link chord} is already bound in this library. */
  chordTaken: boolean;
  /** Override project names with no matching project here (dropped on import). */
  unknownProjects: string[];
}

/** Everything {@link planImport}/{@link applyImport} need to know about the
 *  current library, gathered by the caller (which owns keybinding resolution). */
export interface LibrarySnapshot {
  builtins: { id: BuiltinPromptId; name: string }[];
  /** Current custom prompts (the merge base). */
  custom: CustomPrompt[];
  /** Canonical chords already bound (built-in effective chords + custom). */
  takenChords: Set<string>;
  /** Known project names → their stored path. */
  projectPathByName: Map<string, string>;
}

const normChord = (c: string): string => normalizeChord(c);

/** Classify each file entry against the current library: name collision,
 *  chord availability, and which overrides have no home here. */
export function planImport(file: PromptsFile, snap: LibrarySnapshot): ImportCandidate[] {
  const builtinNames = new Set(snap.builtins.map((b) => b.name));
  const customNames = new Set(snap.custom.map((c) => (c.name ?? "").trim()));
  return file.prompts.map((p, i) => {
    const conflict: ImportCandidate["conflict"] = builtinNames.has(p.name)
      ? { kind: "builtin" }
      : customNames.has(p.name)
        ? { kind: "custom" }
        : null;
    const chord = p.chord ?? null;
    return {
      key: String(i),
      name: p.name,
      body: p.body,
      chord,
      overridesByName: p.overrides ?? {},
      conflict,
      chordTaken: chord !== null && snap.takenChords.has(normChord(chord)),
      unknownProjects: Object.keys(p.overrides ?? {}).filter(
        (name) => !snap.projectPathByName.has(name),
      ),
    };
  });
}

export interface ImportDecision {
  include: boolean;
  /** Only consulted when the candidate has a name conflict. */
  resolution: ImportResolution;
}

export interface ImportOutcome {
  prompts: PromptSettings;
  imported: number;
  /** Prompt/built-in id to select after import (first added/updated), or null. */
  firstId: string | null;
}

/** Fold the chosen candidates into a new {@link PromptSettings}. Pure, the
 *  caller persists the result. `newId` mints ids for added prompts (injectable
 *  so tests are deterministic). Rules mirror the design mock:
 *  - no conflict → add as a new custom prompt;
 *  - conflict + keep-both → add as a new custom prompt, name suffixed;
 *  - conflict + replace → overwrite the matched prompt's body (a built-in
 *    updates its `builtinBodies` entry; a custom updates in place);
 *  - conflict + skip → drop it.
 *  A chord is only assigned when free (checked against the library *and* chords
 *  claimed earlier in this same import); otherwise the prompt lands unbound. */
export function applyImport(
  candidates: ImportCandidate[],
  decisions: Record<string, ImportDecision>,
  base: PromptSettings,
  snap: LibrarySnapshot,
  newId: () => string,
): ImportOutcome {
  let builtinBodies = { ...(base.builtinBodies ?? {}) };
  let custom = (base.custom ?? []).map((c) => ({ ...c }));
  const taken = new Set(snap.takenChords);
  const builtinIdByName = new Map(snap.builtins.map((b) => [b.name, b.id] as const));
  let imported = 0;
  let firstId: string | null = null;

  /** Claim `chord` if it's free (or already this prompt's own); else null. */
  const claimChord = (chord: string | null, ownChord: string | null): string | null => {
    if (!chord) return ownChord ?? null;
    const canon = normChord(chord);
    if (!taken.has(canon) || canon === ownChord) {
      taken.add(canon);
      return canon;
    }
    return ownChord ?? null;
  };

  for (const c of candidates) {
    const decision = decisions[c.key];
    if (!decision?.include) continue;
    const resolution: ImportResolution = c.conflict ? decision.resolution : "keep-both";
    if (c.conflict && resolution === "skip") continue;

    // Resolve overrides (project name → stored path), dropping unknowns.
    const overrides: Record<string, string> = {};
    for (const [name, body] of Object.entries(c.overridesByName)) {
      const path = snap.projectPathByName.get(name);
      if (path) overrides[path] = body;
    }

    if (c.conflict && resolution === "replace") {
      if (c.conflict.kind === "builtin") {
        // A built-in's shortcut lives in keybindings, not here, replace only
        // its body (its `builtinBodies` override).
        const id = builtinIdByName.get(c.name);
        if (id) {
          builtinBodies = { ...builtinBodies, [id]: c.body };
          if (!firstId) firstId = id;
          imported++;
        }
        continue;
      }
      const idx = custom.findIndex((p) => (p.name ?? "").trim() === c.name);
      if (idx >= 0) {
        const existing = custom[idx];
        const ownChord = existing.chord ? normChord(existing.chord) : null;
        custom[idx] = {
          ...existing,
          body: c.body,
          overrides: { ...(existing.overrides ?? {}), ...overrides },
          chord: claimChord(c.chord, ownChord),
        };
        if (!firstId) firstId = existing.id ?? null;
        imported++;
      }
      continue;
    }

    // Add a new custom prompt. A name collision under keep-both gets a suffix.
    const id = newId();
    custom = [
      ...custom,
      {
        id,
        name: c.conflict ? `${c.name} (imported)` : c.name,
        body: c.body,
        overrides,
        chord: claimChord(c.chord, null),
        pinned: false,
      },
    ];
    if (!firstId) firstId = id;
    imported++;
  }

  return { prompts: { builtinBodies, custom }, imported, firstId };
}
