
export type PatchLineKind = "add" | "del" | "hunk" | "context";

export type PatchLine = { kind: PatchLineKind; text: string };

export const PATCH_LINE_CLASSES: Record<PatchLineKind, string> = {
  add: "bg-ok/10 text-diff-add",
  del: "bg-bad/10 text-diff-del",
  hunk: "text-brand",
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
