import type { IconName } from "@/components/Icon";

const TYPE_ICON: Record<string, IconName> = {
  plan: "tasks",
  research: "research",
  handoff: "handoff",
  note: "note",
  pr: "pullRequest",
  external: "external",
};

export function docTypeIcon(type: string | null): IconName {
  return (type && TYPE_ICON[type]) || "docText";
}
