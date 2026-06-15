import type { BadgeTone } from "../../components/ui";

/** Canonical doc types/statuses/scopes — the frontmatter tolerates anything else. */
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
