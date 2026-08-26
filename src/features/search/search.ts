import type { WorkstreamActivity } from "@/features/session/sessionStore";

export type SearchResultKind = "workstream" | "agent" | "doc" | "file";

export type NavTarget =
  | { type: "workstream"; projectPath: string; workstreamId: string }
  | { type: "agent"; projectPath: string; workstreamId: string; sessionId: string }
  | { type: "doc"; id: string }
  | { type: "file"; workstreamId: string; path: string };

export type SearchResult = {
  kind: SearchResultKind;
  key: string;
  title: string;
  subtitle?: string;
  score: number;
  nav: NavTarget;
  activity?: WorkstreamActivity;
  tag?: string;
};

export type SearchGroup = { kind: SearchResultKind; label: string; results: SearchResult[] };

const GROUP_ORDER: { kind: SearchResultKind; label: string }[] = [
  { kind: "workstream", label: "Workstreams" },
  { kind: "agent", label: "Agents" },
  { kind: "file", label: "Files" },
  { kind: "doc", label: "Documents" },
];

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
    if (found === -1) return null;
    matched++;
    run = found === ti ? run + 1 : 0;
    score += 1 + run * 2;
    if (found === 0 || /\W|_/.test(t[found - 1])) score += 3;
    ti = found + 1;
  }
  if (t.startsWith(needle)) score += 5;
  score -= t.length * 0.01;
  return matched === needle.length ? score : null;
}

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

export function flattenGroups(groups: SearchGroup[]): SearchResult[] {
  return groups.flatMap((g) => g.results);
}
