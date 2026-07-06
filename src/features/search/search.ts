import type { WorkstreamActivity } from "@/features/session/sessionStore";

export type SearchResultKind = "workstream" | "agent" | "doc" | "file";

/** Where a result sends you; dispatched centrally by the palette. */
export type NavTarget =
  | { type: "workstream"; projectPath: string; workstreamId: string }
  | { type: "agent"; projectPath: string; workstreamId: string; sessionId: string }
  | { type: "doc"; id: string }
  | { type: "file"; workstreamId: string; path: string };

export type SearchResult = {
  kind: SearchResultKind;
  /** Stable React key + dedupe id. */
  key: string;
  title: string;
  subtitle?: string;
  /** Higher = better, within its source. */
  score: number;
  nav: NavTarget;
  /** Agents only: drives the activity badge. */
  activity?: WorkstreamActivity;
  /** Trailing tag (doc type, file ext, stage…). */
  tag?: string;
};

export type SearchGroup = { kind: SearchResultKind; label: string; results: SearchResult[] };

const GROUP_ORDER: { kind: SearchResultKind; label: string }[] = [
  { kind: "workstream", label: "Workstreams" },
  { kind: "agent", label: "Agents" },
  { kind: "file", label: "Files" },
  { kind: "doc", label: "Documents" },
];

/** Case-insensitive subsequence score, or null when `q` isn't a subsequence of
 *  `text`. Rewards contiguous runs, word-boundary starts, and a full prefix;
 *  shorter targets win ties. Empty `q` returns 0 (keep caller's order). */
export function fuzzyScore(text: string, q: string): number | null {
  if (!q) return 0;
  const t = text.toLowerCase();
  const needle = q.toLowerCase();
  let ti = 0;
  let score = 0;
  let run = 0;
  let matched = 0;
  for (let ni = 0; ni < needle.length; ni++) {
    const ch = needle[ni];
    let found = -1;
    for (let i = ti; i < t.length; i++) {
      if (t[i] === ch) {
        found = i;
        break;
      }
    }
    if (found === -1) return null; // not a subsequence
    matched++;
    run = found === ti ? run + 1 : 0;
    score += 1 + run * 2; // contiguity bonus
    if (found === 0 || /\W|_/.test(t[found - 1])) score += 3; // word-boundary bonus
    ti = found + 1;
  }
  if (t.startsWith(needle)) score += 5; // prefix bonus
  score -= t.length * 0.01; // prefer shorter targets on ties
  return matched === needle.length ? score : null;
}

/** Order + label non-empty groups (each sorted by score desc, capped). */
export function groupResults(results: SearchResult[], perGroup = 8): SearchGroup[] {
  return GROUP_ORDER.map(({ kind, label }) => ({
    kind,
    label,
    results: results
      .filter((r) => r.kind === kind)
      .sort((a, b) => b.score - a.score)
      .slice(0, perGroup),
  })).filter((g) => g.results.length > 0);
}

/** Flatten groups (in display order) for the single keyboard cursor. */
export function flattenGroups(groups: SearchGroup[]): SearchResult[] {
  return groups.flatMap((g) => g.results);
}
