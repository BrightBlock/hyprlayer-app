import { cx } from "@/components/ui";

import type { ArchiveTone } from "./archiveStatus";

// The muted fill is the hovered row's own colour, so it steps darker there.
const MUTED = "bg-panel-2 text-ink-dim group-hover:bg-edge group-focus-within:bg-edge";

const TAG: Record<ArchiveTone, string> = {
  merged: "bg-ok text-accent-ink",
  abandoned: MUTED,
  superseded: "bg-info text-accent-ink",
  purge: "bg-tag-purge-bg text-warn-ink",
  neutral: MUTED,
};

export const STATUS_DOT: Record<ArchiveTone, string> = {
  merged: "bg-ok",
  abandoned: "bg-ink-4",
  superseded: "bg-info",
  purge: "bg-warn",
  neutral: "bg-ink-4",
};

export function StatusTag({ tone, label }: { tone: ArchiveTone; label: string }) {
  return (
    <span
      className={cx(
        "shrink-0 whitespace-nowrap rounded-card px-[7px] py-0.5 font-mono text-[9.5px] font-semibold leading-none",
        TAG[tone],
      )}
    >
      {label}
    </span>
  );
}
