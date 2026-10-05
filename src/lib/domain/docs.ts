import type { IconName } from "@/components/Icon";
import type { DocScope, DocStatus, DocType } from "@/lib/bindings";

export type { DocScope, DocStatus, DocType } from "@/lib/bindings";

export const DOC_TYPE_META = {
  plan: { label: "Plan", plural: "Plans", icon: "tasks" },
  research: { label: "Research", plural: "Research", icon: "research" },
  handoff: { label: "Handoff", plural: "Handoffs", icon: "handoff" },
  note: { label: "Note", plural: "Notes", icon: "note" },
  pr: { label: "PR", plural: "PRs", icon: "pullRequest" },
} as const satisfies Record<DocType, { label: string; plural: string; icon: IconName }>;

export const DOC_STATUS_LABEL = {
  draft: "Draft",
  active: "Active",
  implemented: "Implemented",
  superseded: "Superseded",
  archived: "Archived",
} as const satisfies Record<DocStatus, string>;

export const DOC_SCOPE_LABEL = {
  user: "User",
  shared: "Shared",
  global: "Global",
} as const satisfies Record<DocScope, string>;

export const DOC_TYPES = Object.keys(DOC_TYPE_META) as DocType[];
export const DOC_STATUSES = Object.keys(DOC_STATUS_LABEL) as DocStatus[];
export const DOC_SCOPES = Object.keys(DOC_SCOPE_LABEL) as DocScope[];

function asMember<T extends string>(all: readonly T[], s: string | null | undefined): T | null {
  const v = s?.toLowerCase();
  return all.find((t) => t === v) ?? null;
}

export function asDocType(s: string | null | undefined): DocType | null {
  return asMember(DOC_TYPES, s);
}

export function asDocStatus(s: string | null | undefined): DocStatus | null {
  return asMember(DOC_STATUSES, s);
}
