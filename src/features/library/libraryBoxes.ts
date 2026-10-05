import type { IconName } from "@/components/Icon";
import type { DocCounts, ThoughtQuery } from "@/lib/bindings";
import { asDocType, DOC_TYPE_META, type DocStatus, type DocType } from "@/lib/domain/docs";

export type LibraryBox = "all" | "new" | "drafts" | DocType | "implemented" | "archived";

type BoxMeta = { label: string; icon: IconName; group: "smart" | "types" | "status" };

const typeBox = (t: DocType): BoxMeta => ({
  label: DOC_TYPE_META[t].plural,
  icon: DOC_TYPE_META[t].icon,
  group: "types",
});

// Key order is nav order.
export const LIBRARY_BOXES: Record<LibraryBox, BoxMeta> = {
  all: { label: "All thoughts", icon: "inbox", group: "smart" },
  new: { label: "New", icon: "unread", group: "smart" },
  drafts: { label: "Drafts", icon: "draft", group: "smart" },
  plan: typeBox("plan"),
  research: typeBox("research"),
  handoff: typeBox("handoff"),
  note: typeBox("note"),
  pr: typeBox("pr"),
  implemented: { label: "Implemented", icon: "validate", group: "status" },
  archived: { label: "Archived", icon: "archive", group: "status" },
};

export const LIBRARY_BOX_KEYS = Object.keys(LIBRARY_BOXES) as LibraryBox[];

const BOX_STATUS: Partial<Record<LibraryBox, DocStatus>> = {
  drafts: "draft",
  implemented: "implemented",
  archived: "archived",
};

export function asLibraryBox(s: string | undefined): LibraryBox {
  return LIBRARY_BOX_KEYS.find((k) => k === s) ?? "all";
}

export function boxQuery(box: LibraryBox): Pick<ThoughtQuery, "types" | "statuses" | "unreadOnly"> {
  if (box === "new") return { unreadOnly: true };
  const status = BOX_STATUS[box];
  if (status) return { statuses: [status] };
  const type = asDocType(box);
  return type ? { types: [type] } : {};
}

export function boxCount(box: LibraryBox, counts: DocCounts | undefined): number | null {
  if (!counts) return null;
  if (box === "all") return counts.total;
  if (box === "new") return counts.unread;
  const status = BOX_STATUS[box];
  if (status) return counts.statuses.find((s) => s.status === status)?.count ?? 0;
  return counts.types.find((t) => t.docType === box)?.count ?? 0;
}
