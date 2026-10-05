import type { IconName } from "@/components/Icon";
import { DOC_TYPE_META, asDocType } from "@/lib/domain/docs";

export function docTypeIcon(type: string | null): IconName {
  if (type === "external") return "external";
  const known = asDocType(type);
  return known ? DOC_TYPE_META[known].icon : "docText";
}
