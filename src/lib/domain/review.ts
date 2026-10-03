import type { FileChangeKind } from "@/lib/bindings";

export type { FileChangeKind };

export type ChangeGlyph = { ch: string; cls: string; label: string };

export const CHANGE_GLYPH: Record<FileChangeKind, ChangeGlyph> = {
  added: { ch: "+", cls: "text-ok", label: "added" },
  modified: { ch: "M", cls: "text-accent", label: "modified" },
  deleted: { ch: "−", cls: "text-bad", label: "deleted" },
  renamed: { ch: "R", cls: "text-warn-ink", label: "renamed" },
  typechange: { ch: "T", cls: "text-warn-ink", label: "typechange" },
  other: { ch: "?", cls: "text-ink-dim", label: "other" },
};

const CONFLICT_GLYPH: ChangeGlyph = { ch: "!", cls: "text-bad", label: "conflict" };

const CHANGE_KINDS = new Set<string>(Object.keys(CHANGE_GLYPH));

export function parseChangeKind(s: string): FileChangeKind {
  if (s === "untracked") return "added";
  return CHANGE_KINDS.has(s) ? (s as FileChangeKind) : "other";
}

// Takes git's free-form status strings, which include `untracked` and `conflict`.
export function glyphForStatus(change: string): ChangeGlyph {
  if (change === "conflict") return CONFLICT_GLYPH;
  const glyph = CHANGE_GLYPH[parseChangeKind(change)];
  return glyph.label === change ? glyph : { ...glyph, label: change };
}
