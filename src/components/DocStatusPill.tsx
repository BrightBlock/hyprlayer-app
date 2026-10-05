import { DOC_STATUS_LABEL, asDocStatus, type DocStatus } from "@/lib/domain/docs";
import { cx } from "./ui";

const STATUS_CLASS = {
  draft: "bg-tag-draft-bg text-warn-ink",
  active: "bg-ok/10 text-diff-add",
  implemented: "bg-accent-soft text-ink-2",
  superseded: "bg-panel-2 text-ink-4",
  archived: "bg-panel-2 text-ink-4",
} as const satisfies Record<DocStatus, string>;

export function DocStatusPill({ status }: { status: string }) {
  const known = asDocStatus(status);
  return (
    <span
      className={cx(
        "inline-flex items-center rounded-control px-1.5 py-0.5 text-ui-sm font-semibold",
        STATUS_CLASS[known ?? "superseded"],
      )}
    >
      {known ? DOC_STATUS_LABEL[known] : status}
    </span>
  );
}
