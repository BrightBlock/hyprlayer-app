import type { DocMeta } from "@/lib/bindings";

export type DocGroup = {
  /** Lowercase `fm.type`, or "" for untyped docs. */
  type: string;
  /** Header label, e.g. "RESEARCH" (or "UNTYPED" for the typeless bucket). */
  label: string;
  /** Newest `mtimeMs` in the group, for the relative-age header; null if none. */
  newestMs: number | null;
  docs: DocMeta[];
};

/**
 * Bucket library docs by frontmatter type for the grouped list. Groups follow
 * first-seen order, i.e. the query's existing sort (date/relevance), so the
 * freshest type leads; docs keep their incoming order within a group.
 */
export function groupDocsByType(docs: DocMeta[]): DocGroup[] {
  const byType = new Map<string, DocMeta[]>();
  for (const doc of docs) {
    const key = (doc.fm.type ?? "").trim().toLowerCase();
    const list = byType.get(key);
    if (list) list.push(doc);
    else byType.set(key, [doc]);
  }

  const groups: DocGroup[] = [];
  for (const [type, items] of byType) {
    const newestMs = items.reduce<number | null>(
      (max, d) => (d.mtimeMs != null && (max == null || d.mtimeMs > max) ? d.mtimeMs : max),
      null,
    );
    groups.push({ type, label: type ? type.toUpperCase() : "UNTYPED", newestMs, docs: items });
  }
  return groups;
}
