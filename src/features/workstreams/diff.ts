/** Pure helpers for rendering unified-diff patch text. No React, no IPC. */

export type PatchLineKind = "add" | "del" | "hunk" | "meta" | "context";

/**
 * Classify one line of unified-diff output for display.
 * File headers (`+++`/`---`, `diff --git`, `index …`) are "meta", hunk
 * headers (`@@ … @@`) are "hunk"; only real content lines count as add/del.
 */
export function classifyPatchLine(line: string): PatchLineKind {
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+++") || line.startsWith("---")) return "meta";
  if (
    line.startsWith("diff ") ||
    line.startsWith("index ") ||
    line.startsWith("new file mode") ||
    line.startsWith("deleted file mode") ||
    line.startsWith("old mode") ||
    line.startsWith("new mode") ||
    line.startsWith("similarity index") ||
    line.startsWith("rename from") ||
    line.startsWith("rename to") ||
    line.startsWith("Binary files") ||
    line.startsWith("\\ No newline")
  ) {
    return "meta";
  }
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "context";
}

export type PatchStats = { additions: number; deletions: number };

/** Count added/deleted content lines in a unified diff (headers excluded). */
export function countPatchStats(patch: string): PatchStats {
  let additions = 0;
  let deletions = 0;
  for (const line of patch.split("\n")) {
    const kind = classifyPatchLine(line);
    if (kind === "add") additions += 1;
    else if (kind === "del") deletions += 1;
  }
  return { additions, deletions };
}

export type PatchLine = { kind: PatchLineKind; text: string };

/** Split patch text into classified lines, dropping a single trailing blank. */
export function parsePatchLines(patch: string): PatchLine[] {
  const lines = patch.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((text) => ({ kind: classifyPatchLine(text), text }));
}
