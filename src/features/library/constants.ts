import type { BadgeTone } from "@/components/ui";

/** Page size for a single library query, the list shows the first N matches and
 *  prompts to narrow the filters past it. */
export const DOC_PAGE_SIZE = 50;

/** Canonical doc types/statuses/scopes, the frontmatter tolerates anything else. */
export const DOC_TYPES = ["plan", "research", "handoff", "note", "pr"] as const;
export const DOC_STATUSES = ["draft", "active", "implemented", "superseded", "archived"] as const;
export const DOC_SCOPES = ["user", "shared", "global"] as const;

export const STATUS_TONE: Record<string, BadgeTone> = {
  draft: "dim",
  active: "ok",
  implemented: "accent",
  superseded: "warn",
  archived: "dim",
};
