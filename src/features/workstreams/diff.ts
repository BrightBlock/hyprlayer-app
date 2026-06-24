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

/** Tailwind classes per diff-line kind — shared by every diff view so additions,
 *  deletions, and hunk headers read the same everywhere. Additions/deletions/hunks
 *  bind the themed status tokens (ok/bad/brand) rather than fixed emerald/rose/sky
 *  shades, so a diff reads native to the active theme and legible on the light
 *  surface. */
export const PATCH_LINE_CLASSES: Record<PatchLineKind, string> = {
  add: "bg-ok/10 text-ok",
  del: "bg-bad/10 text-bad",
  hunk: "text-brand",
  meta: "text-ink-dim/70",
  context: "text-ink-dim",
};

/** Guard so a huge before/after pair can't blow up the O(n·m) LCS table; past it
 *  we fall back to a plain replace block (all deletions, then all additions). */
const LCS_CELL_LIMIT = 250_000;

/**
 * A line-level diff between two strings (an Edit tool's `old_string` /
 * `new_string`), as classified patch lines with the usual `+`/`-`/` ` prefixes —
 * so the same renderer that draws a git patch can draw an in-flight edit. Uses an
 * LCS to keep unchanged lines as shared context; the inputs are normally the
 * small surrounding snippet an Edit carries, not whole files.
 */
export function diffStrings(oldText: string, newText: string): PatchLine[] {
  const a = oldText.split("\n");
  const b = newText.split("\n");
  const n = a.length;
  const m = b.length;

  if (n * m > LCS_CELL_LIMIT) {
    return [
      ...a.map((text): PatchLine => ({ kind: "del", text: `-${text}` })),
      ...b.map((text): PatchLine => ({ kind: "add", text: `+${text}` })),
    ];
  }

  // dp[i][j] = length of the LCS of a[i:] and b[j:].
  const dp: Int32Array[] = Array.from({ length: n + 1 }, () => new Int32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const out: PatchLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "context", text: ` ${a[i]}` });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ kind: "del", text: `-${a[i]}` });
      i++;
    } else {
      out.push({ kind: "add", text: `+${b[j]}` });
      j++;
    }
  }
  while (i < n) out.push({ kind: "del", text: `-${a[i++]}` });
  while (j < m) out.push({ kind: "add", text: `+${b[j++]}` });
  return out;
}
