import type { BadgeTone } from "@/components/ui";

export const DOC_PAGE_SIZE = 50;

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
