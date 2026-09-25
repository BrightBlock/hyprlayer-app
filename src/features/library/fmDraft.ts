import type { FmPatchDto, FmSetEntry, Frontmatter } from "@/lib/bindings";

// Order mirrors core's SCHEMA_KEYS; both checks tie the list to the Frontmatter binding.
export const SCHEMA_KEYS = [
  "title",
  "type",
  "date",
  "status",
  "ticket",
  "project",
  "scope",
  "tags",
  "author",
  "related",
] as const satisfies readonly Exclude<keyof Frontmatter, "extra">[];

export type SchemaKey = (typeof SCHEMA_KEYS)[number];

// Fails to compile if Frontmatter grows a field SCHEMA_KEYS doesn't list.
const _complete: Exclude<Exclude<keyof Frontmatter, "extra">, SchemaKey> extends never ? true : never = true;
void _complete;

export type DraftValue =
  | { kind: "str"; value: string }
  | { kind: "list"; values: string[] }
  | { kind: "remove" };

export type FmDraft = Readonly<Partial<Record<SchemaKey, DraftValue>>>;

const LIST_KEYS: ReadonlySet<SchemaKey> = new Set(["tags", "related"]);

function isListKey(key: SchemaKey): boolean {
  return LIST_KEYS.has(key);
}

function withoutKey(draft: FmDraft, key: SchemaKey): FmDraft {
  const next = { ...draft };
  delete next[key];
  return next;
}

// Only a key actually present in the file counts as a file value; the indexer's
// path-derived title/type/date never do (see `placeholder`).
export function fileValue(fm: Frontmatter, fmKeys: string[], key: SchemaKey): string | string[] {
  if (!fmKeys.includes(key)) return isListKey(key) ? [] : "";
  const value = fm[key];
  if (isListKey(key)) return (value as string[] | undefined) ?? [];
  return (value as string | undefined) ?? "";
}

// The path-derived value for a key absent from the file, shown as placeholder text only.
export function placeholder(fm: Frontmatter, fmKeys: string[], key: SchemaKey): string | undefined {
  if (fmKeys.includes(key)) return undefined;
  const value = fm[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

export function fieldValue(
  fm: Frontmatter,
  fmKeys: string[],
  draft: FmDraft,
  key: SchemaKey,
): string | string[] {
  const entry = draft[key];
  if (!entry) return fileValue(fm, fmKeys, key);
  if (entry.kind === "remove") return isListKey(key) ? [] : "";
  return entry.kind === "list" ? entry.values : entry.value;
}

function sameValue(a: string | string[], b: string | string[]): boolean {
  if (Array.isArray(a) || Array.isArray(b)) {
    const x = Array.isArray(a) ? a : [];
    const y = Array.isArray(b) ? b : [];
    return x.length === y.length && x.every((v, i) => v === y[i]);
  }
  return a === b;
}

export function setField(
  draft: FmDraft,
  fm: Frontmatter,
  fmKeys: string[],
  key: SchemaKey,
  next: string | string[],
): FmDraft {
  if (sameValue(fileValue(fm, fmKeys, key), next)) return withoutKey(draft, key);

  const empty = Array.isArray(next) ? next.length === 0 : next.trim().length === 0;
  if (empty) {
    if (!fmKeys.includes(key)) return withoutKey(draft, key);
    return { ...draft, [key]: { kind: "remove" } };
  }
  return {
    ...draft,
    [key]: Array.isArray(next) ? { kind: "list", values: next } : { kind: "str", value: next },
  };
}

export function toPatch(draft: FmDraft): FmPatchDto {
  const set: FmSetEntry[] = [];
  const remove: string[] = [];
  for (const [key, value] of Object.entries(draft) as [SchemaKey, DraftValue][]) {
    if (value.kind === "remove") remove.push(key);
    else if (value.kind === "list") set.push({ key, value: { kind: "list", values: value.values } });
    else set.push({ key, value: { kind: "str", value: value.value } });
  }
  return { set, remove };
}

export function presentKeys(fmKeys: string[], draft: FmDraft): number {
  const keys = new Set(fmKeys);
  for (const [key, value] of Object.entries(draft) as [SchemaKey, DraftValue][]) {
    if (value.kind === "remove") keys.delete(key);
    else keys.add(key);
  }
  return keys.size;
}
