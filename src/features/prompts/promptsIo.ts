
import type { CustomPrompt, PromptSettings } from "@/lib/bindings";
import { normalizeChord } from "@/lib/keybindings/chord";
import type { BuiltinPromptId } from "./prompts";

export const PROMPTS_FILE_VERSION = 1;
export const PROMPTS_FILE_EXT = "hlprompts";

export interface PromptFileEntry {
  name: string;
  body: string;
  chord?: string;
  overrides?: Record<string, string>;
}

export interface PromptsFile {
  hlprompts: number;
  source?: string | null;
  prompts: PromptFileEntry[];
}


export interface ExportEntry {
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


export type ParseResult =
  | { ok: true; file: PromptsFile }
  | { ok: false; error: string };

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
    if (!name) continue;
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


export type ImportResolution = "keep-both" | "replace" | "skip";

export interface ImportCandidate {
  key: string;
  name: string;
  body: string;
  chord: string | null;
  overridesByName: Record<string, string>;
  conflict: { kind: "builtin" | "custom" } | null;
  chordTaken: boolean;
  unknownProjects: string[];
}

export interface LibrarySnapshot {
  builtins: { id: BuiltinPromptId; name: string }[];
  custom: CustomPrompt[];
  takenChords: Set<string>;
  projectPathByName: Map<string, string>;
}

const normChord = (c: string): string => normalizeChord(c);

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
  resolution: ImportResolution;
}

export interface ImportOutcome {
  prompts: PromptSettings;
  imported: number;
  firstId: string | null;
}

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

    const overrides: Record<string, string> = {};
    for (const [name, body] of Object.entries(c.overridesByName)) {
      const path = snap.projectPathByName.get(name);
      if (path) overrides[path] = body;
    }

    if (c.conflict && resolution === "replace") {
      if (c.conflict.kind === "builtin") {
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
