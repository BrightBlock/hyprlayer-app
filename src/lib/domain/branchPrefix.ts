import type { BranchPrefix } from "@/lib/bindings";

export type { BranchPrefix };

export function previewSlug(title: string): string {
  let kebab = "";
  let lastDash = true;
  for (const c of title) {
    if (/[a-zA-Z0-9]/.test(c)) {
      kebab += c.toLowerCase();
      lastDash = false;
    } else if (!lastDash) {
      kebab += "-";
      lastDash = true;
    }
    if (kebab.length >= 48) break;
  }
  const trimmed = kebab.replace(/^-+|-+$/g, "");
  return trimmed || "work-item";
}

export function suggestPrefix(title: string, list: BranchPrefix[]): BranchPrefix | null {
  const words = title
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
  for (const w of words) {
    const hit = list.find((p) => (p.keywords ?? []).some((k) => k.toLowerCase() === w));
    if (hit) return hit;
  }
  return null;
}

export function validateBranchPrefix(prefix: string): string | null {
  const trimmed = prefix.trim();
  if (!trimmed) return null;
  const m = INVALID_PREFIX.exec(trimmed);
  if (!m?.groups) return null;
  if (m.groups.leadSlash !== undefined) return "must not start with '/'";
  if (m.groups.leadDash !== undefined) return "must not start with '-'";
  if (m.groups.doubleSlash !== undefined) return "must not contain '//'";
  if (m.groups.dotDot !== undefined) return "must not contain '..'";
  if (m.groups.reflog !== undefined) return "must not contain '@{'";
  if (m.groups.wsCtl !== undefined) return "must not contain whitespace or control characters";
  if (m.groups.forbidden !== undefined) return `must not contain '${m.groups.forbidden}'`;
  if (m.groups.dotSegment !== undefined) return "a path segment must not start with '.'";
  return "a path segment must not end in '.lock'";
}

// eslint-disable-next-line no-control-regex
const INVALID_PREFIX = /(?<leadSlash>^\/)|(?<leadDash>^-)|(?<doubleSlash>\/\/)|(?<dotDot>\.\.)|(?<reflog>@\{)|(?<wsCtl>[\x00-\x20\x7f])|(?<forbidden>[~^:?*[\\])|(?<dotSegment>(?:^|\/)\.)|(?<lockSegment>\.lock\/)/;

export function validatePrefixList(
  rows: { label: string; prefix: string }[],
  skipBlank: ReadonlySet<number> = new Set(),
): Map<number, { label?: string; prefix?: string }> {
  const errors = new Map<number, { label?: string; prefix?: string }>();
  const put = (i: number, patch: { label?: string; prefix?: string }) =>
    errors.set(i, { ...errors.get(i), ...patch });
  const seen = new Map<string, number>();
  rows.forEach((row, i) => {
    const label = row.label.trim();
    const prefix = row.prefix.trim();
    const blankExempt = skipBlank.has(i);
    if (!label && !blankExempt) put(i, { label: "Every branch type needs a label." });
    if (!prefix) {
      if (!blankExempt) put(i, { prefix: "This branch type needs a prefix." });
      return;
    }
    const why = validateBranchPrefix(prefix);
    if (why) {
      put(i, { prefix: `Prefix ${why}.` });
      return;
    }
    const first = seen.get(prefix);
    if (first !== undefined) put(i, { prefix: "Another branch type already uses this prefix." });
    else seen.set(prefix, i);
  });
  return errors;
}
