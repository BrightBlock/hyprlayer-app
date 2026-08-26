import type { DocMeta } from "@/lib/bindings";

export type DocGroup = {
  type: string;
  label: string;
  newestMs: number | null;
  docs: DocMeta[];
};

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
