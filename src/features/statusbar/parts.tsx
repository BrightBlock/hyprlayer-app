import { useEffect, useState } from "react";

import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import { formatCompactCount } from "@/lib/format";
import type { RepoSummary } from "./statusBarState";

export const BAR_BUTTON = cx(
  "flex h-6 shrink-0 items-center gap-1.5 rounded-control px-1.5 transition-colors",
  "focus:outline-none enabled:hover:bg-panel-2 enabled:focus-visible:bg-panel-2",
  "disabled:cursor-not-allowed disabled:opacity-50",
);

export function Divider() {
  return <span aria-hidden="true" className="h-4 w-px shrink-0 bg-edge" />;
}

export function Dot({ className }: { className: string }) {
  return <span aria-hidden="true" className={cx("h-1.5 w-1.5 shrink-0 rounded-full", className)} />;
}

/** Menu open state that closes once its trigger is disabled, so it cannot reappear later. */
export function useMenuOpen(enabled: boolean) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!enabled && open) setOpen(false);
  }, [enabled, open]);
  return [open && enabled, setOpen] as const;
}

export function RepoStatus({ repo, compact }: { repo: RepoSummary; compact: boolean }) {
  return (
    <div className="flex min-w-0 items-center gap-3">
      <span className="flex min-w-0 items-center gap-1.5" title={repo.branch}>
        <Icon name="branch" size="xs" className="text-ink-dim" />
        <span className="truncate text-ink-2">{repo.branch}</span>
      </span>
      {!compact && (
        <span title="Uncommitted changes vs HEAD" className="flex shrink-0 items-center gap-1.5">
          <span className="text-diff-add">+{formatCompactCount(repo.additions)}</span>
          <span className="text-diff-del">
            {"−"}
            {formatCompactCount(repo.deletions)}
          </span>
        </span>
      )}
    </div>
  );
}
