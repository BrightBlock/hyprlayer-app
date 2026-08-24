/**
 * Branch-type presentation helpers. The VOCABULARY is the backend's
 * (`BranchPrefix` is generated from `crates/hyprlayer-core/src/branch_prefix.rs`
 * via specta); only preview and matching live here.
 *
 * The matching ALGORITHM is deliberately on this side while the keyword DATA
 * stays on the backend's: a suggestion has to update on every keystroke, and an
 * IPC round-trip per character is not that.
 */
import type { BranchPrefix } from "@/lib/bindings";

export type { BranchPrefix };

/**
 * The slug `unique_slug` will produce, minus its `-<4 hex>` suffix — the
 * preview cannot know a uuid the backend has not generated yet, so the caller
 * renders a fixed placeholder after this.
 *
 * A duplicated transform, accepted knowingly: the alternative is an IPC call per
 * keystroke to render a preview. Precedent for saying so out loud rather than
 * hiding it is `src/routes/Settings.tsx:415-416`'s explicit sync comment.
 * Keep in sync with `crates/hyprlayer-core/src/worktree/manager.rs:823`.
 *
 * The 48 cap is on `kebab.len()` (BYTES) in Rust; only ASCII alphanumerics and
 * `-` are ever pushed, so `.length` here counts the same units.
 */
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

/**
 * The type a title suggests, or null. Earliest matching WORD in the title wins,
 * ties within one word broken by list order — so "Fix the hotfix script"
 * suggests Bug fix, not Hotfix, and the answer never depends on how many
 * keywords an entry happens to carry.
 *
 * A keyword matches a whole title word, not a substring: "prefix" must not
 * suggest Bug fix. Because the title is split on non-alphanumerics, a stored
 * keyword that is not itself a single alphanumeric word can never equal one of
 * those words and so never matches — which is exactly what makes `-` the
 * documented "never suggest this type" opt-out (see `BranchPrefix::match_words`
 * in `crates/hyprlayer-core/src/branch_prefix.rs`). The editor's placeholder
 * says so.
 */
export function suggestPrefix(title: string, list: BranchPrefix[]): BranchPrefix | null {
  // Unicode-aware to match `match_words`'s `char::is_alphanumeric`, NOT
  // `previewSlug`'s ASCII rule above — the two Rust functions they mirror use
  // different alphabets on purpose. Splitting ASCII-only tore "réparation"
  // into "r" and "paration", so a label or keyword carrying any non-ASCII
  // letter could never match the very title that spelled it.
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

/**
 * Why a prefix is invalid, or null when it is fine. A MIRROR of
 * `validate_prefix` (`crates/hyprlayer-core/src/branch_prefix.rs`), kept in the
 * same rule order so both sides report the same first failure.
 *
 * The backend stays the authority — it is what a direct IPC call or a
 * hand-edited `settings.toml` still has to pass. This copy exists so the editor
 * can mark the offending FIELD as you type, instead of round-tripping a save to
 * put a banner at the top of the card, several hundred pixels from the row it
 * is about.
 *
 * Duplicated knowingly, the same trade `previewSlug` above makes against
 * `unique_slug`. The anti-drift device is the shared case list: the Rust tests
 * `validate_rejects_git_hostile_prefixes` and
 * `validate_accepts_the_shapes_users_want` enumerate the same strings this
 * module's test does, so a rule added on one side without the other fails a test.
 *
 * Empty is NOT an error here either — it means "unset". A list entry may not be
 * blank, but that is `validatePrefixList`'s rule, not this one's.
 */
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
  return "a path segment must not end in '.lock'"; // lockSegment, the only group left
}

/**
 * One alternation, mirroring `INVALID_PREFIX` in
 * `crates/hyprlayer-core/src/branch_prefix.rs` group-for-group — same names,
 * same order, same rationale in the Rust doc comment for `leadDash`,
 * `dotSegment`, and `lockSegment`, the three rules `check-ref-format` itself
 * doesn't enforce. `RegExp#exec` returns the leftmost match, and JS regex
 * alternation has no verbose/`(?x)` mode to carry the per-branch comments the
 * Rust version does inline, so read them there instead of duplicating them
 * here to drift out of sync.
 *
 * The whitespace/control branch is the one place this can't just transcribe
 * the Rust class: `\x00-\x20` triggers `no-control-regex`, and unlike the
 * ASCII-only ranges below, it can't be rewritten as a few literal characters.
 */
// eslint-disable-next-line no-control-regex
const INVALID_PREFIX = /(?<leadSlash>^\/)|(?<leadDash>^-)|(?<doubleSlash>\/\/)|(?<dotDot>\.\.)|(?<reflog>@\{)|(?<wsCtl>[\x00-\x20\x7f])|(?<forbidden>[~^:?*[\\])|(?<dotSegment>(?:^|\/)\.)|(?<lockSegment>\.lock\/)/;

/** Per-row problems for a whole list, keyed by row index. Mirrors the list-level
 *  half of `validate_prefixes`: blank label, blank prefix, duplicate prefix.
 *  Values are display-ready sentences, so the editor renders them verbatim.
 *  `skipBlank` holds the indexes of rows the user has not filled in yet (a
 *  freshly added one), so the editor does not shout red at an untouched row. */
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
