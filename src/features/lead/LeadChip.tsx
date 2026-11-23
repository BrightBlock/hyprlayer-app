import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";

export function LeadMark({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cx(
        "inline-flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border border-ink text-ink",
        className,
      )}
    >
      <Icon name="lead" size="xs" />
    </span>
  );
}

export function LeadChip({ driving, of }: { driving: number; of: number }) {
  return (
    <span className="inline-flex h-6 shrink-0 items-center gap-1.5 rounded-control border border-line-strong px-2 text-ink-2">
      <LeadMark className="h-4 w-4" />
      <span className="font-mono text-meta">{driving > 0 ? `Lead · driving ${driving} of ${of}` : "Lead"}</span>
    </span>
  );
}
