import type {
  PromptConflictAction,
  PromptConflictKind,
  PromptExportRequest,
  PromptImportCandidate,
  PromptImportPreview,
} from "@/lib/bindings";

export const PROMPTS_FILE_EXT = "hlprompts";
export const DEFAULT_EXPORT_NAME = `hyprlayer-prompts.${PROMPTS_FILE_EXT}`;

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

export function exportFileName(name: string): string {
  const slug =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "prompt";
  return `${slug}.${PROMPTS_FILE_EXT}`;
}

export function buildExportRequest(
  path: string,
  entries: ExportEntry[],
  opts: ExportOptions & { source?: string | null },
): PromptExportRequest {
  return {
    path,
    entries: entries.map((e) => ({
      name: e.name,
      body: e.body,
      chord: e.chord,
      overrides: e.overrides,
    })),
    includeChords: opts.includeChords,
    includeOverrides: opts.includeOverrides,
    source: opts.source ?? null,
  };
}

export type ImportRow = PromptImportCandidate & { conflict: PromptConflictKind | null };

export function importRows(preview: PromptImportPreview): ImportRow[] {
  const kinds = new Map(preview.conflicts.map((c) => [c.key, c.kind]));
  return preview.prompts.map((p) => ({ ...p, conflict: kinds.get(p.key) ?? null }));
}

export interface ImportDecision {
  include: boolean;
  resolution: PromptConflictAction;
}

export function conflictActions(
  rows: ImportRow[],
  decisions: Record<string, ImportDecision>,
): Record<string, PromptConflictAction> {
  const actions: Record<string, PromptConflictAction> = {};
  for (const row of rows) {
    const decision = decisions[row.key];
    actions[row.key] = !decision?.include
      ? "skip"
      : row.conflict
        ? decision.resolution
        : "keepBoth";
  }
  return actions;
}

export function willImportCount(
  rows: ImportRow[],
  decisions: Record<string, ImportDecision>,
): number {
  return Object.values(conflictActions(rows, decisions)).filter((a) => a !== "skip").length;
}
