import { describe, expect, it } from "vitest";

import type { BranchPrefix } from "./branchPrefix";
import { previewSlug, suggestPrefix, validateBranchPrefix, validatePrefixList } from "./branchPrefix";

/** The built-in set as `get_branch_prefixes` hands it over, keywords already
 *  filled (`BUILTIN_PREFIXES`, `crates/hyprlayer-core/src/branch_prefix.rs`).
 *  List order matters: it is what breaks a tie inside one title word. */
const BUILTINS: BranchPrefix[] = [
  { label: "Feature", prefix: "feat/", keywords: ["feature", "add", "new", "implement"] },
  { label: "Bug fix", prefix: "fix/", keywords: ["fix", "bug", "crash", "broken"] },
  { label: "Hotfix", prefix: "hotfix/", keywords: ["hotfix", "urgent", "revert"] },
];

describe("previewSlug", () => {
  // Same fixtures as `slugs_are_kebab_and_unique`
  // (`crates/hyprlayer-core/src/worktree/manager.rs:977-993`), minus the suffix
  // the backend appends — that is what keeps the accepted duplication honest.
  it("kebabs a title the way unique_slug does", () => {
    expect(previewSlug("Add OAuth2 login!")).toBe("add-oauth2-login");
  });

  it("falls back to work-item when nothing survives", () => {
    expect(previewSlug("!!!")).toBe("work-item");
    // The empty title needs no branch at the call site for the same reason.
    expect(previewSlug("")).toBe("work-item");
  });

  it("caps a long title at 48 with no trailing dash", () => {
    // Fixture string borrowed from `slug_is_capped_and_sanitized`
    // (`crates/hyprlayer-core/src/docs/templates.rs:164-182`); that test guards
    // a different slugger (cap 62, "untitled" fallback), so this is a fixture
    // source, not a cross-check.
    const slug = previewSlug("Wow!! " + "very long words ".repeat(20));
    expect(slug.length).toBeLessThanOrEqual(48);
    expect(slug.startsWith("wow-very-long-words")).toBe(true);
    expect(slug.endsWith("-")).toBe(false);
    expect(/^[a-z0-9-]+$/.test(slug)).toBe(true);
  });
});

describe("suggestPrefix", () => {
  it("picks the type a title's keywords name", () => {
    expect(suggestPrefix("Fix the login crash", BUILTINS)?.label).toBe("Bug fix");
    expect(suggestPrefix("Urgent revert of the deploy", BUILTINS)?.label).toBe("Hotfix");
    expect(suggestPrefix("Dashboard tweaks", BUILTINS)).toBeNull();
  });

  it("lets the earliest matching word win, not the longest keyword list", () => {
    // "fix" is word 1, "hotfix" is word 3: position in the title decides.
    expect(suggestPrefix("Fix the hotfix script", BUILTINS)?.label).toBe("Bug fix");
  });

  it("matches whole words, never substrings", () => {
    // "prefix" contains "fix" and must not suggest Bug fix; here "add" is what
    // actually hits, one word earlier.
    expect(suggestPrefix("Add a prefix picker", BUILTINS)?.label).not.toBe("Bug fix");
    // With nothing else to match, the substring alone suggests nothing at all.
    expect(suggestPrefix("The prefix picker", BUILTINS)).toBeNull();
  });

  it("tokenizes the title the way match_words tokenizes a label", () => {
    // `match_words` splits a label on `char::is_alphanumeric`, so the label
    // "Réparation" yields the keyword "réparation". An ASCII-only split here
    // produced "r" and "paration" and never matched it.
    const accented: BranchPrefix[] = [{ label: "Réparation", prefix: "fix/", keywords: [] }];
    const filled: BranchPrefix[] = [
      { label: "Réparation", prefix: "fix/", keywords: ["réparation"] },
    ];
    expect(suggestPrefix("Réparation du login", filled)?.prefix).toBe("fix/");
    // And the fallback path the backend fills from the same label.
    expect(suggestPrefix("réparation urgente", filled)?.prefix).toBe("fix/");
    expect(accented[0].label).toBe("Réparation");
  });

  it("never suggests an entry whose only keyword is the '-' opt-out", () => {
    const optedOut: BranchPrefix[] = [{ label: "Never", prefix: "never/", keywords: ["-"] }];
    for (const title of ["Fix the login crash", "never", "-", "a - b", ""]) {
      expect(suggestPrefix(title, optedOut)).toBeNull();
    }
  });
});

/**
 * The anti-drift device for the TS mirror of `validate_prefix`. These two arrays
 * are copied verbatim from the Rust tests `validate_rejects_git_hostile_prefixes`
 * and `validate_accepts_the_shapes_users_want`
 * (`crates/hyprlayer-core/src/branch_prefix.rs`). Add a rule on one side without
 * the other and one of these fails.
 */
describe("validateBranchPrefix", () => {
  const REJECTED = [
    "/fix/",
    "a//b/",
    "my feature/",
    "a..b/",
    "fix~/",
    "fix^/",
    "fix:/",
    "fix?/",
    "fix*/",
    "fix[/",
    "fix\\/",
    "fix/\u007f/",
    "./fix/",
    "fix/.hidden/",
    "fix.lock/",
    "fix/x.lock/",
    "-fix/",
    "-/",
    "fix@{/",
    "@{/",
    "a/b@{c/",
  ];

  const ACCEPTED = [
    "fix/",
    "hotfix-",
    "jt/fix/",
    "release_1/",
    "fix/-x/",
    "my-fix/",
    "fix@x/",
    "fix@/",
    "fix/x.lock",
    "topic.lock",
  ];

  it.each(REJECTED)("rejects %j exactly as the Rust side does", (prefix) => {
    expect(validateBranchPrefix(prefix)).not.toBeNull();
  });

  it.each(ACCEPTED)("accepts %j exactly as the Rust side does", (prefix) => {
    expect(validateBranchPrefix(prefix)).toBeNull();
  });

  it("treats blank as unset, not invalid", () => {
    // Same asymmetry as `validate_prefix`: blank is "unset" on its own, and only
    // `validatePrefixList` refuses it as a list entry.
    expect(validateBranchPrefix("")).toBeNull();
    expect(validateBranchPrefix("   ")).toBeNull();
  });
});

describe("validatePrefixList", () => {
  it("flags a blanked saved row but stays quiet on an untouched new one", () => {
    const rows = [
      { label: "Bug fix", prefix: "" },
      { label: "", prefix: "" },
    ];
    // Row 1 is the freshly-added one, exempt until the user types in it.
    const errors = validatePrefixList(rows, new Set([1]));
    expect(errors.get(0)?.prefix).toBe("This branch type needs a prefix.");
    expect(errors.has(1)).toBe(false);
  });

  it("flags the second of two rows sharing a prefix, after trimming", () => {
    const errors = validatePrefixList([
      { label: "Bug fix", prefix: "fix/" },
      { label: "Fix", prefix: " fix/ " },
    ]);
    expect(errors.has(0)).toBe(false);
    expect(errors.get(1)?.prefix).toMatch(/already uses/);
  });

  it("reports the character rule rather than the duplicate rule", () => {
    const errors = validatePrefixList([{ label: "Bad", prefix: "-fix/" }]);
    expect(errors.get(0)?.prefix).toBe("Prefix must not start with '-'.");
  });
});
