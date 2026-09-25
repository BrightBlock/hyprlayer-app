import type { WorkstreamActivity } from "@/features/session/sessionStore";

export { fuzzyScore } from "@/lib/fuzzy";

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
