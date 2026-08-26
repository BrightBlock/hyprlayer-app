
export type PatchLineKind = "add" | "del" | "hunk" | "meta" | "context";

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

export function parsePatchLines(patch: string): PatchLine[] {
  const lines = patch.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((text) => ({ kind: classifyPatchLine(text), text }));
}

export const PATCH_LINE_CLASSES: Record<PatchLineKind, string> = {
  add: "bg-ok/10 text-ok",
  del: "bg-bad/10 text-bad",
  hunk: "text-brand",
  meta: "text-ink-dim/70",
  context: "text-ink-dim",
};

const LCS_CELL_LIMIT = 250_000;

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

export function intraLineRanges(text: string, from: number, to: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let pos = from; pos < to; ) {
    const lineStart = text.lastIndexOf("\n", pos - 1) + 1;
    const nl = text.indexOf("\n", pos);
    const lineEnd = nl === -1 ? text.length : nl;
    const segFrom = Math.max(pos, lineStart);
    const segTo = Math.min(to, lineEnd);
    if (segFrom < segTo && !(segFrom === lineStart && segTo === lineEnd)) {
      out.push([segFrom, segTo]);
    }
    pos = lineEnd + 1;
  }
  return out;
}
